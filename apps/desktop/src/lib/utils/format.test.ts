import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    formatDate,
    formatDateTime,
    formatDuration,
    formatFileSize,
    formatNumber,
    formatRelativeTime,
    stripHtml,
    truncateText,
} from './format';

describe('formatRelativeTime', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(new Date('2025-06-15T12:00:00Z'));
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('returns empty string for null input', () => {
        expect(formatRelativeTime(null)).toBe('');
    });

    it('returns empty string for undefined input', () => {
        expect(formatRelativeTime(undefined)).toBe('');
    });

    it('formats seconds ago as just now', () => {
        expect(formatRelativeTime('2025-06-15T11:59:30Z')).toBe('just now');
    });

    it('formats minutes ago', () => {
        expect(formatRelativeTime('2025-06-15T11:45:00Z')).toBe('15m ago');
    });

    it('formats hours ago', () => {
        expect(formatRelativeTime('2025-06-15T08:00:00Z')).toBe('4h ago');
    });

    it('formats days ago', () => {
        expect(formatRelativeTime('2025-06-13T12:00:00Z')).toBe('2d ago');
    });

    it('formats weeks ago', () => {
        expect(formatRelativeTime('2025-06-01T12:00:00Z')).toBe('2w ago');
    });

    it('formats months ago', () => {
        expect(formatRelativeTime('2025-03-15T12:00:00Z')).toBe('3mo ago');
    });

    it('formats years ago', () => {
        expect(formatRelativeTime('2023-06-15T12:00:00Z')).toBe('2y ago');
    });
});

describe('formatDate', () => {
    beforeEach(() => {
        vi.useFakeTimers();
        vi.setSystemTime(new Date('2025-06-15T12:00:00Z'));
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('returns empty string for null input', () => {
        expect(formatDate(null)).toBe('');
    });

    it('formats date in same year without year', () => {
        const result = formatDate('2025-03-20T10:00:00Z');
        expect(result).toBe('March 20');
    });

    it('formats date in different year with year', () => {
        const result = formatDate('2024-03-20T10:00:00Z');
        expect(result).toBe('March 20, 2024');
    });
});

describe('formatDateTime', () => {
    it('returns empty string for null input', () => {
        expect(formatDateTime(null)).toBe('');
    });

    it('formats full datetime', () => {
        const result = formatDateTime('2025-03-20T14:30:00Z');
        // Output format depends on locale, just verify it contains expected parts
        expect(result).toContain('20');
        expect(result).toContain('2025');
    });
});

describe('formatDuration', () => {
    it('formats zero as 0:00', () => {
        expect(formatDuration(0)).toBe('0:00');
    });

    it('formats negative as 0:00', () => {
        expect(formatDuration(-5)).toBe('0:00');
    });

    it('formats seconds only', () => {
        expect(formatDuration(45)).toBe('0:45');
    });

    it('formats minutes and seconds', () => {
        expect(formatDuration(185)).toBe('3:05');
    });

    it('formats hours, minutes and seconds', () => {
        expect(formatDuration(3725)).toBe('1:02:05');
    });

    it('formats large durations', () => {
        expect(formatDuration(7385)).toBe('2:03:05');
    });
});

describe('formatNumber', () => {
    it('formats small numbers as-is', () => {
        expect(formatNumber(123)).toBe('123');
    });

    it('formats thousands with K suffix', () => {
        expect(formatNumber(1500)).toBe('1.5K');
    });

    it('formats millions with M suffix', () => {
        expect(formatNumber(2500000)).toBe('2.5M');
    });

    it('formats billions with B suffix', () => {
        expect(formatNumber(1200000000)).toBe('1.2B');
    });
});

describe('formatFileSize', () => {
    it('formats zero bytes', () => {
        expect(formatFileSize(0)).toBe('0 B');
    });

    it('formats bytes', () => {
        expect(formatFileSize(500)).toBe('500 B');
    });

    it('formats kilobytes', () => {
        expect(formatFileSize(1536)).toBe('1.5 KB');
    });

    it('formats megabytes', () => {
        expect(formatFileSize(1048576)).toBe('1.0 MB');
    });

    it('formats gigabytes', () => {
        expect(formatFileSize(1610612736)).toBe('1.5 GB');
    });
});

describe('truncateText', () => {
    it('returns text unchanged if shorter than limit', () => {
        expect(truncateText('Hello', 10)).toBe('Hello');
    });

    it('truncates text at limit with ellipsis', () => {
        expect(truncateText('Hello World', 8)).toBe('Hello W…');
    });

    it('handles exact length', () => {
        expect(truncateText('Hello', 5)).toBe('Hello');
    });
});

describe('stripHtml', () => {
    it('removes HTML tags', () => {
        expect(stripHtml('<p>Hello <strong>World</strong></p>')).toBe('Hello World');
    });

    it('handles empty string', () => {
        expect(stripHtml('')).toBe('');
    });

    it('handles text without HTML', () => {
        expect(stripHtml('Plain text')).toBe('Plain text');
    });

    it('decodes HTML entities', () => {
        expect(stripHtml('&lt;script&gt;')).toBe('<script>');
    });
});
