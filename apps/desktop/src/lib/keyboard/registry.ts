/**
 * The keyboard shortcut registry — shortcuts are DATA (a table the help
 * overlay renders verbatim), and matching is a small pure state machine, not
 * an if-chain. Keys are `KeyboardEvent.key` values, so `Shift+R` is just
 * `'R'` and `?` is `'?'`; ctrl/alt/meta chords are deliberately absent from
 * the v1 map. Two-key g-chords (`g` then `a`) ride a pending state with a
 * timeout. Handlers bind elsewhere (the shell wires ids to actions) — the
 * registry knows nothing about what a shortcut does.
 */
import type { MessageKey } from '$lib/i18n';

/** Stable action ids — the currency between the registry and the shell. */
export type ShortcutId =
    | 'nav.nextArticle'
    | 'nav.previousArticle'
    | 'article.open'
    | 'article.toggleStar'
    | 'article.toggleReadLater'
    | 'article.toggleRead'
    | 'article.promote'
    | 'feed.refresh'
    | 'app.refreshAll'
    | 'search.focus'
    | 'view.all'
    | 'view.starred'
    | 'view.readLater'
    | 'view.archived'
    | 'view.feeds'
    | 'app.addFeed'
    | 'app.settings'
    | 'help.toggle';

export type ShortcutCategory = 'navigation' | 'actions' | 'views' | 'app';

export interface ShortcutDef {
    /** The action this shortcut triggers. */
    readonly id: ShortcutId;
    /** The key sequence as `KeyboardEvent.key` values (length 1 or 2). */
    readonly keys: readonly string[];
    /** i18n key for the help overlay's description column. */
    readonly description: MessageKey;
    /** Help-overlay grouping. */
    readonly category: ShortcutCategory;
}

/** The full v1 keyboard map. Order here is display order in the help overlay. */
export const SHORTCUTS: readonly ShortcutDef[] = [
    {
        id: 'nav.nextArticle',
        keys: ['j'],
        description: 'shortcut.nextArticle',
        category: 'navigation',
    },
    {
        id: 'nav.previousArticle',
        keys: ['k'],
        description: 'shortcut.previousArticle',
        category: 'navigation',
    },
    { id: 'article.open', keys: ['o'], description: 'shortcut.openArticle', category: 'actions' },
    {
        id: 'article.toggleStar',
        keys: ['s'],
        description: 'shortcut.toggleStar',
        category: 'actions',
    },
    {
        id: 'article.toggleReadLater',
        keys: ['l'],
        description: 'shortcut.toggleReadLater',
        category: 'actions',
    },
    {
        id: 'article.toggleRead',
        keys: ['m'],
        description: 'shortcut.toggleRead',
        category: 'actions',
    },
    { id: 'article.promote', keys: ['p'], description: 'shortcut.promote', category: 'actions' },
    { id: 'feed.refresh', keys: ['r'], description: 'shortcut.refreshFeed', category: 'actions' },
    { id: 'app.refreshAll', keys: ['R'], description: 'shortcut.refreshAll', category: 'app' },
    { id: 'search.focus', keys: ['/'], description: 'shortcut.search', category: 'app' },
    { id: 'view.all', keys: ['g', 'a'], description: 'shortcut.viewAll', category: 'views' },
    {
        id: 'view.starred',
        keys: ['g', 's'],
        description: 'shortcut.viewStarred',
        category: 'views',
    },
    {
        id: 'view.readLater',
        keys: ['g', 'l'],
        description: 'shortcut.viewReadLater',
        category: 'views',
    },
    {
        id: 'view.archived',
        keys: ['g', 'e'],
        description: 'shortcut.viewArchived',
        category: 'views',
    },
    { id: 'view.feeds', keys: ['g', 'f'], description: 'shortcut.viewFeeds', category: 'views' },
    { id: 'app.addFeed', keys: ['a'], description: 'shortcut.addFeed', category: 'app' },
    { id: 'app.settings', keys: [','], description: 'shortcut.settings', category: 'app' },
    { id: 'help.toggle', keys: ['?'], description: 'shortcut.help', category: 'app' },
] as const;

/** What a keystroke resolved to. */
export type MatchResult =
    | { kind: 'match'; id: ShortcutId }
    | { kind: 'pending'; prefix: string }
    | { kind: 'none' };

/** How long a chord prefix stays armed. */
export const CHORD_TIMEOUT_MS = 1000;

export interface Matcher {
    /**
     * Feeds one `KeyboardEvent.key` value into the machine. `now` is a
     * millisecond clock reading (injectable for tests).
     */
    handle(key: string, now?: number): MatchResult;
    /** Drops any armed chord prefix (e.g. on focus loss). */
    reset(): void;
}

/**
 * Builds the matching state machine for a shortcut table. Single-key
 * shortcuts match immediately; chord prefixes arm a pending state that
 * expires after `timeoutMs`.
 */
export function createMatcher(
    shortcuts: readonly ShortcutDef[] = SHORTCUTS,
    timeoutMs: number = CHORD_TIMEOUT_MS,
): Matcher {
    const singles = new Map<string, ShortcutId>();
    const chords = new Map<string, ShortcutId>();
    const prefixes = new Set<string>();

    for (const shortcut of shortcuts) {
        if (shortcut.keys.length === 1) {
            singles.set(shortcut.keys[0], shortcut.id);
        } else {
            prefixes.add(shortcut.keys[0]);
            chords.set(shortcut.keys.join(' '), shortcut.id);
        }
    }

    let pendingPrefix: string | null = null;
    let pendingSince = 0;

    // A dead chord swallows the key — falling through would make `g s`
    // star an article whenever the chord table changed.
    function resolveChord(prefix: string, key: string): MatchResult {
        pendingPrefix = null;
        const chordId = chords.get(`${prefix} ${key}`);
        return chordId === undefined ? { kind: 'none' } : { kind: 'match', id: chordId };
    }

    return {
        handle(key: string, now: number = Date.now()): MatchResult {
            if (pendingPrefix !== null && now - pendingSince > timeoutMs) {
                pendingPrefix = null;
            }
            if (pendingPrefix !== null) {
                return resolveChord(pendingPrefix, key);
            }
            if (prefixes.has(key)) {
                pendingPrefix = key;
                pendingSince = now;
                return { kind: 'pending', prefix: key };
            }
            const singleId = singles.get(key);
            return singleId === undefined ? { kind: 'none' } : { kind: 'match', id: singleId };
        },
        reset(): void {
            pendingPrefix = null;
        },
    };
}

/**
 * Whether a keydown event should bypass shortcuts entirely: typing contexts
 * (inputs, textareas, contenteditable) and any ctrl/alt/meta combination
 * belong to the platform, not to us.
 */
export function shouldIgnoreKeyEvent(event: KeyboardEvent): boolean {
    if (event.ctrlKey || event.altKey || event.metaKey) {
        return true;
    }
    const target = event.target;
    if (target instanceof HTMLElement) {
        return (
            target instanceof HTMLInputElement ||
            target instanceof HTMLTextAreaElement ||
            target instanceof HTMLSelectElement ||
            // Coerced: jsdom leaves isContentEditable undefined.
            target.isContentEditable === true
        );
    }
    return false;
}
