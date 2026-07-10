/**
 * Search-term segmentation: the pure basis of list highlighting. Verifies term
 * tokenising, case-insensitivity, overlap/adjacency merging, and the always-one
 * segment fallback so the renderer can treat "no highlight" uniformly.
 */
import { highlightSegments } from '$lib/utils/highlight';
import { describe, expect, it } from 'vitest';

describe('highlightSegments', () => {
    it('returns a single unmarked segment when there is nothing to match', () => {
        expect(highlightSegments('Hello world', '')).toEqual([
            { text: 'Hello world', mark: false },
        ]);
        expect(highlightSegments('', 'rust')).toEqual([{ text: '', mark: false }]);
        expect(highlightSegments('Hello world', 'xyz')).toEqual([
            { text: 'Hello world', mark: false },
        ]);
    });

    it('marks a case-insensitive term and its surrounding gaps', () => {
        expect(highlightSegments('The Rust Book', 'rust')).toEqual([
            { text: 'The ', mark: false },
            { text: 'Rust', mark: true },
            { text: ' Book', mark: false },
        ]);
    });

    it('marks every occurrence of every query term', () => {
        expect(highlightSegments('rust and rust bindings', 'rust bindings')).toEqual([
            { text: 'rust', mark: true },
            { text: ' and ', mark: false },
            { text: 'rust', mark: true },
            { text: ' ', mark: false },
            { text: 'bindings', mark: true },
        ]);
    });

    it('merges overlapping matches from different terms into one run', () => {
        // "ratio" and "ation" overlap in "ratation"? use a real overlap: "abc" + "bcd".
        expect(highlightSegments('abcd', 'abc bcd')).toEqual([{ text: 'abcd', mark: true }]);
    });

    it('ignores FTS punctuation/operators in the query', () => {
        expect(highlightSegments('async runtime', '"async" OR runtime*')).toEqual([
            { text: 'async', mark: true },
            { text: ' ', mark: false },
            { text: 'runtime', mark: true },
        ]);
    });
});
