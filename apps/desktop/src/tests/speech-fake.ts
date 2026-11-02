/**
 * A scripted Web Speech engine for the read-aloud tests: jsdom has no
 * `speechSynthesis`, and a real one would speak. `speak` only records the
 * utterance; a test then plays the engine's part — `finish(i)` fires the
 * utterance's `end`, `fail(i, error)` its `error` — and `cancel` records the
 * call without firing anything (a test fires the late `end` a real engine
 * may send after a cancel itself, to prove it is ignored).
 *
 * `installHighlights()` adds the CSS Custom Highlight registry jsdom also
 * lacks.
 */
import { vi } from 'vitest';

export class FakeUtterance {
    text: string;
    rate = 1;
    lang = '';
    voice: SpeechSynthesisVoice | null = null;
    onend: (() => void) | null = null;
    onerror: ((event: { error: string }) => void) | null = null;

    constructor(text: string) {
        this.text = text;
    }
}

export interface SpeechFake {
    readonly synth: {
        speak: ReturnType<typeof vi.fn>;
        cancel: ReturnType<typeof vi.fn>;
        getVoices: () => SpeechSynthesisVoice[];
    };
    /** Every utterance handed to `speak`, in order. */
    readonly spoken: FakeUtterance[];
    /** The texts handed to `speak`, in order. */
    texts(): string[];
    /** Fires the `end` of utterance `index` (default: the latest). */
    finish(index?: number): void;
    /** Fires the `error` of utterance `index` (default: the latest). */
    fail(error: string, index?: number): void;
}

/** A voice as the engine lists it. */
export function voice(
    name: string,
    lang: string,
    { local = true, isDefault = false }: { local?: boolean; isDefault?: boolean } = {},
): SpeechSynthesisVoice {
    return { name, lang, localService: local, default: isDefault, voiceURI: name };
}

/** Installs the engine (undo with `vi.unstubAllGlobals()`). */
export function installSpeech(voices: SpeechSynthesisVoice[] = []): SpeechFake {
    const spoken: FakeUtterance[] = [];
    const synth = {
        speak: vi.fn((utterance: FakeUtterance) => {
            spoken.push(utterance);
        }),
        cancel: vi.fn(),
        getVoices: () => voices,
    };
    vi.stubGlobal('speechSynthesis', synth);
    vi.stubGlobal('SpeechSynthesisUtterance', FakeUtterance);
    const at = (index: number | undefined): FakeUtterance => {
        const utterance = spoken[index ?? spoken.length - 1];
        if (utterance === undefined) {
            throw new Error('no such utterance');
        }
        return utterance;
    };
    return {
        synth,
        spoken,
        texts: () => spoken.map((utterance) => utterance.text),
        finish: (index) => at(index).onend?.(),
        fail: (error, index) => at(index).onerror?.({ error }),
    };
}

/** A CSS Custom Highlight registry; returns it for inspection. */
export function installHighlights(): Map<string, { ranges: Range[] }> {
    const registry = new Map<string, { ranges: Range[] }>();
    class FakeHighlight {
        ranges: Range[];
        constructor(...ranges: Range[]) {
            this.ranges = ranges;
        }
    }
    vi.stubGlobal('Highlight', FakeHighlight);
    vi.stubGlobal('CSS', { ...globalThis.CSS, highlights: registry });
    return registry;
}
