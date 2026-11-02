/**
 * The command palette's fuzzy matcher: does every character of the query
 * appear in the text, in order — and how well? Pure and synchronous, so the
 * palette re-ranks on every keystroke without a debounce.
 *
 * Matching folds case and diacritics one character at a time (`é` matches
 * `e`, `Ö` matches `o`), so the indices it reports point straight into the
 * original text for highlighting. A contiguous run scores far above a
 * scattered one, a run that starts a word (after a space, `/`, `-`, `_`, `.`
 * or a lower-to-upper case change) above one inside a word, and the start of
 * the text above everything: typing `star` puts "Go to starred" above "Restart".
 */

/** One match: its score (higher is better) and the matched text indices. */
export interface FuzzyMatch {
    readonly score: number;
    /**
     * The matched characters' positions in the original text, ascending —
     * counted in characters (`Array.from(text)`), not UTF-16 units.
     */
    readonly indices: readonly number[];
}

/** Characters after which a new word starts. */
const SEPARATORS = new Set([' ', '/', '-', '_', '.', ':', '·', '(', '[']);

/** Points per matched character, and the bonuses that order the matches. */
const SCORE = {
    char: 1,
    consecutive: 4,
    wordStart: 6,
    textStart: 8,
    /** Subtracted per skipped character between two matched ones (capped). */
    gap: 0.25,
    gapCap: 3,
} as const;

/** One character folded for comparison: lower-cased, diacritics stripped. */
function fold(char: string): string {
    const base = char.normalize('NFD').replace(/\p{M}/gu, '').toLowerCase();
    // Keep one-to-one positions: a character that folds to several (or none)
    // compares as its own lower-case form.
    return base.length === 1 ? base : char.toLowerCase();
}

/** `text` folded character by character (same length as `text`). */
function foldAll(text: string): string[] {
    return Array.from(text, fold);
}

/** `text` folded the way matching compares it (case and diacritics dropped). */
export function foldText(text: string): string {
    return foldAll(text).join('');
}

/** Whether the character at `index` of `raw` starts a word. */
function startsWord(raw: readonly string[], index: number): boolean {
    if (index === 0) {
        return true;
    }
    const previous = raw[index - 1];
    if (SEPARATORS.has(previous)) {
        return true;
    }
    // camelCase / PascalCase boundaries: "readLater" → "L".
    const current = raw[index];
    return previous === previous.toLowerCase() && current !== current.toLowerCase();
}

/** What a matched character at `index` earns for where it sits. */
function placeBonus(raw: readonly string[], index: number): number {
    if (index === 0) {
        return SCORE.textStart;
    }
    return startsWord(raw, index) ? SCORE.wordStart : 0;
}

/** What `gap` skipped characters between two matched ones cost (or earn). */
function gapScore(gap: number): number {
    return gap === 0 ? SCORE.consecutive : -Math.min(gap * SCORE.gap, SCORE.gapCap);
}

/** The score of a match at exactly `indices` of `raw`. */
function scoreAt(raw: readonly string[], indices: readonly number[]): number {
    let score = 0;
    indices.forEach((index, position) => {
        score += SCORE.char + placeBonus(raw, index);
        if (position > 0) {
            score += gapScore(index - indices[position - 1] - 1);
        }
    });
    return score;
}

/** The first index at or after `from` where `text` has `char` and `accept` holds, or -1. */
function indexOf(
    text: readonly string[],
    char: string,
    from: number,
    accept: (index: number) => boolean = () => true,
): number {
    for (let index = from; index < text.length; index += 1) {
        if (text[index] === char && accept(index)) {
            return index;
        }
    }
    return -1;
}

/**
 * Where `query[position]` goes, searching from `cursor` (just past the
 * previous hit): right at the cursor (a consecutive run) or at a word start
 * if it's there; else at a later word start when the rest of the query still
 * fits after it; else at the first occurrence. -1 when it doesn't occur.
 */
function nextHit(
    text: readonly string[],
    raw: readonly string[],
    query: readonly string[],
    position: number,
    cursor: number,
): number {
    const char = query[position];
    const found = indexOf(text, char, cursor);
    if (found === -1 || found === cursor || startsWord(raw, found)) {
        return found;
    }
    const wordStart = indexOf(text, char, found + 1, (index) => startsWord(raw, index));
    return wordStart !== -1 && fits(text, query, position + 1, wordStart + 1) ? wordStart : found;
}

/**
 * The leftmost in-order placement of `query` in `text` starting the first
 * character at `from`, preferring word starts for each later character when
 * one is reachable without giving up the match. `null` when it doesn't fit.
 */
function placeFrom(
    text: readonly string[],
    raw: readonly string[],
    query: readonly string[],
    from: number,
): number[] | null {
    const indices = [from];
    for (let position = 1; position < query.length; position += 1) {
        const hit = nextHit(text, raw, query, position, indices[position - 1] + 1);
        if (hit === -1) {
            return null;
        }
        indices.push(hit);
    }
    return indices;
}

/** Whether `query[position..]` occurs in order within `text[from..]`. */
function fits(
    text: readonly string[],
    query: readonly string[],
    position: number,
    from: number,
): boolean {
    let index = from;
    for (let at = position; at < query.length; at += 1) {
        while (index < text.length && text[index] !== query[at]) {
            index += 1;
        }
        if (index >= text.length) {
            return false;
        }
        index += 1;
    }
    return true;
}

/**
 * Matches `query` against `text`. An empty (or all-space) query matches
 * everything with score 0. Whitespace in the query is ignored, so a
 * multi-word query matches across words. Every start of the first character
 * is tried and the best placement kept.
 */
export function fuzzyMatch(query: string, text: string): FuzzyMatch | null {
    const folded = foldAll(query.replace(/\s+/g, ''));
    if (folded.length === 0) {
        return { score: 0, indices: [] };
    }
    const raw = Array.from(text);
    const haystack = foldAll(text);
    let best: FuzzyMatch | null = null;
    for (let from = 0; from < haystack.length; from += 1) {
        if (haystack[from] !== folded[0]) {
            continue;
        }
        const indices = placeFrom(haystack, raw, folded, from);
        if (indices === null) {
            break; // a later start can't fit what this one couldn't
        }
        const score = scoreAt(raw, indices);
        if (best === null || score > best.score) {
            best = { score, indices };
        }
    }
    return best;
}
