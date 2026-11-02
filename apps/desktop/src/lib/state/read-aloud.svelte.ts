/**
 * Read aloud (#153): the reader speaks the article on screen through the Web
 * Speech API, with a voice the OS synthesizes locally (`pickVoice`), so it
 * works offline and no article text leaves the machine. The store always
 * names that voice: where the OS has no local voice at all it says so and
 * does not read, rather than letting the engine fall back on a network one.
 * Chromium-family engines list no voices until `voiceschanged`, so a read's
 * first sentence waits (briefly) for the list.
 *
 * The article is spoken one sentence per utterance. That gives the sentence
 * highlight a dependable clock (each utterance's start, rather than word
 * `boundary` events that engines report unevenly), lets a speed change take
 * effect at once, and keeps every utterance short (some engines stall on long
 * ones). Pause cancels the sentence in flight and keeps its place, and resume
 * speaks that sentence again from its start — the same on every engine,
 * unlike `speechSynthesis.pause()`, which some never resume.
 *
 * The highlight is a CSS Custom Highlight over the sentence's range (nothing
 * in the sanitized markup changes); where the engine has no highlight
 * registry (WebKit before Safari 17.2), the sentence's paragraph carries
 * `data-speaking` instead.
 *
 * A cancelled utterance still reports `end` or `error` — late, and after its
 * successor may have started — so every callback checks it belongs to the
 * current generation and does nothing otherwise. A cancel error on the
 * current generation is not the reader's doing (the OS cut the audio off):
 * the read pauses on that sentence, so Resume picks it up.
 *
 * A read belongs to the rendered article: a sentence whose text has left the
 * page (the body re-rendered) ends the read rather than speaking text that is
 * no longer shown.
 */
import { t } from '$lib/i18n';
import { type SpokenSentence, pickVoice } from '$lib/utils/speech';
import { SETTING_KEYS, settingsStore } from './settings.svelte';
import { toastStore } from './toasts.svelte';

/** What the reader is doing with the article's audio. */
export type ReadAloudStatus = 'idle' | 'playing' | 'paused';

/** The speeds offered, as `SpeechSynthesisUtterance.rate` multipliers. */
export const SPEECH_RATES: readonly number[] = [0.75, 1, 1.25, 1.5, 2];

/** The speed until one is chosen. */
export const SPEECH_RATE_DEFAULT = 1;

/** The CSS highlight name the reader styles (`::highlight(…)`). */
export const SPEECH_HIGHLIGHT = 'curio-read-aloud';

/** The fallback highlight's attribute, on the spoken sentence's block. */
export const SPEAKING_ATTR = 'data-speaking';

/** The engine errors a cancel produces — an interruption, not a failure. */
const CANCEL_ERRORS: ReadonlySet<string> = new Set(['interrupted', 'canceled']);

/** How long a read waits for an engine that lists no voices yet (`voiceschanged`). */
export const VOICES_WAIT_MS = 1_500;

/** The page's speech engine, or `null` where the webview has none. */
function engine(): SpeechSynthesis | null {
    if (typeof window === 'undefined' || typeof window.SpeechSynthesisUtterance !== 'function') {
        return null;
    }
    return window.speechSynthesis ?? null;
}

/** The engine's highlight registry, or `null` where it has none. */
function highlightRegistry(): HighlightRegistry | null {
    const registry = (globalThis.CSS as { highlights?: HighlightRegistry } | undefined)?.highlights;
    return registry !== undefined && typeof globalThis.Highlight === 'function' ? registry : null;
}

/** A stored speed, if it is one of the offered ones. */
function parseRate(raw: string | undefined): number {
    const rate = Number(raw);
    return SPEECH_RATES.includes(rate) ? rate : SPEECH_RATE_DEFAULT;
}

/** What `start` needs beyond the sentences. */
export interface ReadAloudOptions {
    /** The article's language (a canonical tag), if it declares one. */
    readonly lang: string | null;
    /** The sentence to start at (default: the first) — where the reader is. */
    readonly from?: number;
    /** Called as each sentence starts (the reader follows it on screen). */
    readonly onSentence?: (sentence: SpokenSentence) => void;
}

