/**
 * Read-aloud text (#153): which text of the rendered article is spoken, how
 * it splits into sentences, that each sentence's range covers exactly its
 * text (the highlight), and which voice reads it — local voices only, so a
 * read works offline.
 */
import { collectSentences, pickVoice, sentenceSpans, speechLang } from '$lib/utils/speech';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { voice } from './speech-fake';

function dom(html: string): HTMLElement {
    const root = document.createElement('div');
    root.innerHTML = html;
    document.body.append(root);
    return root;
}

function texts(roots: Element[], lang: string | null = 'en'): string[] {
    return collectSentences(roots, lang).map((sentence) => sentence.text);
}

afterEach(() => {
    document.body.innerHTML = '';
    vi.unstubAllGlobals();
});

describe('speechLang', () => {
    it('canonicalizes the loose tags feeds declare', () => {
        expect(speechLang('en_US')).toBe('en-US');
        expect(speechLang('EN')).toBe('en');
        expect(speechLang(' de-at ')).toBe('de-AT');
    });

    it('is null for none, blank, or not a language tag', () => {
        expect(speechLang(null)).toBeNull();
        expect(speechLang(undefined)).toBeNull();
        expect(speechLang('  ')).toBeNull();
        expect(speechLang('not a tag!')).toBeNull();
    });
});

describe('sentenceSpans', () => {
    it('splits on sentence ends and trims the whitespace around each', () => {
        const text = '  First one. Second?  Third!  ';
        const spans = sentenceSpans(text, 'en');
        expect(spans.map(([start, end]) => text.slice(start, end))).toEqual([
            'First one.',
            'Second?',
            'Third!',
        ]);
    });

    it('drops spans with nothing to say', () => {
        expect(sentenceSpans(' … — ', 'en')).toEqual([]);
    });

    it('splits without Intl.Segmenter too', () => {
        vi.stubGlobal('Intl', { ...Intl, Segmenter: undefined });
        const text = 'One. Two! Three without an end';
        expect(sentenceSpans(text, 'en').map(([start, end]) => text.slice(start, end))).toEqual([
            'One.',
            'Two!',
            'Three without an end',
        ]);
        const cjk = '第一句。第二句！';
        expect(sentenceSpans(cjk, null).map(([start, end]) => cjk.slice(start, end))).toEqual([
            '第一句。',
            '第二句！',
        ]);
    });
});

describe('collectSentences', () => {
    it('reads the title, then each block of the body, sentence by sentence', () => {
        const title = dom('<h1>A <a href="#">linked</a> title</h1>').firstElementChild as Element;
        const body = dom(
            '<p>Hello <em>world</em>. Second  sentence\nhere.</p><ul><li>An item</li></ul><blockquote><p>Quoted.</p></blockquote>',
        );
        expect(texts([title, body])).toEqual([
            'A linked title',
            'Hello world.',
            'Second sentence here.',
            'An item',
            'Quoted.',
        ]);
    });

    it('never fuses two blocks or two lines into one sentence', () => {
        const body = dom('<p>No full stop</p><p>Next block</p><p>Line one<br>Line two</p>');
        expect(texts([body])).toEqual(['No full stop', 'Next block', 'Line one', 'Line two']);
    });

    it('skips code blocks, hidden text and controls', () => {
        const body = dom(
            '<p>Kept.</p><pre>let x = 1;</pre><p aria-hidden="true">Hidden.</p><p hidden>Gone.</p><button>Press</button><span class="sr-only">Screen reader only.</span><p>Inline <code>code</code> stays, a note<sup><a href="#fn1">1</a></sup> does not.</p>',
        );
        expect(texts([body])).toEqual(['Kept.', 'Inline code stays, a note does not.']);
    });

    it('gives each sentence the range of its rendered text and its block', () => {
        const body = dom('<p>Hello <em>wor</em>ld. Bye <strong>now</strong>.</p>');
        const sentences = collectSentences([body], 'en');
        expect(sentences.map((sentence) => sentence.range.toString())).toEqual([
            'Hello world.',
            'Bye now.',
        ]);
        expect(sentences.every((sentence) => sentence.block === body.querySelector('p'))).toBe(
            true,
        );
    });

    it('text straight in a root belongs to that root', () => {
        const body = dom('Loose text. More.');
        const sentences = collectSentences([body], 'en');
        expect(sentences.map((sentence) => sentence.text)).toEqual(['Loose text.', 'More.']);
        expect(sentences[0]?.block).toBe(body);
    });

    it('finds nothing in an empty body', () => {
        expect(collectSentences([dom('<p> </p><img alt="x">')], 'en')).toEqual([]);
    });
});

describe('pickVoice', () => {
    const voices = [
        voice('Cloud', 'en-US', { local: false, isDefault: true }),
        voice('Anna', 'de-DE'),
        voice('Petra', 'de_DE', { isDefault: true }),
        voice('Samantha', 'en-US'),
        voice('Daniel', 'en-GB'),
    ];

    it('never picks a network voice', () => {
        expect(pickVoice([voice('Cloud', 'en-US', { local: false })], 'en-US')).toBeNull();
        expect(
            pickVoice([voice('Cloud', 'en-US', { local: false, isDefault: true })], null),
        ).toBeNull();
    });

    it('prefers the default voice for the exact language, then any for it', () => {
        expect(pickVoice(voices, 'de-DE')?.name).toBe('Petra');
        expect(pickVoice(voices, 'en-GB')?.name).toBe('Daniel');
    });

    it('falls back to a voice for the base language', () => {
        expect(pickVoice(voices, 'de-AT')?.name).toBe('Petra');
        expect(pickVoice(voices, 'en')?.name).toBe('Samantha');
    });

    it('is null for a language no local voice speaks', () => {
        expect(pickVoice(voices, 'fr')).toBeNull();
    });

    it('with no language, takes the default local voice', () => {
        expect(pickVoice(voices, null)?.name).toBe('Petra');
        expect(pickVoice([voice('Anna', 'de-DE')], null)).toBeNull();
    });
});
