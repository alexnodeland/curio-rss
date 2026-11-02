/**
 * Reading aids (#153): the reading-time estimate, the scroll ↔ position
 * mapping, the resume pin, and the reading session's dwell + position saves.
 */
import {
    DWELL_MIN_MS,
    HIDDEN_END_MS,
    IDLE_CAP_MS,
    PROGRESS_SAVE_MS,
    readingSession,
} from '$lib/state/reading-session';
import {
    PERMILLE_END,
    RESUME_HOLD_MS,
    formatPercent,
    permilleAt,
    readMinutes,
    resumable,
    resumeScroll,
    scrollTopFor,
} from '$lib/utils/reading';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';

describe('readMinutes', () => {
    it('rounds to whole minutes at 230 wpm, never below one', () => {
        expect(readMinutes(1)).toBe(1);
        expect(readMinutes(230)).toBe(1);
        expect(readMinutes(460)).toBe(2);
        expect(readMinutes(1_840)).toBe(8);
    });

    it('has no estimate without a usable count', () => {
        expect(readMinutes(null)).toBeNull();
        expect(readMinutes(0)).toBeNull();
        expect(readMinutes(-3)).toBeNull();
        expect(readMinutes(Number.NaN)).toBeNull();
    });
});

describe('formatPercent', () => {
    it("formats thousandths in the locale's own percent style", () => {
        expect(formatPercent(420, 'en')).toBe('42%');
        expect(formatPercent(420, 'de')).toBe('42\u00a0%');
        expect(formatPercent(420, 'en')).toBe('42%'); // cached, same result
    });
});

describe('scroll position ↔ permille', () => {
    it('reads the share of the scroll extent, snapping the edges', () => {
        const at = (scrollTop: number) =>
            permilleAt({ scrollTop, scrollHeight: 5_000, clientHeight: 1_000 });
        expect(at(0)).toBe(0);
        expect(at(3)).toBe(0); // sub-pixel slack at the top
        expect(at(1_680)).toBe(420);
        expect(at(3_998)).toBe(PERMILLE_END); // the last few pixels are the end
        expect(at(4_000)).toBe(PERMILLE_END);
    });

    it('has no position for a body that does not scroll', () => {
        expect(permilleAt({ scrollTop: 0, scrollHeight: 900, clientHeight: 1_000 })).toBeNull();
        expect(permilleAt({ scrollTop: 0, scrollHeight: 1_002, clientHeight: 1_000 })).toBeNull();
    });

    it('maps a position back to a scrollTop, clamped', () => {
        expect(scrollTopFor(420, 5_000, 1_000)).toBe(1_680);
        expect(scrollTopFor(-5, 5_000, 1_000)).toBe(0);
        expect(scrollTopFor(4_000, 5_000, 1_000)).toBe(4_000);
        expect(scrollTopFor(500, 800, 1_000)).toBe(0);
    });

    it('resumes only a started, unfinished article', () => {
        expect(resumable(0)).toBe(false);
        expect(resumable(1)).toBe(true);
        expect(resumable(999)).toBe(true);
        expect(resumable(PERMILLE_END)).toBe(false);
    });
});

