/**
 * Formatting utilities — pure functions over wire values (RFC 3339 strings,
 * counts, byte sizes). Locale-sensitive date output routes through the i18n
 * layer's `Intl` helpers so the locale lives in exactly one place.
 */
import { formatIntlDate, formatIntlDateTime } from '$lib/i18n';

/** Formats a date string as a relative time (e.g. `4h ago`). */
export function formatRelativeTime(dateString: string | null | undefined): string {
    if (!dateString) return '';

    const date = new Date(dateString);
    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffSeconds = Math.floor(diffMs / 1000);
    const diffMinutes = Math.floor(diffSeconds / 60);
    const diffHours = Math.floor(diffMinutes / 60);
    const diffDays = Math.floor(diffHours / 24);
    const diffWeeks = Math.floor(diffDays / 7);
    const diffMonths = Math.floor(diffDays / 30);
    const diffYears = Math.floor(diffDays / 365);

    if (diffSeconds < 60) {
        return 'just now';
    }
    if (diffMinutes < 60) {
        return `${diffMinutes}m ago`;
    }
    if (diffHours < 24) {
        return `${diffHours}h ago`;
    }
    if (diffDays < 7) {
        return `${diffDays}d ago`;
    }
    if (diffWeeks < 4) {
        return `${diffWeeks}w ago`;
    }
    if (diffMonths < 12) {
        return `${diffMonths}mo ago`;
    }
    return `${diffYears}y ago`;
}

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

/** Formats a count with `K`/`M`/`B` suffixes. */
export function formatNumber(num: number): string {
    if (num >= 1_000_000_000) {
        return `${(num / 1_000_000_000).toFixed(1)}B`;
    }
    if (num >= 1_000_000) {
        return `${(num / 1_000_000).toFixed(1)}M`;
    }
    if (num >= 1_000) {
        return `${(num / 1_000).toFixed(1)}K`;
    }
    return num.toString();
}

/** Formats a byte count as a human-readable size. */
export function formatFileSize(bytes: number): string {
    if (bytes === 0) return '0 B';

    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    const i = Math.floor(Math.log(bytes) / Math.log(1024));
    const size = bytes / 1024 ** i;

    return `${size.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
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
