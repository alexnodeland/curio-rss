/**
 * The read-aloud store (#153) over a scripted speech engine: one utterance
 * per sentence in a local voice, the highlight following the sentence being
 * spoken, play/pause/resume/stop, the speed (stored, applied at once), the
 * engine's late callbacks after a cancel being ignored, an engine that lists
 * its voices late, and no read at all without a local voice.
 */
import {
    SPEAKING_ATTR,
    SPEECH_HIGHLIGHT,
    SPEECH_RATE_DEFAULT,
    VOICES_WAIT_MS,
    readAloudStore,
} from '$lib/state/read-aloud.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { type SpokenSentence, collectSentences } from '$lib/utils/speech';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';
import { SYSTEM_VOICE, installHighlights, installSpeech, voice } from './speech-fake';

function sentences(html = '<p>One. Two. Three.</p>'): SpokenSentence[] {
    const root = document.createElement('div');
    root.innerHTML = html;
    document.body.append(root);
    return collectSentences([root], 'en');
}

describe('readAloudStore', () => {
    let harness: IpcHarness | null = null;

    beforeEach(() => {
        harness = installIpcHarness({ set_setting: null });
    });

    afterEach(() => {
        readAloudStore.reset();
        settingsStore.reset();
        toastStore.reset();
        vi.unstubAllGlobals();
        document.body.innerHTML = '';
        harness?.teardown();
        harness = null;
    });

    it('is unsupported, and starts nothing, without a speech engine', () => {
        expect(readAloudStore.supported).toBe(false);
        expect(readAloudStore.start(1, sentences(), { lang: 'en' })).toBe(false);
        expect(readAloudStore.status).toBe('idle');
    });

    it('speaks sentence by sentence in a local voice for the language', () => {
        const engine = installSpeech([
            voice('Cloud', 'en-US', { local: false, isDefault: true }),
            voice('Samantha', 'en-US'),
        ]);
        const followed: string[] = [];
        expect(readAloudStore.supported).toBe(true);
        expect(
            readAloudStore.start(7, sentences(), {
                lang: 'en-US',
                onSentence: (sentence) => followed.push(sentence.text),
            }),
        ).toBe(true);
        expect(readAloudStore.status).toBe('playing');
        expect(readAloudStore.isReading(7)).toBe(true);
        expect(readAloudStore.isReading(8)).toBe(false);
        expect(readAloudStore.total).toBe(3);
        expect(engine.texts()).toEqual(['One.']);
        expect(engine.spoken[0]?.lang).toBe('en-US');
        expect(engine.spoken[0]?.voice?.name).toBe('Samantha');
        expect(engine.spoken[0]?.rate).toBe(SPEECH_RATE_DEFAULT);

        engine.finish();
        expect(readAloudStore.index).toBe(1);
        engine.finish();
        expect(engine.texts()).toEqual(['One.', 'Two.', 'Three.']);
        expect(followed).toEqual(['One.', 'Two.', 'Three.']);

        engine.finish();
        expect(readAloudStore.status).toBe('idle');
        expect(readAloudStore.articleId).toBeNull();
    });

    it('names the voice itself, a local one, even when none speaks the language', () => {
        const engine = installSpeech([
            voice('Cloud fr', 'fr-FR', { local: false }),
            voice('Thomas', 'de-DE'),
        ]);
        readAloudStore.start(1, sentences(), { lang: 'fr' });
        expect(engine.spoken[0]?.voice?.name).toBe('Thomas');
        expect(engine.spoken[0]?.lang).toBe('fr');
        engine.finish();
        expect(engine.spoken[1]?.voice?.name).toBe('Thomas');
    });

    it('without a local voice, reads nothing and says why', () => {
        const engine = installSpeech([voice('Cloud', 'en-US', { local: false, isDefault: true })]);
        expect(readAloudStore.start(1, sentences(), { lang: 'en' })).toBe(true);
        expect(engine.synth.speak).not.toHaveBeenCalled();
        expect(readAloudStore.status).toBe('idle');
        expect(toastStore.toasts.map((toast) => toast.message)).toEqual([
            'No offline voice is installed to read aloud',
        ]);
    });

    describe('an engine that lists its voices late', () => {
        beforeEach(() => {
            vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        });

        afterEach(() => {
            vi.useRealTimers();
        });

        it('waits for the list, then reads in a local voice', () => {
            const engine = installSpeech([]);
            readAloudStore.start(1, sentences(), { lang: 'en' });
            expect(readAloudStore.status).toBe('playing');
            expect(engine.synth.speak).not.toHaveBeenCalled();
            engine.listVoices([SYSTEM_VOICE]);
            expect(engine.texts()).toEqual(['One.']);
            expect(engine.spoken[0]?.voice).toBe(SYSTEM_VOICE);
            expect(engine.listeners()).toBe(0);
            // The wait's timer is gone with it: nothing speaks twice.
            vi.advanceTimersByTime(VOICES_WAIT_MS);
            expect(engine.texts()).toEqual(['One.']);
        });

        it('gives up after a while, without speaking in the engine’s own voice', () => {
            const engine = installSpeech([]);
            readAloudStore.start(1, sentences(), { lang: 'en' });
            vi.advanceTimersByTime(VOICES_WAIT_MS);
            expect(engine.synth.speak).not.toHaveBeenCalled();
            expect(engine.listeners()).toBe(0);
            expect(readAloudStore.status).toBe('idle');
            expect(toastStore.toasts.map((toast) => toast.message)).toEqual([
                'No offline voice is installed to read aloud',
            ]);
        });

        it('a read stopped while it waits stays stopped', () => {
            const engine = installSpeech([]);
            readAloudStore.start(1, sentences(), { lang: 'en' });
            readAloudStore.stop();
            engine.listVoices([SYSTEM_VOICE]);
            vi.advanceTimersByTime(VOICES_WAIT_MS);
            expect(engine.synth.speak).not.toHaveBeenCalled();
            expect(toastStore.toasts).toHaveLength(0);
        });
    });

    it('starts at the sentence asked for, clamped to the article', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en', from: 1 });
        expect(engine.texts()).toEqual(['Two.']);
        expect(readAloudStore.index).toBe(1);
        readAloudStore.start(1, sentences(), { lang: 'en', from: 99 });
        expect(engine.texts()).toEqual(['Two.', 'Three.']);
        readAloudStore.start(1, sentences(), { lang: 'en', from: -4 });
        expect(engine.texts()).toEqual(['Two.', 'Three.', 'One.']);
    });

    it('ends when the article it reads is no longer on the page', () => {
        const engine = installSpeech();
        const spoken = sentences();
        readAloudStore.start(1, spoken, { lang: 'en' });
        // The body re-renders: its text nodes are replaced.
        const paragraph = document.querySelector('p');
        paragraph?.replaceChildren(document.createTextNode('One. Two. Three.'));
        engine.finish();
        expect(engine.texts()).toEqual(['One.']);
        expect(readAloudStore.status).toBe('idle');

        // Or it is taken off the page whole.
        const again = sentences('<p>Four. Five.</p>');
        readAloudStore.start(2, again, { lang: 'en' });
        document.body.replaceChildren();
        engine.finish();
        expect(engine.texts()).toEqual(['One.', 'Four.']);
        expect(readAloudStore.status).toBe('idle');
    });

    it('highlights the sentence being spoken, and clears it on stop', () => {
        const engine = installSpeech();
        const registry = installHighlights();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        expect(registry.get(SPEECH_HIGHLIGHT)?.ranges[0]?.toString()).toBe('One.');
        engine.finish();
        expect(registry.get(SPEECH_HIGHLIGHT)?.ranges[0]?.toString()).toBe('Two.');
        readAloudStore.stop();
        expect(registry.has(SPEECH_HIGHLIGHT)).toBe(false);
    });

    it('without a highlight registry, marks the sentence’s paragraph instead', () => {
        const engine = installSpeech();
        const spoken = sentences('<p>First paragraph.</p><p>Second paragraph.</p>');
        const [first, second] = document.querySelectorAll('p');
        readAloudStore.start(1, spoken, { lang: 'en' });
        expect(first?.hasAttribute(SPEAKING_ATTR)).toBe(true);
        engine.finish();
        expect(first?.hasAttribute(SPEAKING_ATTR)).toBe(false);
        expect(second?.hasAttribute(SPEAKING_ATTR)).toBe(true);
        readAloudStore.stop();
        expect(document.querySelector(`[${SPEAKING_ATTR}]`)).toBeNull();
    });

    it('pause keeps the place; resume speaks that sentence again', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        engine.finish();
        readAloudStore.toggle();
        expect(readAloudStore.status).toBe('paused');
        expect(engine.synth.cancel).toHaveBeenCalled();
        // The engine's late `end` for the cancelled sentence must not advance.
        engine.finish(1);
        expect(readAloudStore.index).toBe(1);
        expect(engine.texts()).toEqual(['One.', 'Two.']);

        readAloudStore.toggle();
        expect(readAloudStore.status).toBe('playing');
        expect(engine.texts()).toEqual(['One.', 'Two.', 'Two.']);
        // Pausing or resuming out of turn is a no-op.
        readAloudStore.resume();
        expect(engine.texts()).toHaveLength(3);
    });

    it('a new speed is stored and applies to the sentence in flight', async () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        readAloudStore.setRate(1.5);
        await flushIpc();
        expect(readAloudStore.rate).toBe(1.5);
        expect(harness?.callsFor('set_setting')).toEqual([
            { key: 'ui.reading.speech-rate', value: '1.5' },
        ]);
        expect(engine.texts()).toEqual(['One.', 'One.']);
        expect(engine.spoken[1]?.rate).toBe(1.5);
        // The superseded utterance's late end changes nothing.
        engine.finish(0);
        expect(engine.texts()).toHaveLength(2);

        // A speed not on offer, or the current one, is ignored.
        readAloudStore.setRate(3);
        readAloudStore.setRate(1.5);
        expect(engine.texts()).toHaveLength(2);
    });

    it('reads the stored speed, falling back on a bad one', async () => {
        harness?.teardown();
        let stored = '1.25';
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.reading.speech-rate' ? stored : null),
        });
        await settingsStore.load();
        expect(readAloudStore.rate).toBe(1.25);
        stored = '9';
        await settingsStore.load();
        expect(readAloudStore.rate).toBe(SPEECH_RATE_DEFAULT);
    });

    it('a paused speed change waits for resume', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        readAloudStore.pause();
        readAloudStore.setRate(2);
        expect(engine.texts()).toHaveLength(1);
        readAloudStore.resume();
        expect(engine.spoken[1]?.rate).toBe(2);
    });

    it('an interruption from outside pauses on the sentence; Resume picks it up', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        engine.finish();
        // The OS took the audio: no cancel of the reader's own came first.
        engine.fail('interrupted');
        expect(readAloudStore.status).toBe('paused');
        expect(readAloudStore.index).toBe(1);
        expect(toastStore.toasts).toHaveLength(0);
        readAloudStore.resume();
        expect(engine.texts()).toEqual(['One.', 'Two.', 'Two.']);
        engine.fail('canceled');
        expect(readAloudStore.status).toBe('paused');
    });

    it('an engine failure stops the read and says so', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        // A late error from a sentence the reader itself cancelled is ignored.
        readAloudStore.pause();
        engine.fail('interrupted');
        expect(readAloudStore.status).toBe('paused');
        readAloudStore.resume();
        engine.fail('synthesis-failed', 0);
        expect(readAloudStore.status).toBe('playing');

        engine.fail('synthesis-failed');
        expect(readAloudStore.status).toBe('idle');
        expect(toastStore.toasts.map((toast) => toast.message)).toEqual([
            'Couldn’t read the article aloud',
        ]);
    });

    it('an article with nothing to say is toasted, not started', () => {
        installSpeech();
        expect(readAloudStore.start(1, [], { lang: 'en' })).toBe(false);
        expect(readAloudStore.status).toBe('idle');
        expect(toastStore.toasts.map((toast) => toast.message)).toEqual([
            'This article has no text to read aloud',
        ]);
    });

    it('starting another article replaces the read in progress', () => {
        const engine = installSpeech();
        readAloudStore.start(1, sentences(), { lang: 'en' });
        readAloudStore.start(2, sentences('<p>Other.</p>'), { lang: 'en' });
        expect(readAloudStore.articleId).toBe(2);
        expect(engine.synth.cancel).toHaveBeenCalledTimes(1);
        engine.finish(0);
        expect(engine.texts()).toEqual(['One.', 'Other.']);
    });
});