export class ReadAloudStore {
    /** Idle, speaking, or paused on a sentence. */
    status: ReadAloudStatus = $state('idle');
    /** The article being read, `null` when idle. */
    articleId: number | null = $state(null);
    /** The sentence being spoken (or paused on), 0-based. */
    index: number = $state(0);
    /** How many sentences the article has. */
    total: number = $state(0);

    /** A speed chosen this session, ahead of the stored one. */
    #rate: number | null = $state(null);
    #sentences: readonly SpokenSentence[] = [];
    #options: ReadAloudOptions = { lang: null };
    #generation = 0;
    /** The local voice this read speaks in; `undefined` until it is chosen. */
    #voice: SpeechSynthesisVoice | undefined = undefined;
    /**
     * Utterances the engine has not finished with — held so their callbacks
     * fire (an engine may let an unreferenced utterance be collected first).
     */
    #inFlight: SpeechSynthesisUtterance[] = [];
    /** The block carrying the fallback highlight. */
    #marked: Element | null = null;

    /** Whether this webview can speak at all (the toolbar hides the action if not). */
    get supported(): boolean {
        return engine() !== null;
    }

    /** The speech rate: this session's choice, else the stored one. */
    get rate(): number {
        return this.#rate ?? parseRate(settingsStore.get(SETTING_KEYS.speechRate));
    }

    /** Whether `articleId` is the one being read (playing or paused). */
    isReading(articleId: number | null): boolean {
        return this.status !== 'idle' && articleId !== null && this.articleId === articleId;
    }

    /**
     * Reads `sentences` from `options.from` (default: the first), replacing
     * any read in progress. Returns whether it started (it does not without
     * an engine, or with nothing to say — the latter is toasted; a machine
     * with no local voice is found out, and toasted, as the read begins).
     */
    start(
        articleId: number,
        sentences: readonly SpokenSentence[],
        options: ReadAloudOptions,
    ): boolean {
        this.stop();
        const synth = engine();
        if (synth === null) {
            return false;
        }
        if (sentences.length === 0) {
            toastStore.showToast(t('toast.readAloudEmpty'), 'info');
            return false;
        }
        this.#sentences = sentences;
        this.#options = options;
        this.articleId = articleId;
        this.total = sentences.length;
        this.index = Math.min(Math.max(0, Math.trunc(options.from ?? 0)), sentences.length - 1);
        this.status = 'playing';
        this.#play(synth, this.index);
        return true;
    }

    /** Stops the sentence in flight, keeping its place. */
    pause(): void {
        if (this.status !== 'playing') {
            return;
        }
        this.#generation += 1;
        this.status = 'paused';
        engine()?.cancel();
    }

    /** Speaks on from the start of the paused sentence. */
    resume(): void {
        const synth = engine();
        if (this.status !== 'paused' || synth === null) {
            return;
        }
        this.status = 'playing';
        this.#play(synth, this.index);
    }

    /** Play/pause. */
    toggle(): void {
        if (this.status === 'playing') {
            this.pause();
        } else {
            this.resume();
        }
    }

    /** Ends the read: silence, no highlight, idle. A no-op when idle. */
    stop(): void {
        if (this.status === 'idle') {
            return;
        }
        this.#generation += 1;
        engine()?.cancel();
        this.#paint(null);
        this.#inFlight = [];
        this.#sentences = [];
        this.#options = { lang: null };
        this.#voice = undefined;
        this.status = 'idle';
        this.articleId = null;
        this.index = 0;
        this.total = 0;
    }

    /**
     * Sets the speed (one of {@link SPEECH_RATES}) and stores it. While
     * speaking, the current sentence restarts at the new speed.
     */
    setRate(rate: number): void {
        if (!SPEECH_RATES.includes(rate) || rate === this.rate) {
            return;
        }
        this.#rate = rate;
        void settingsStore.set(SETTING_KEYS.speechRate, String(rate));
        const synth = engine();
        if (this.status === 'playing' && synth !== null) {
            this.#generation += 1;
            synth.cancel();
            this.#play(synth, this.index);
        }
    }

