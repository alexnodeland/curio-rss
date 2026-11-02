/**
 * i18n routing: typed lookup, interpolation, display names, and the Intl
 * date/number helpers.
 */
import {
    LIST_DATE_RELATIVE_DAYS,
    displayName,
    formatIntlDate,
    formatIntlDateTime,
    formatIntlListDate,
    formatIntlNumber,
    localeStore,
    t,
} from '$lib/i18n';
import { afterEach, describe, expect, it } from 'vitest';

describe('t', () => {
    it('resolves a message key', () => {
        expect(t('app.title')).toBe('Curio');
    });

    it('interpolates {placeholders}', () => {
        expect(t('shell.feeds.count', { count: 12 })).toBe('12 feeds');
        expect(t('shell.unread.count', { count: 0 })).toBe('0 unread');
    });

    it('leaves unknown placeholders visible rather than eating them', () => {
        expect(t('reddit.postedBy', {})).toBe('Posted by u/{author}');
    });

    it('selects the grammatical plural form by count', () => {
        expect(t('shell.feeds.count', { count: 1 })).toBe('1 feed');
        expect(t('shell.feeds.count', { count: 5 })).toBe('5 feeds');
        expect(t('reader.meta.words', { count: 1 })).toBe('1 word');
        expect(t('reader.meta.words', { count: 240 })).toBe('240 words');
    });

    it('pluralizes each count in the compound summary strings (B3/B4)', () => {
        // Single-noun count keys inflect on their own count.
        expect(t('shell.unread.count', { count: 1 })).toBe('1 unread');
        expect(t('toast.markAllRead', { count: 3 })).toBe('Marked 3 read');
        // Compound strings pluralize every noun independently.
        expect(t('opml.imported', { added: 1, skipped: 0 })).toBe('Imported 1 feed, skipped 0');
        expect(t('import.done', { feeds: 2, articles: 1, skipped: 5 })).toBe(
            'Imported 2 feeds and 1 article, skipped 5',
        );
        expect(t('import.doneWithInvalid', { feeds: 1, articles: 0, skipped: 0, invalid: 1 })).toBe(
            'Imported 1 feed and 0 articles, skipped 0; ignored 1 invalid entry',
        );
        expect(t('refresh.announce.done', { count: 1, feeds: 4 })).toBe(
            'Refresh complete: 1 new article across 4 feeds',
        );
    });
});

describe('Intl date helpers', () => {
    it('omits the year for same-year dates', () => {
        const now = new Date('2026-07-06T12:00:00Z');
        expect(formatIntlDate(new Date('2026-03-05T12:00:00Z'), now)).toBe('March 5');
    });

    it('includes the year otherwise', () => {
        const now = new Date('2026-07-06T12:00:00Z');
        expect(formatIntlDate(new Date('2024-03-05T12:00:00Z'), now)).toBe('March 5, 2024');
    });

    it('formats a full datetime', () => {
        const formatted = formatIntlDateTime(new Date('2026-03-05T13:45:00'));
        expect(formatted).toContain('Mar 5, 2026');
        expect(formatted).toMatch(/1:45/);
    });
});

describe('formatIntlListDate — a list date scaled to its age', () => {
    // Local wall-clock dates, so the calendar-day maths holds in any TZ.
    const now = new Date(2026, 9, 31, 18, 30);

    afterEach(() => {
        localeStore.reset();
    });

    it('shows the time of day for an article published today', () => {
        expect(formatIntlListDate(new Date(2026, 9, 31, 9, 5), now)).toMatch(/^9:05\sAM$/);
        // Just after midnight is still today, not "yesterday".
        expect(formatIntlListDate(new Date(2026, 9, 31, 0, 1), now)).toMatch(/^12:01\sAM$/);
    });

    it('says "yesterday" by calendar day, not by 24-hour distance', () => {
        // 19 hours earlier, but on the previous calendar day.
        expect(formatIntlListDate(new Date(2026, 9, 30, 23, 30), now)).toBe('yesterday');
    });

    it('counts back relative days for the last few days', () => {
        expect(formatIntlListDate(new Date(2026, 9, 28, 12, 0), now)).toBe('3 days ago');
        const edge = new Date(2026, 9, 31 - LIST_DATE_RELATIVE_DAYS, 12, 0);
        expect(formatIntlListDate(edge, now)).toBe(`${LIST_DATE_RELATIVE_DAYS} days ago`);
    });

    it('falls back to the date beyond the relative window', () => {
        const older = new Date(2026, 9, 31 - LIST_DATE_RELATIVE_DAYS - 1, 12, 0);
        expect(formatIntlListDate(older, now)).toBe(formatIntlDate(older, now));
        expect(formatIntlListDate(new Date(2024, 2, 5, 12, 0), now)).toBe('March 5, 2024');
    });

    it('shows a future day (feed clock skew) as a date, never "in 2 days"', () => {
        expect(formatIntlListDate(new Date(2026, 10, 2, 8, 0), now)).toBe('November 2');
    });

    it('localizes the time and the relative day', () => {
        localeStore.active = 'de';
        // German reads the 24-hour clock — no AM/PM.
        expect(formatIntlListDate(new Date(2026, 9, 31, 17, 5), now)).toBe('17:05');
        expect(formatIntlListDate(new Date(2026, 9, 30, 9, 5), now)).toBe('gestern');
    });

    it('returns an empty string for an unparseable date', () => {
        expect(formatIntlListDate(new Date('not-a-date'), now)).toBe('');
    });
});

describe('formatIntlNumber', () => {
    afterEach(() => {
        localeStore.reset();
    });

    it('writes decimals in the active locale', () => {
        const oneDigit = { minimumFractionDigits: 1, maximumFractionDigits: 1 };
        expect(formatIntlNumber(1.6, oneDigit)).toBe('1.6');
        localeStore.active = 'de';
        expect(formatIntlNumber(1.6, oneDigit)).toBe('1,6');
        localeStore.active = 'fr';
        expect(formatIntlNumber(0.025, { minimumFractionDigits: 3 })).toBe('0,025');
    });

    it('pads to the requested precision', () => {
        expect(formatIntlNumber(1, { minimumFractionDigits: 1 })).toBe('1.0');
    });
});

describe('displayName', () => {
    afterEach(() => {
        localeStore.reset();
    });

    it('translates a keyed name in the active locale', () => {
        expect(displayName({ key: 'theme.name.dark' })).toBe('Dark');
        localeStore.active = 'de';
        expect(displayName({ key: 'theme.name.dark' })).toBe('Dunkel');
    });

    it('shows a proper noun verbatim in every locale', () => {
        localeStore.active = 'zh-Hans';
        expect(displayName({ proper: 'Nord' })).toBe('Nord');
    });
});