describe('resumeScroll', () => {
    afterEach(() => {
        vi.useRealTimers();
    });

    /** A scroll body whose content height the test controls. */
    function body(height: { value: number }): HTMLElement {
        const element = document.createElement('div');
        element.append(document.createElement('article'));
        Object.defineProperty(element, 'scrollHeight', { get: () => height.value });
        Object.defineProperty(element, 'clientHeight', { value: 1_000 });
        element.style.scrollBehavior = 'smooth';
        return element;
    }

    /** Captures the ResizeObserver callback the pin registers. */
    function captureResize(): { fire: () => void; disconnected: () => boolean } {
        let callback: (() => void) | null = null;
        let disconnected = false;
        vi.stubGlobal(
            'ResizeObserver',
            class {
                constructor(cb: () => void) {
                    callback = cb;
                }
                observe(): void {}
                disconnect(): void {
                    disconnected = true;
                }
            },
        );
        return { fire: () => callback?.(), disconnected: () => disconnected };
    }

    it('pins the fraction while the content grows, then lets go', () => {
        vi.useFakeTimers();
        const resize = captureResize();
        const height = { value: 5_000 };
        const element = body(height);

        resumeScroll(element, 500);
        expect(element.scrollTop).toBe(2_000);
        expect(element.style.scrollBehavior).toBe('smooth'); // restored after the jump

        height.value = 9_000; // images arrived
        resize.fire();
        expect(element.scrollTop).toBe(4_000);

        vi.advanceTimersByTime(RESUME_HOLD_MS);
        expect(resize.disconnected()).toBe(true);
        vi.unstubAllGlobals();
    });

    it('lets go as soon as the reader scrolls themselves', () => {
        const resize = captureResize();
        const element = body({ value: 5_000 });
        resumeScroll(element, 500);
        element.dispatchEvent(new Event('wheel'));
        expect(resize.disconnected()).toBe(true);
        vi.unstubAllGlobals();
    });

    it('lets go when something else scrolls the body (a page step from a global shortcut)', () => {
        const resize = captureResize();
        const element = body({ value: 5_000 });
        resumeScroll(element, 500);

        // The pin's own scroll lands where the pin put it: still held.
        element.dispatchEvent(new Event('scroll'));
        expect(resize.disconnected()).toBe(false);

        // Space with focus in the list pages the body directly.
        element.scrollTop = 2_900;
        element.dispatchEvent(new Event('scroll'));
        // Released: a late image no longer snaps the page step back.
        expect(resize.disconnected()).toBe(true);
        vi.unstubAllGlobals();
    });

    it('returns a cancel', () => {
        const resize = captureResize();
        const cancel = resumeScroll(body({ value: 5_000 }), 300);
        cancel();
        expect(resize.disconnected()).toBe(true);
        vi.unstubAllGlobals();
    });
});

