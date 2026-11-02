/**
 * The reader's session bookkeeping (#153): real dwell time for the
 * `article.opened` contract event, and the saved reading position.
 *
 * A *session* is one stretch of an article sitting in the reader pane. Its
 * dwell counts only the time the window is in the foreground (visible and
 * focused) *and* the reader is there: a foreground stretch with no input for
 * {@link IDLE_CAP_MS} stops counting at that mark, so an article left open
 * overnight, behind another app, or in front of a Mac that went to sleep
 * does not read as hours of reading. Time comes from the monotonic clock
 * (`performance.now()`), so a wall-clock step (an NTP correction, a manual
 * change) can never make a dwell negative.
 *
 * The session ends when the selection moves on, the reader closes, the page
 * goes away, or the window stays in the background for
 * {@link HIDDEN_END_MS}; only then is the dwell known, so that is when
 * `article.opened` is recorded — with a real `dwell_ms`. A brief trip to the
 * background (another app's full-screen window, a Spaces switch) only pauses
 * it, so one read stays one open. A pass shorter than {@link DWELL_MIN_MS}
 * (j/k scanning past an article) was not an open-for-reading and records
 * nothing.
 *
 * The position rides alongside: the reader reports it as it scrolls, and it
 * is saved once the scroll settles ({@link PROGRESS_SAVE_MS}), as the window
 * goes to the background, and as the session ends, so a reopen resumes where
 * reading stopped.
 *
 * Both writes are fire-and-forget: a failure is logged by the Rust command
 * layer, and a toast for a background save the user never asked for would
 * only be noise.
 */
import { commands } from '$lib/bindings';

/** The shortest stay that counts as opening an article for reading. */
export const DWELL_MIN_MS = 1500;

/** How long the scroll must settle before the position is saved. */
export const PROGRESS_SAVE_MS = 800;

/**
 * How long a foreground stretch counts without any input from the reader —
 * long enough for a slow page of text, short enough that walking away (or the
 * Mac sleeping) adds at most this much.
 */
export const IDLE_CAP_MS = 2 * 60_000;

/**
 * How long the window may stay in the background before the session ends
 * (recording its open). A shorter absence only pauses it. The open is
 * recorded while the webview can still deliver it — a quit need not run
 * page-lifecycle handlers — so this is short of "for good".
 */
export const HIDDEN_END_MS = 5 * 60_000;

/** The monotonic clock dwell is measured on. */
function now(): number {
    return performance.now();
}

/**
 * Sends a background write without waiting on it. A contract error comes
 * back as a value (already logged Rust-side); a thrown one — no Tauri
 * runtime — is dropped here rather than surfacing as an unhandled rejection.
 */
function fireAndForget(write: Promise<unknown>): void {
    write.catch(() => undefined);
}

class ReadingSession {
    #articleId: number | null = null;
    /** Foreground time banked from finished stretches. */
    #dwellMs = 0;
    /** When the running foreground stretch began (`now()`), if one is. */
    #since: number | null = null;
    /** The reader's latest input within the running stretch (`now()`). */
    #lastInput = 0;
    /** The newest position not yet saved. */
    #pending: number | null = null;
    /** The position this session last saved (an unmoved one is not re-sent). */
    #saved: number | null = null;
    #saveTimer: ReturnType<typeof setTimeout> | null = null;
    /** Ends the session once the window has stayed in the background. */
    #hiddenTimer: ReturnType<typeof setTimeout> | null = null;

    /** The article this session is on, if any. */
    get articleId(): number | null {
        return this.#articleId;
    }

    /**
     * Starts a session on `articleId`, ending any other one first. A repeat
     * call for the running article keeps that session — back from the
     * background, it carries on — and only updates whether it counts.
     * `active` says whether the window is in the foreground right now.
     */
    begin(articleId: number, active: boolean): void {
        if (this.#articleId === articleId) {
            this.#clearHiddenTimer();
            this.setActive(active);
            return;
        }
        this.end();
        this.#articleId = articleId;
        this.#dwellMs = 0;
        this.#since = null;
        this.#saved = null;
        this.setActive(active);
    }

