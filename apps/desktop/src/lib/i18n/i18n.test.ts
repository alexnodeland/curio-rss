/**
 * i18n routing: typed lookup, interpolation, and the Intl date helpers.
 */
import { formatIntlDate, formatIntlDateTime, t } from '$lib/i18n';
import { describe, expect, it } from 'vitest';

describe('t', () => {
    it('resolves a message key', () => {
        expect(t('app.title')).toBe('Curio');
    });

    it('interpolates {placeholders}', () => {
        expect(t('shell.feeds.count', { count: 12 })).toBe('12 feeds');
        expect(t('shell.unread.count', { count: 0 })).toBe('0 unread');
    });

    it('leaves unknown placeholders visible rather than eating them', () => {
        expect(t('shell.unread.count', {})).toBe('{count} unread');
    });

    it('selects the grammatical plural form by count', () => {
        expect(t('shell.feeds.count', { count: 1 })).toBe('1 feed');
        expect(t('shell.feeds.count', { count: 5 })).toBe('5 feeds');
        expect(t('reader.meta.words', { count: 1 })).toBe('1 word');
        expect(t('reader.meta.words', { count: 240 })).toBe('240 words');
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
