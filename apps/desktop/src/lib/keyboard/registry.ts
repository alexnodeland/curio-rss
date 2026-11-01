/**
 * The keyboard shortcut registry — shortcuts are DATA (a table the help
 * overlay renders verbatim), and matching is a small pure state machine, not
 * an if-chain. Keys are {@link eventKeyName} values: `KeyboardEvent.key`,
 * except that Space is `'Space'` and a shifted named key carries a `Shift+`
 * prefix (`'Shift+Space'`) — a shifted printable key is already its own
 * character, so `Shift+R` is just `'R'` and `?` is `'?'` (a letter's case
 * is read from Shift, so Caps Lock never turns `r` into `R`). Ctrl/alt/meta
 * chords are deliberately absent from the v1 map (they belong to the native
 * menu). Two-key g-chords (`g` then `a`) ride a pending state with a timeout.
 *
 * Global keys go through the shell's window matcher. Pane keys
 * ({@link ShortcutDef.paneKeys}) are the ARIA-pattern keys — the listbox's
 * arrows and Enter, the reader's ← and Esc — bound only while that pane owns
 * the keyboard, so Enter never hijacks a focused button. Both live in this
 * one table so the help overlay and the empty-state hint can never advertise
 * a key that does nothing. Handlers bind elsewhere (the shell wires ids to
 * actions) — the registry knows nothing about what a shortcut does.
 */
import type { MessageKey } from '$lib/i18n';

/** Stable action ids — the currency between the registry and the shell. */
export type ShortcutId =
    | 'nav.nextArticle'
    | 'nav.previousArticle'
    | 'nav.nextUnread'
    | 'nav.focusReader'
    | 'nav.focusList'
    | 'reader.pageDown'
    | 'reader.pageUp'
    | 'article.open'
    | 'article.toggleStar'
    | 'article.toggleReadLater'
    | 'article.toggleRead'
    | 'article.toggleArchived'
    | 'article.promote'
    | 'view.markAllRead'
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

/** A pane that binds its own keys: the article listbox or the reader body. */
export type PaneScope = 'list' | 'reader';