describe('readingSession', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        readingSession.end();
        vi.useRealTimers();
        harness?.teardown();
        harness = null;
    });

    function install(): IpcHarness {
        return installIpcHarness({ record_opened: null, set_read_progress: true });
    }

    it('records the foreground dwell when the session ends', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.begin(7, true);
        vi.advanceTimersByTime(2_500);
        readingSession.setActive(false);
        vi.advanceTimersByTime(30_000);
        readingSession.setActive(true);
        readingSession.setActive(true); // a repeat focus changes nothing
        vi.advanceTimersByTime(500);
        expect(readingSession.dwellMs()).toBe(3_000);

        readingSession.end();
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([{ articleId: 7, dwellMs: 3_000 }]);
        expect(readingSession.articleId).toBeNull();
    });

    it('a session begun in the background starts counting on focus', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.begin(7, false);
        vi.advanceTimersByTime(10_000);
        readingSession.setActive(true);
        vi.advanceTimersByTime(DWELL_MIN_MS);
        readingSession.end();
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([
            { articleId: 7, dwellMs: DWELL_MIN_MS },
        ]);
    });

    it('records nothing for a pass shorter than a read, and ignores calls without a session', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.setActive(true);
        readingSession.end();
        readingSession.begin(7, true);
        readingSession.begin(7, true); // the running article: a no-op
        vi.advanceTimersByTime(DWELL_MIN_MS - 1);
        readingSession.begin(8, true); // moving on ends 7's session
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([]);
        expect(readingSession.articleId).toBe(8);
    });

    it('an external open of the session article records at once and restarts the count', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.begin(7, true);
        vi.advanceTimersByTime(900);
        expect(readingSession.recordExternalOpen(8)).toBe(false);
        expect(readingSession.recordExternalOpen(7)).toBe(true);
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([{ articleId: 7, dwellMs: 900 }]);
        expect(readingSession.dwellMs()).toBe(0);
    });

    it('a short trip to the background pauses; a long one ends the session', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance', 'setTimeout', 'clearTimeout'] });
        harness = install();
        readingSession.begin(7, true);
        readingSession.trackProgress(7, 300);
        vi.advanceTimersByTime(2_000);
        readingSession.hide();
        await vi.advanceTimersByTimeAsync(HIDDEN_END_MS - 1);
        // Paused, with the position saved as it went.
        expect(readingSession.articleId).toBe(7);
        expect(readingSession.dwellMs()).toBe(2_000);
        expect(harness.callsFor('set_read_progress')).toEqual([
            { articleId: 7, readPermille: 300 },
        ]);
        expect(harness.callsFor('record_opened')).toEqual([]);

        readingSession.begin(7, true); // back: the same session carries on
        vi.advanceTimersByTime(1_000);
        readingSession.touch();
        // The pending end was called off (counting stops at the idle cap).
        await vi.advanceTimersByTimeAsync(HIDDEN_END_MS);
        expect(readingSession.articleId).toBe(7);
        expect(readingSession.dwellMs()).toBe(3_000 + IDLE_CAP_MS);
        expect(harness.callsFor('record_opened')).toEqual([]);

        readingSession.hide();
        readingSession.hide(); // a repeat restarts the wait, records nothing
        await vi.advanceTimersByTimeAsync(HIDDEN_END_MS);
        expect(readingSession.articleId).toBeNull();
        expect(harness.callsFor('record_opened')).toEqual([
            { articleId: 7, dwellMs: 3_000 + IDLE_CAP_MS },
        ]);
        readingSession.hide(); // no session: nothing to do
    });

    it('caps a quiet foreground stretch at the idle limit', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.touch(); // no session: ignored
        readingSession.begin(7, true);
        vi.advanceTimersByTime(30_000);
        readingSession.touch(); // still there
        vi.advanceTimersByTime(8 * 60 * 60_000); // overnight, window in front
        expect(readingSession.dwellMs()).toBe(30_000 + IDLE_CAP_MS);

        readingSession.touch(); // back: counting resumes from now
        vi.advanceTimersByTime(4_000);
        expect(readingSession.dwellMs()).toBe(30_000 + IDLE_CAP_MS + 4_000);

        readingSession.setActive(false);
        readingSession.touch(); // in the background: ignored
        readingSession.end();
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([
            { articleId: 7, dwellMs: 34_000 + IDLE_CAP_MS },
        ]);
    });

    it('a wall-clock step backwards never makes the dwell negative', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = install();
        readingSession.begin(7, true);
        vi.advanceTimersByTime(2_000);
        vi.setSystemTime(new Date(Date.now() - 60 * 60_000)); // NTP steps back an hour
        vi.advanceTimersByTime(1_000);
        readingSession.end();
        await flushIpc();
        expect(harness.callsFor('record_opened')).toEqual([{ articleId: 7, dwellMs: 3_000 }]);
    });

    it('a write that throws (no Tauri runtime) is dropped, not an unhandled rejection', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        harness = installIpcHarness({}); // no responders: every invoke throws
        readingSession.begin(7, true);
        readingSession.trackProgress(7, 400);
        vi.advanceTimersByTime(DWELL_MIN_MS);
        readingSession.end();
        await flushIpc();
        expect(harness.callsFor('record_opened')).toHaveLength(1);
        expect(harness.callsFor('set_read_progress')).toHaveLength(1);
    });

    it('saves the position once the scroll settles, never an unmoved one', async () => {
        vi.useFakeTimers();
        harness = install();
        readingSession.begin(7, true);
        readingSession.trackProgress(8, 300); // not the session's article
        readingSession.trackProgress(7, 200);
        readingSession.trackProgress(7, 250);
        vi.advanceTimersByTime(PROGRESS_SAVE_MS);
        await vi.runAllTimersAsync();
        expect(harness.callsFor('set_read_progress')).toEqual([
            { articleId: 7, readPermille: 250 },
        ]);

        readingSession.trackProgress(7, 250);
        vi.advanceTimersByTime(PROGRESS_SAVE_MS);
        readingSession.trackProgress(7, 600);
        readingSession.end(); // the pending save goes out with the session
        await vi.runAllTimersAsync();
        expect(harness.callsFor('set_read_progress')).toEqual([
            { articleId: 7, readPermille: 250 },
            { articleId: 7, readPermille: 600 },
        ]);
    });
});
