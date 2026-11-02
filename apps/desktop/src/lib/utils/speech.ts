/**
 * Read-aloud text (#153): the sentences of the article as the reader renders
 * it — the title and the sanitized body, never the raw feed HTML — each with
 * the DOM `Range` it covers, so the sentence being spoken can be highlighted
 * in place without touching the `{@html}`-owned markup. Plus the voice choice
 * that keeps a read offline: only voices the OS synthesizes locally.
 */

/**
 * Text that is never read: preformatted code (read aloud it is noise),
 * footnote markers (the "1" in "footnote¹" would fuse into the word),
 * anything hidden from assistive tech, the reader's own controls and hints.
 */
const SKIP_SELECTOR = [
    'pre',
    'sup > a[href^="#"]',
    'a[href^="#"] > sup',
    'script',
    'style',
    'noscript',
    'template',
    'button',
    '[aria-hidden="true"]',
    '[hidden]',
    // A closed <details> shows only its summary.
    'details:not([open]) > :not(summary)',
    '.sr-only',
    '.media-hint',
].join(', ');

/** Elements that start a new run of text (a sentence never spans two). */
const BLOCK_SELECTOR = [
    'address',
    'article',
    'aside',
    'blockquote',
    'caption',
    'dd',
    'details',
    'div',
    'dl',
    'dt',
    'figcaption',
    'figure',
    'footer',
    'h1',
    'h2',
    'h3',
    'h4',
    'h5',
    'h6',
    'header',
    'li',
    'main',
    'nav',
    'ol',
    'p',
    'section',
    'summary',
    'table',
    'td',
    'th',
    'tr',
    'ul',
].join(', ');

/** Something worth saying: at least one letter or digit. */
const SPEAKABLE = /[\p{L}\p{N}]/u;

