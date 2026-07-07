/**
 * Feed-tree derivation: `/`-path tags become a folder hierarchy, feeds sit
 * under every tag they carry, tagless feeds fall out as `ungrouped`, and the
 * whole thing sorts deterministically.
 */
import { buildFeedTree, subtreeFeedIds, tagSegments } from '$lib/state/feed-tree';
import { describe, expect, it } from 'vitest';
import { feedFixture } from './ipc-harness';

describe('tagSegments', () => {
    it('splits on / and drops empty or whitespace segments', () => {
        expect(tagSegments('Tech/Databases')).toEqual(['Tech', 'Databases']);
        expect(tagSegments(' Tech / Databases ')).toEqual(['Tech', 'Databases']);
        expect(tagSegments('/Tech//Databases/')).toEqual(['Tech', 'Databases']);
        expect(tagSegments('')).toEqual([]);
        expect(tagSegments('   ')).toEqual([]);
    });
});

describe('buildFeedTree', () => {
    it('nests feeds under their path tags, sorted, with tagless feeds ungrouped', () => {
        const rust = feedFixture({ id: 1, title: 'Rust Blog', tags: ['Tech'] });
        const sqlite = feedFixture({ id: 2, title: 'SQLite', tags: ['Tech/Databases'] });
        const bread = feedFixture({ id: 3, title: 'Bread', tags: ['Cooking'] });
        const plain = feedFixture({ id: 4, title: 'Plain', tags: [] });
        const tree = buildFeedTree([sqlite, rust, bread, plain]);

        expect(tree.folders.map((folder) => folder.name)).toEqual(['Cooking', 'Tech']);
        const tech = tree.folders[1];
        expect(tech.path).toBe('Tech');
        expect(tech.feeds.map((feed) => feed.id)).toEqual([1]);
        expect(tech.subfolders.map((sub) => sub.name)).toEqual(['Databases']);
        const databases = tech.subfolders[0];
        expect(databases.path).toBe('Tech/Databases');
        expect(databases.feeds.map((feed) => feed.id)).toEqual([2]);
        expect(tree.ungrouped.map((feed) => feed.id)).toEqual([4]);
    });

    it('places a feed under every one of its tags (multi-membership)', () => {
        const feed = feedFixture({ id: 7, title: 'Cross', tags: ['A', 'B/C'] });
        const tree = buildFeedTree([feed]);

        expect(tree.folders.map((folder) => folder.name)).toEqual(['A', 'B']);
        expect(tree.folders[0].feeds.map((entry) => entry.id)).toEqual([7]);
        expect(tree.folders[1].subfolders[0].feeds.map((entry) => entry.id)).toEqual([7]);
    });

    it('sorts feeds within a folder by label', () => {
        const zebra = feedFixture({ id: 1, title: 'Zebra', tags: ['X'] });
        const apple = feedFixture({ id: 2, title: 'Apple', tags: ['X'] });
        const tree = buildFeedTree([zebra, apple]);

        expect(tree.folders[0].feeds.map((feed) => feed.title)).toEqual(['Apple', 'Zebra']);
    });
});

describe('subtreeFeedIds', () => {
    it('collects distinct feed ids across the subtree (a shared feed counts once)', () => {
        const shared = feedFixture({ id: 9, tags: ['Tech', 'Tech/Databases'] });
        const only = feedFixture({ id: 10, tags: ['Tech/Databases'] });
        const tree = buildFeedTree([shared, only]);

        const ids = [...subtreeFeedIds(tree.folders[0])].sort((a, b) => a - b);
        expect(ids).toEqual([9, 10]);
    });
});
