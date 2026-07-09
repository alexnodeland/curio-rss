/**
 * The pure tag-path transforms behind the sidebar's folder operations: move a
 * feed into a folder, ungroup it, rename a folder (prefix rewrite), delete a
 * folder (feeds move to parent, never unsubscribed), and enumerate folder paths.
 * Tags are a set — every transform preserves unrelated tags and dedupes.
 */
import {
    allFolderPaths,
    feedInFolder,
    tagsForDelete,
    tagsForMove,
    tagsForRename,
    tagsForUngroup,
} from '$lib/state/folder-ops';
import { describe, expect, it } from 'vitest';
import { feedFixture } from './ipc-harness';

describe('tagsForMove', () => {
    it('replaces the first path tag with the folder, preserving the rest', () => {
        expect(tagsForMove(['Old', 'Keep'], 'New')).toEqual(['New', 'Keep']);
    });
    it('adds the folder when the feed has no tags', () => {
        expect(tagsForMove([], 'Tech')).toEqual(['Tech']);
    });
    it('dedupes when the move collides with an existing tag', () => {
        expect(tagsForMove(['A', 'B'], 'B')).toEqual(['B']);
    });
});

describe('tagsForUngroup', () => {
    it('drops every path tag', () => {
        expect(tagsForUngroup(['A', 'B/C'])).toEqual([]);
    });
});

describe('tagsForRename', () => {
    it('rewrites the exact path and its descendants, leaving others', () => {
        expect(tagsForRename(['Tech', 'Tech/DB', 'News'], 'Tech', 'Eng')).toEqual([
            'Eng',
            'Eng/DB',
            'News',
        ]);
    });
    it('does not touch a same-prefix sibling that is not a path descendant', () => {
        expect(tagsForRename(['Technology'], 'Tech', 'Eng')).toEqual(['Technology']);
    });
});

describe('tagsForDelete', () => {
    it('moves a nested folder up to its parent', () => {
        expect(tagsForDelete(['A/B', 'A/B/C', 'Other'], 'A/B')).toEqual(['A', 'A/C', 'Other']);
    });
    it('ungroups feeds when a top-level folder is deleted', () => {
        expect(tagsForDelete(['Tech', 'Tech/DB'], 'Tech')).toEqual(['DB']);
    });
});

describe('feedInFolder', () => {
    it('matches the exact path and descendants, not siblings', () => {
        const feed = feedFixture({ tags: ['Tech/DB'] });
        expect(feedInFolder(feed, 'Tech')).toBe(true);
        expect(feedInFolder(feed, 'Tech/DB')).toBe(true);
        expect(feedInFolder(feed, 'Techno')).toBe(false);
        expect(feedInFolder(feed, 'News')).toBe(false);
    });
});

describe('allFolderPaths', () => {
    it('lists every distinct path and its ancestors, sorted', () => {
        const feeds = [
            feedFixture({ id: 1, tags: ['Tech/DB'] }),
            feedFixture({ id: 2, tags: ['News'] }),
        ];
        expect(allFolderPaths(feeds)).toEqual(['News', 'Tech', 'Tech/DB']);
    });
});
