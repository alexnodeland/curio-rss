/**
 * Reading aids' arithmetic (#153): the estimated reading time, and the
 * mapping between a scroll position and the stored reading position.
 *
 * The position is kept in thousandths of the body's scroll extent
 * (`scrollHeight - clientHeight`) — the `read_permille` the backend stores.
 * A fraction of the extent survives a window resize or a font-size change
 * well enough to land within a paragraph, which is all a resume needs.
 */

/** Words per minute for the estimate — a common adult silent-reading pace. */
export const WORDS_PER_MINUTE = 230;

/** Thousandths per whole article (`read_permille` at the end). */
export const PERMILLE_END = 1000;

/**
 * Pixels within which the body counts as at its top or its end — sub-pixel
 * scroll offsets and a last line of slack never read as "1 % read" or "99 %".
 */
export const EDGE_SLACK_PX = 4;

/**
 * Whole minutes to read `wordCount` words, at least one; `null` when the
 * count is unknown or zero (nothing to estimate).
 */
export function readMinutes(wordCount: number | null): number | null {
    if (wordCount === null || !Number.isFinite(wordCount) || wordCount <= 0) {
        return null;
    }
    return Math.max(1, Math.round(wordCount / WORDS_PER_MINUTE));
}

/** The geometry a position is read from (a scroll container's own fields). */
export interface ScrollGeometry {
    readonly scrollTop: number;
    readonly scrollHeight: number;
    readonly clientHeight: number;
}

/**
 * The reading position of a scroll container, in thousandths — `null` when
 * the body fits without scrolling (there is no position to keep).
 */
export function permilleAt({
    scrollTop,
    scrollHeight,
    clientHeight,
}: ScrollGeometry): number | null {
    const extent = scrollHeight - clientHeight;
    if (!(extent > EDGE_SLACK_PX)) {
        return null;
    }
    if (scrollTop <= EDGE_SLACK_PX) {
        return 0;
    }
    if (scrollTop >= extent - EDGE_SLACK_PX) {
        return PERMILLE_END;
    }
    return Math.min(PERMILLE_END, Math.max(0, Math.round((scrollTop / extent) * PERMILLE_END)));
}

/** The `scrollTop` that puts a container back at `permille`. */
export function scrollTopFor(permille: number, scrollHeight: number, clientHeight: number): number {
    const extent = Math.max(0, scrollHeight - clientHeight);
    const clamped = Math.min(PERMILLE_END, Math.max(0, permille));
    return Math.round((clamped / PERMILLE_END) * extent);
}

/**
 * Whether a stored position is worth resuming at: somewhere past the top
 * and short of the end. A finished article opens at the top again (you are
 * rereading it), and an untouched one simply starts there.
 */
export function resumable(permille: number): boolean {
    return permille > 0 && permille < PERMILLE_END;
}

/** How long a resume keeps the body pinned while late layout settles. */
export const RESUME_HOLD_MS = 2000;

/** The inputs that mean the reader took over the scroll from a resume. */
const TAKEOVER_EVENTS = ['wheel', 'touchstart', 'pointerdown', 'keydown'] as const;

/**
 * Puts `body` back at `permille`, then holds it there while late layout
 * (images arriving, a web font swapping in) grows the content — re-pinning
 * the same fraction on every resize — until the reader scrolls themselves
 * or {@link RESUME_HOLD_MS} passes. "Scrolls themselves" is an input on the
 * body, or any scroll that leaves the body somewhere the pin did not put it:
 * a page step from a global shortcut (Space with focus in the list) moves the
 * body without an input landing on it, and must not be undone by the next
 * image to load. The jump is instant even where the body scrolls smoothly: a
 * resume is not a glide. Returns a cancel.
 */
export function resumeScroll(body: HTMLElement, permille: number): () => void {
    /** Where the last pin left the body (as the browser clamped it). */
    let pinned = 0;
    const pin = (): void => {
        const previous = body.style.scrollBehavior;
        body.style.scrollBehavior = 'auto';
        body.scrollTop = scrollTopFor(permille, body.scrollHeight, body.clientHeight);
        pinned = body.scrollTop;
        body.style.scrollBehavior = previous;
    };
    pin();
    const content = body.firstElementChild;
    const observer =
        typeof ResizeObserver === 'undefined' || content === null ? null : new ResizeObserver(pin);
    if (observer !== null && content !== null) {
        observer.observe(content);
    }
    let timer: ReturnType<typeof setTimeout> | null = null;
    const release = (): void => {
        observer?.disconnect();
        if (timer !== null) {
            clearTimeout(timer);
            timer = null;
        }
        for (const type of TAKEOVER_EVENTS) {
            body.removeEventListener(type, release);
        }
        body.removeEventListener('scroll', onScroll);
    };
    // A pin's own scroll lands where the pin put it; anything else is the
    // reader (or a shortcut acting for them) moving on.
    const onScroll = (): void => {
        if (Math.abs(body.scrollTop - pinned) > 1) {
            release();
        }
    };
    for (const type of TAKEOVER_EVENTS) {
        body.addEventListener(type, release, { passive: true });
    }
    body.addEventListener('scroll', onScroll, { passive: true });
    timer = setTimeout(release, RESUME_HOLD_MS);
    return release;
}

/** One percent formatter per locale — rows format their progress on every render. */
const percentFormats = new Map<string, Intl.NumberFormat>();

/** `permille` as a percentage ("42 %") in `locale`'s own percent format. */
export function formatPercent(permille: number, locale: string): string {
    let format = percentFormats.get(locale);
    if (format === undefined) {
        format = new Intl.NumberFormat(locale, { style: 'percent' });
        percentFormats.set(locale, format);
    }
    return format.format(permille / PERMILLE_END);
}