    /** Test isolation. */
    reset(): void {
        this.stop();
        this.#rate = null;
    }

    /**
     * Speaks sentence `index` once the read's voice is known: at once when
     * it is, else after the engine lists its voices (or {@link VOICES_WAIT_MS}
     * passes). With no local voice to choose, the read ends and says why.
     */
    #play(synth: SpeechSynthesis, index: number): void {
        if (this.#voice !== undefined) {
            this.#speak(synth, this.#voice, index);
            return;
        }
        this.#generation += 1;
        const generation = this.#generation;
        const choose = (): void => {
            if (generation !== this.#generation) {
                return;
            }
            const voice = pickVoice(synth.getVoices(), this.#options.lang);
            if (voice === null) {
                this.stop();
                toastStore.showToast(t('toast.readAloudNoVoice'), 'error');
                return;
            }
            this.#voice = voice;
            this.#speak(synth, voice, index);
        };
        if (synth.getVoices().length > 0) {
            choose();
            return;
        }
        const listed = (): void => {
            clearTimeout(timer);
            synth.removeEventListener('voiceschanged', listed);
            choose();
        };
        const timer = setTimeout(listed, VOICES_WAIT_MS);
        synth.addEventListener('voiceschanged', listed);
    }

    #speak(synth: SpeechSynthesis, voice: SpeechSynthesisVoice, index: number): void {
        const sentence = this.#sentences[index];
        // Past the last sentence, or the body re-rendered under the read: its
        // text is gone from the page (a live range collapses when its nodes
        // are removed), so there is nothing left to read or highlight.
        if (
            sentence === undefined ||
            sentence.range.collapsed ||
            !sentence.range.startContainer.isConnected
        ) {
            this.stop();
            return;
        }
        this.#generation += 1;
        const generation = this.#generation;
        const { lang, onSentence } = this.#options;
        const utterance = new window.SpeechSynthesisUtterance(sentence.text);
        utterance.rate = this.rate;
        if (lang !== null) {
            utterance.lang = lang;
        }
        utterance.voice = voice;
        utterance.onend = () => {
            this.#release(utterance);
            if (generation === this.#generation) {
                this.#speak(synth, voice, index + 1);
            }
        };
        utterance.onerror = (event) => {
            this.#release(utterance);
            if (generation !== this.#generation) {
                return;
            }
            if (CANCEL_ERRORS.has(event.error)) {
                // Every cancel the reader makes moves the generation on first,
                // so this one came from outside (the OS took the audio, or the
                // engine dropped the sentence): hold the place, show Resume.
                this.pause();
                return;
            }
            this.stop();
            toastStore.showToast(t('toast.readAloudFailed'), 'error');
        };
        this.index = index;
        this.#inFlight.push(utterance);
        this.#paint(sentence);
        onSentence?.(sentence);
        synth.speak(utterance);
    }

    #release(utterance: SpeechSynthesisUtterance): void {
        this.#inFlight = this.#inFlight.filter((held) => held !== utterance);
    }

    /** Highlights `sentence` (or clears the highlight). */
    #paint(sentence: SpokenSentence | null): void {
        const registry = highlightRegistry();
        if (registry !== null) {
            if (sentence === null) {
                registry.delete(SPEECH_HIGHLIGHT);
            } else {
                registry.set(SPEECH_HIGHLIGHT, new Highlight(sentence.range));
            }
            return;
        }
        const block = sentence?.block ?? null;
        if (block === this.#marked) {
            return;
        }
        this.#marked?.removeAttribute(SPEAKING_ATTR);
        block?.setAttribute(SPEAKING_ATTR, '');
        this.#marked = block;
    }
}

export const readAloudStore = new ReadAloudStore();
