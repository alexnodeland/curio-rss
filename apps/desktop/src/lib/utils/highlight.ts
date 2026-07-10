/**
 * Search-term highlighting: splits a plain string into matched / unmatched runs
 * for each term of a query, so the caller can wrap the matched runs in `<mark>`
 * without any raw-HTML injection (the segments are rendered as escaped text).
 * Kept pure and DOM-free so the (fiddly) range-merge logic is unit-tested.
 */

export interface HighlightSegment {
    readonly text: string;
    /** Whether this run matched a query term (⇒ render inside `<mark>`). */
    readonly mark: boolean;
}

/** The query's word tokens (letters/numbers), lower-cased; FTS operators drop out. */
function queryTerms(query: string): string[] {
    return query
        .toLowerCase()
        .split(/[^\p{L}\p{N}]+/u)
        .filter((term) => term.length > 0);
}

/**
 * Segments `text` against `query`: each case-insensitive occurrence of any query
 * term becomes a `mark: true` run, the gaps between them `mark: false`. Returns a
 * single unmarked segment when there is nothing to highlight, so the caller can
 * always render the same way. Overlapping/adjacent matches are merged.
 */
export function highlightSegments(text: string, query: string): HighlightSegment[] {
    const terms = queryTerms(query);
    if (terms.length === 0 || text.length === 0) {
        return [{ text, mark: false }];
    }
    const haystack = text.toLowerCase();
    const ranges: [number, number][] = [];
    for (const term of terms) {
        for (
            let from = haystack.indexOf(term);
            from !== -1;
            from = haystack.indexOf(term, from + term.length)
        ) {
            ranges.push([from, from + term.length]);
        }
    }
    if (ranges.length === 0) {
        return [{ text, mark: false }];
    }
    ranges.sort((a, b) => a[0] - b[0]);
    const merged: [number, number][] = [];
    for (const [start, end] of ranges) {
        const last = merged.at(-1);
        if (last !== undefined && start <= last[1]) {
            last[1] = Math.max(last[1], end);
        } else {
            merged.push([start, end]);
        }
    }
    const segments: HighlightSegment[] = [];
    let cursor = 0;
    for (const [start, end] of merged) {
        if (start > cursor) {
            segments.push({ text: text.slice(cursor, start), mark: false });
        }
        segments.push({ text: text.slice(start, end), mark: true });
        cursor = end;
    }
    if (cursor < text.length) {
        segments.push({ text: text.slice(cursor), mark: false });
    }
    return segments;
}