    /** The window came to (`true`) or left (`false`) the foreground. */
    setActive(active: boolean): void {
        if (this.#articleId === null) {
            return;
        }
        if (active && this.#since === null) {
            this.#startStretch();
        } else if (!active && this.#since !== null) {
            this.#dwellMs += this.#running();
            this.#since = null;
        }
    }

    /**
     * The reader did something (scrolled, pressed a key, moved the pointer):
     * they are still there. After an idle gap past {@link IDLE_CAP_MS} the
     * capped stretch is banked and counting starts again from now.
     */
    touch(): void {
        if (this.#since === null) {
            return;
        }
        const at = now();
        if (at - this.#lastInput > IDLE_CAP_MS) {
            this.#dwellMs += this.#running();
            this.#since = at;
        }
        this.#lastInput = at;
    }

    /**
     * The window went to the background: dwell pauses, the position is saved
     * now, and the session ends if the window stays away for
     * {@link HIDDEN_END_MS}. A {@link begin} on the same article before then
     * carries the session on.
     */
    hide(): void {
        if (this.#articleId === null) {
            return;
        }
        this.setActive(false);
        this.#flushProgress();
        this.#clearHiddenTimer();
        this.#hiddenTimer = setTimeout(() => this.end(), HIDDEN_END_MS);
    }

    /** Foreground milliseconds so far in this session. */
    dwellMs(): number {
        return Math.round(this.#dwellMs + this.#running());
    }

    /**
     * Ends the session: saves the position, then records `article.opened`
     * with the dwell — if the article stayed long enough to count.
     */
    end(): void {
        const articleId = this.#articleId;
        if (articleId === null) {
            return;
        }
        this.#clearHiddenTimer();
        this.#flushProgress();
        const dwell = this.dwellMs();
        this.#articleId = null;
        this.#dwellMs = 0;
        this.#since = null;
        this.#saved = null;
        if (dwell >= DWELL_MIN_MS) {
            fireAndForget(commands.recordOpened(articleId, dwell));
        }
    }

    /**
     * The session's article is being opened in the browser: that is an open
     * whatever the dwell, so it is recorded now with the time read so far,
     * and the count restarts for any reading that follows. `false` when
     * `articleId` is not the session's — the caller records that open itself.
     */
    recordExternalOpen(articleId: number): boolean {
        if (this.#articleId !== articleId) {
            return false;
        }
        const dwell = this.dwellMs();
        this.#dwellMs = 0;
        if (this.#since !== null) {
            this.#startStretch();
        }
        fireAndForget(commands.recordOpened(articleId, dwell));
        return true;
    }

    /** The reader scrolled to `permille`; saved once the scroll settles. */
    trackProgress(articleId: number, permille: number): void {
        if (articleId !== this.#articleId) {
            return;
        }
        this.#pending = permille;
        this.#clearTimer();
        this.#saveTimer = setTimeout(() => this.#flushProgress(), PROGRESS_SAVE_MS);
    }

    #flushProgress(): void {
        this.#clearTimer();
        const articleId = this.#articleId;
        const permille = this.#pending;
        this.#pending = null;
        if (articleId === null || permille === null || permille === this.#saved) {
            return;
        }
        this.#saved = permille;
        fireAndForget(commands.setReadProgress(articleId, permille));
    }

    #clearTimer(): void {
        if (this.#saveTimer !== null) {
            clearTimeout(this.#saveTimer);
            this.#saveTimer = null;
        }
    }

    #clearHiddenTimer(): void {
        if (this.#hiddenTimer !== null) {
            clearTimeout(this.#hiddenTimer);
            this.#hiddenTimer = null;
        }
    }

    #startStretch(): void {
        const at = now();
        this.#since = at;
        this.#lastInput = at;
    }

    /**
     * The running stretch's countable time: up to now, or up to
     * {@link IDLE_CAP_MS} past the last input if the reader has gone quiet.
     * Never negative.
     */
    #running(): number {
        if (this.#since === null) {
            return 0;
        }
        const until = Math.min(now(), this.#lastInput + IDLE_CAP_MS);
        return Math.max(0, until - this.#since);
    }
}

/** The one reading session — the reader pane shows one article at a time. */
export const readingSession = new ReadingSession();
