/**
 * The article display facts core encodes in data: a link-less entry carries
 * its feed's own URL as `source_url`, and a date-less entry keeps
 * `published_at` null (dated by its first-seen `saved_at` instead).
 */
import { articleDate, isLinkless, listDateLabel } from '$lib/utils/article';
import { describe, expect, it } from 'vitest';
import { articleSummaryFixture, feedFixture } from './ipc-harness';

const FEEDS = [
    feedFixture({ id: 1, url: 'https://example.org/feed.xml' }),
    feedFixture({ id: 2, url: 'https://other.example/rss' }),
];

describe('isLinkless', () => {
    it('is true when the source URL is the owning feed’s own URL', () => {
        const article = articleSummaryFixture({
            feed_id: 1,
            source_url: 'https://example.org/feed.xml',
        });
        expect(isLinkless(article, FEEDS)).toBe(true);
    });

    it('is false for an entry with a link of its own', () => {
        expect(isLinkless(articleSummaryFixture({ feed_id: 1 }), FEEDS)).toBe(false);
    });

    it('matches only the owning feed, exactly', () => {
        // Another feed's URL is still a real (if odd) link for this article.
        const foreign = articleSummaryFixture({
            feed_id: 1,
            source_url: 'https://other.example/rss',
        });
        expect(isLinkless(foreign, FEEDS)).toBe(false);
        // Core compares exactly — so does the UI.
        const trailing = articleSummaryFixture({
            feed_id: 1,
            source_url: 'https://example.org/feed.xml/',
        });
        expect(isLinkless(trailing, FEEDS)).toBe(false);
    });

    it('is false without a known feed (feedless save, feeds not loaded)', () => {
        const feedless = articleSummaryFixture({
            feed_id: null,
            source_url: 'https://example.org/feed.xml',
        });
        expect(isLinkless(feedless, FEEDS)).toBe(false);
        const unloaded = articleSummaryFixture({
            feed_id: 1,
            source_url: 'https://example.org/feed.xml',
        });
        expect(isLinkless(unloaded, [])).toBe(false);
    });
});

describe('articleDate', () => {
    it('uses the publication date when the source declared one', () => {
        const article = articleSummaryFixture({
            published_at: '2026-07-01T09:00:00.000Z',
            saved_at: '2026-07-02T10:00:00.000Z',
        });
        expect(articleDate(article)).toEqual({
            iso: '2026-07-01T09:00:00.000Z',
            firstSeen: false,
        });
    });

    it('falls back to the first-seen time, flagged, when it did not', () => {
        const article = articleSummaryFixture({
            published_at: null,
            saved_at: '2026-07-02T10:00:00.000Z',
        });
        expect(articleDate(article)).toEqual({
            iso: '2026-07-02T10:00:00.000Z',
            firstSeen: true,
        });
    });
});

describe('listDateLabel', () => {
    it('labels the publication date as-is', () => {
        const article = articleSummaryFixture({
            published_at: '2026-07-01T12:00:00.000Z',
            saved_at: '2026-07-02T12:00:00.000Z',
        });
        const label = listDateLabel(article);
        expect(label?.firstSeen).toBe(false);
        expect(label?.text).toContain('July 1');
        expect(label?.text).not.toContain('First seen');
    });

    it('prefixes the first-seen date for a date-less entry', () => {
        const article = articleSummaryFixture({
            published_at: null,
            saved_at: '2026-07-02T12:00:00.000Z',
        });
        const label = listDateLabel(article);
        expect(label?.firstSeen).toBe(true);
        expect(label?.text).toMatch(/^First seen July 2/);
    });

    it('is null for an unparseable timestamp, so no date renders', () => {
        expect(listDateLabel(articleSummaryFixture({ published_at: 'not a date' }))).toBeNull();
    });
});
