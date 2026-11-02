/**
 * Formatting utilities — pure functions over wire values (RFC 3339 strings,
 * counts, byte sizes). Locale-sensitive date and number output routes through
 * the i18n layer's `Intl` helpers so the locale lives in exactly one place.
 */
import { formatIntlDate, formatIntlDateTime, formatIntlNumber } from '$lib/i18n';

/** Formats a date string as a human-readable date (year only when not this year). */
export function formatDate(dateString: string | null | undefined): string {
    if (!dateString) return '';
    return formatIntlDate(new Date(dateString));
}

/** Formats a date string as a full datetime. */
export function formatDateTime(dateString: string | null | undefined): string {
    if (!dateString) return '';
    return formatIntlDateTime(new Date(dateString));
}

/** Formats a duration in seconds as `H:MM:SS` or `M:SS`. */
export function formatDuration(seconds: number): string {
    if (!seconds || seconds < 0) return '0:00';

    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);

    if (hours > 0) {
        return `${hours}:${minutes.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
    }
    return `${minutes}:${secs.toString().padStart(2, '0')}`;
}

/** Formats a count compactly in the active locale (`1.5K` in English, `2,5 Mio.` in German). */
export function formatNumber(num: number): string {
    return formatIntlNumber(num, { notation: 'compact', maximumFractionDigits: 1 });
}

/** Formats a byte count as a human-readable size, the number in the active locale. */
export function formatFileSize(bytes: number): string {
    if (bytes === 0) return '0 B';

    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    const size = bytes / 1024 ** i;

    const digits = i === 0 ? 0 : 1;
    const value = formatIntlNumber(size, {
        minimumFractionDigits: digits,
        maximumFractionDigits: digits,
    });
    return `${value} ${units[i]}`;
}

/** Truncates text to `maxLength` with a trailing ellipsis. */
export function truncateText(text: string, maxLength: number): string {
    if (text.length <= maxLength) return text;
    return `${text.slice(0, maxLength - 1)}…`;
}

/** Strips HTML tags from a string (for plain-text previews, never rendering). */
export function stripHtml(html: string): string {
    const doc = new DOMParser().parseFromString(html, 'text/html');
    return doc.body.textContent || '';
}