/** Sentence ends for engines without `Intl.Segmenter`. */
const FALLBACK_SENTENCE = /[^.!?。！？]+(?:[.!?。！？]+["'”’)\]]*|$)/gu;

/** One sentence to speak, and where it sits in the rendered article. */
export interface SpokenSentence {
    /** The sentence, whitespace collapsed. */
    readonly text: string;
    /** The rendered text it was read from (for the highlight). */
    readonly range: Range;
    /** The block it belongs to (the paragraph-level highlight fallback). */
    readonly block: Element;
}

/**
 * A BCP 47 tag for `lang`, or `null` when there is none or it is not one —
 * feeds declare languages loosely (`en_US`, `EN`, garbage), and both the
 * segmenter and `SpeechSynthesisUtterance.lang` want a well-formed tag.
 */
export function speechLang(lang: string | null | undefined): string | null {
    const trimmed = lang?.trim().replace(/_/g, '-') ?? '';
    if (trimmed === '') {
        return null;
    }
    try {
        return Intl.getCanonicalLocales(trimmed)[0] ?? null;
    } catch {
        return null;
    }
}

/**
 * The `[start, end)` spans of the sentences in `text`, trimmed of the
 * whitespace around them; spans with nothing speakable (a lone dash, an
 * ellipsis) are dropped. Uses the locale-aware `Intl.Segmenter`, with a
 * punctuation split for engines that lack it. Whitespace counts as HTML
 * renders it: a line break in the markup is a space, not a sentence end.
 */
export function sentenceSpans(source: string, lang: string | null): Array<[number, number]> {
    // Same length, so the spans index `source` too.
    const text = source.replace(/[\n\r\t\f\v]/gu, ' ');
    const spans: Array<[number, number]> = [];
    const push = (start: number, end: number): void => {
        let from = start;
        let to = end;
        while (from < to && /\s/u.test(text[from] ?? '')) {
            from += 1;
        }
        while (to > from && /\s/u.test(text[to - 1] ?? '')) {
            to -= 1;
        }
        if (SPEAKABLE.test(text.slice(from, to))) {
            spans.push([from, to]);
        }
    };
    if (typeof Intl.Segmenter === 'function') {
        const segmenter = new Intl.Segmenter(lang ?? undefined, { granularity: 'sentence' });
        for (const { index, segment } of segmenter.segment(text)) {
            push(index, index + segment.length);
        }
        return spans;
    }
    for (const match of text.matchAll(FALLBACK_SENTENCE)) {
        push(match.index, match.index + match[0].length);
    }
    return spans;
}

/** A run of text nodes inside one block, in document order. */
interface TextRun {
    readonly block: Element;
    readonly nodes: Text[];
}

/** The nearest block around `node` (or the root itself). */
function blockOf(node: Node, root: Element): Element {
    const parent = node.parentElement;
    const block = parent?.closest(BLOCK_SELECTOR) ?? null;
    return block !== null && root.contains(block) ? block : root;
}

/**
 * The root's text, grouped into runs: a run ends where the block changes or
 * at a `<br>` (a poem's line, an address), so a sentence never fuses two
 * lines that only look apart.
 */
function textRuns(root: Element): TextRun[] {
    const runs: TextRun[] = [];
    let current: TextRun | null = null;
    const walker = root.ownerDocument.createTreeWalker(
        root,
        NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT,
        {
            acceptNode(node: Node): number {
                if (node instanceof Element) {
                    if (node.matches(SKIP_SELECTOR)) {
                        return NodeFilter.FILTER_REJECT;
                    }
                    return node.localName === 'br'
                        ? NodeFilter.FILTER_ACCEPT
                        : NodeFilter.FILTER_SKIP;
                }
                // Bare text in a closed <details> is not shown either.
                return node.parentElement?.matches('details:not([open])') === true
                    ? NodeFilter.FILTER_REJECT
                    : NodeFilter.FILTER_ACCEPT;
            },
        },
    );
    for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
        if (!(node instanceof Text)) {
            current = null; // a <br>
            continue;
        }
        const block = blockOf(node, root);
        if (current === null || current.block !== block) {
            current = { block, nodes: [] };
            runs.push(current);
        }
        current.nodes.push(node);
    }
    return runs;
}

/** The text node and offset at character `offset` of a run. */
function locate(nodes: readonly Text[], offset: number, atEnd: boolean): [Text, number] | null {
    let start = 0;
    for (const node of nodes) {
        const length = node.data.length;
        const inside = atEnd ? offset <= start + length : offset < start + length;
        if (inside) {
            return [node, offset - start];
        }
        start += length;
    }
    return null;
}

/** The sentences of one run, each with its range. */
function runSentences(run: TextRun, lang: string | null): SpokenSentence[] {
    const text = run.nodes.map((node) => node.data).join('');
    const sentences: SpokenSentence[] = [];
    for (const [start, end] of sentenceSpans(text, lang)) {
        const from = locate(run.nodes, start, false);
        const to = locate(run.nodes, end, true);
        if (from === null || to === null) {
            continue;
        }
        const range = run.block.ownerDocument.createRange();
        range.setStart(from[0], from[1]);
        range.setEnd(to[0], to[1]);
        sentences.push({
            text: text.slice(start, end).replace(/\s+/gu, ' '),
            range,
            block: run.block,
        });
    }
    return sentences;
}

/**
 * Every sentence under `roots`, in order — the reader passes the title and
 * the rendered body. `lang` (from {@link speechLang}) steers the segmenter.
 */
export function collectSentences(roots: readonly Element[], lang: string | null): SpokenSentence[] {
    return roots.flatMap((root) => textRuns(root).flatMap((run) => runSentences(run, lang)));
}

/** A voice's language, comparable with a canonical tag. */
function voiceLang(voice: SpeechSynthesisVoice): string {
    return voice.lang.replace(/_/g, '-').toLowerCase();
}

/**
 * The voice to read in, from those the OS synthesizes on this machine
 * (`localService`) — never a network voice, so reading aloud works offline
 * and sends no article text anywhere. For a known language: the default
 * voice when it speaks that language, else any voice for the exact tag, else
 * any for the base language (`de-AT` → a `de-DE` voice). With no language,
 * or none installed for it, the system's default local voice (else the first
 * local one). `null` only when the OS has no local voice at all — the reader
 * then does not read, since the engine would pick its own (perhaps a network
 * one).
 */
export function pickVoice(
    voices: readonly SpeechSynthesisVoice[],
    lang: string | null,
): SpeechSynthesisVoice | null {
    const local = voices.filter((voice) => voice.localService);
    const fallback = local.find((voice) => voice.default) ?? local[0] ?? null;
    if (lang === null) {
        return fallback;
    }
    const want = lang.toLowerCase();
    const base = want.split('-')[0];
    const exact = (voice: SpeechSynthesisVoice): boolean => voiceLang(voice) === want;
    const sameBase = (voice: SpeechSynthesisVoice): boolean =>
        voiceLang(voice).split('-')[0] === base;
    return (
        local.find((voice) => voice.default && exact(voice)) ??
        local.find(exact) ??
        local.find((voice) => voice.default && sameBase(voice)) ??
        local.find(sameBase) ??
        fallback
    );
}