export interface ShortcutDef {
    /** The action this shortcut triggers. */
    readonly id: ShortcutId;
    /**
     * The global key sequence as {@link eventKeyName} values (length 1 or 2),
     * matched by the shell from anywhere outside a typing context. Empty for
     * an action that only exists inside a pane (see `paneKeys`).
     */
    readonly keys: readonly string[];
    /**
     * Single keys bound only while `scope`'s pane owns the keyboard — the
     * ARIA listbox arrows / Enter, the reader body's ← / Esc. The pane's own
     * keydown handler resolves them through {@link paneShortcut}; they never
     * reach the global matcher.
     */
    readonly paneKeys?: { readonly scope: PaneScope; readonly keys: readonly string[] };
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
        paneKeys: { scope: 'list', keys: ['ArrowDown'] },
        description: 'shortcut.nextArticle',
        category: 'navigation',
    },
    {
        id: 'nav.previousArticle',
        keys: ['k'],
        paneKeys: { scope: 'list', keys: ['ArrowUp'] },
        description: 'shortcut.previousArticle',
        category: 'navigation',
    },
    {
        id: 'nav.nextUnread',
        keys: ['n'],
        description: 'shortcut.nextUnread',
        category: 'navigation',
    },
    {
        id: 'nav.focusReader',
        keys: [],
        paneKeys: { scope: 'list', keys: ['Enter', 'ArrowRight'] },
        description: 'shortcut.focusReader',
        category: 'navigation',
    },
    {
        id: 'nav.focusList',
        keys: [],
        paneKeys: { scope: 'reader', keys: ['ArrowLeft', 'Escape'] },
        description: 'shortcut.focusList',
        category: 'navigation',
    },
    {
        id: 'reader.pageDown',
        keys: ['Space'],
        description: 'shortcut.pageDown',
        category: 'navigation',
    },
    {
        id: 'reader.pageUp',
        keys: ['Shift+Space'],
        description: 'shortcut.pageUp',
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
    {
        id: 'article.toggleArchived',
        keys: ['e'],
        description: 'shortcut.toggleArchived',
        category: 'actions',
    },
    { id: 'article.promote', keys: ['p'], description: 'shortcut.promote', category: 'actions' },
    {
        id: 'view.markAllRead',
        keys: ['A'],
        description: 'shortcut.markAllRead',
        category: 'actions',
    },
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
        if (shortcut.keys.length === 0) {
            continue; // pane-only: resolved by the pane, never globally
        }
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
 * Whether `key` is a single cased letter — the keys whose `KeyboardEvent.key`
 * Caps Lock flips without Shift being held.
 */
function isCasedLetter(key: string): boolean {
    return key.length === 1 && key.toLowerCase() !== key.toUpperCase();
}

/**
 * The registry's name for a keydown: `KeyboardEvent.key`, with Space spelled
 * `'Space'` and a `Shift+` prefix on shifted *named* keys (`'Shift+Space'`,
 * `'Shift+ArrowDown'`). Shifted printable keys are already distinct
 * characters (`'A'`, `'?'`), so they carry no prefix. A letter's case comes
 * from Shift alone, never Caps Lock: `'A'` always means Shift+a, so a plain
 * `a` with Caps Lock on stays Add feed instead of firing the bulk
 * mark-view-read bound to `A`.
 */
export function eventKeyName(event: KeyboardEvent): string {
    if (isCasedLetter(event.key)) {
        return event.shiftKey ? event.key.toUpperCase() : event.key.toLowerCase();
    }
    const name = event.key === ' ' ? 'Space' : event.key;
    const named = name.length > 1 && name !== 'Shift';
    return event.shiftKey && named ? `Shift+${name}` : name;
}

/** `scope`'s pane keys → action ids, built once from the table. */
function paneMap(scope: PaneScope): ReadonlyMap<string, ShortcutId> {
    const map = new Map<string, ShortcutId>();
    for (const shortcut of SHORTCUTS) {
        if (shortcut.paneKeys?.scope === scope) {
            for (const key of shortcut.paneKeys.keys) {
                map.set(key, shortcut.id);
            }
        }
    }
    return map;
}

const PANE_KEYS: Record<PaneScope, ReadonlyMap<string, ShortcutId>> = {
    list: paneMap('list'),
    reader: paneMap('reader'),
};

/**
 * Resolves a pane-local key (an {@link eventKeyName} value) to its action, or
 * `null` when the pane doesn't bind it.
 */
export function paneShortcut(scope: PaneScope, key: string): ShortcutId | null {
    return PANE_KEYS[scope].get(key) ?? null;
}

/**
 * Roles whose Space activation belongs to the control, not to us. Listbox
 * options are absent on purpose: Space over a focused article row pages
 * the reader (options select on focus, they don't activate).
 */
const ACTIVATABLE_ROLES = new Set([
    'button',
    'checkbox',
    'link',
    'menuitem',
    'menuitemcheckbox',
    'menuitemradio',
    'radio',
    'switch',
    'tab',
]);

/**
 * Whether `target` activates on Space itself (a button, link, checkbox…), so
 * a global Space shortcut must not swallow the press.
 */
function isActivatable(target: HTMLElement): boolean {
    if (
        target instanceof HTMLButtonElement ||
        target instanceof HTMLAnchorElement ||
        target instanceof HTMLDetailsElement ||
        target.tagName === 'SUMMARY'
    ) {
        return true;
    }
    return ACTIVATABLE_ROLES.has(target.getAttribute('role') ?? '');
}

/**
 * Whether a keydown event should bypass shortcuts entirely: typing contexts
 * (inputs, textareas, contenteditable), any ctrl/alt/meta combination, and
 * Space on a control that Space activates all belong to the platform, not
 * to us.
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
            target.isContentEditable === true ||
            (event.key === ' ' && isActivatable(target))
        );
    }
    return false;
}
