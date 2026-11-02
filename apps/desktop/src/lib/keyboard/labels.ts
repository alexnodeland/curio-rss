/**
 * Display labels for registry key names — the one place the help overlay and
 * the reader's empty-state hint turn `'ArrowDown'` into `↓` and `'Space'`
 * into the localized key name, so both surfaces print the same glyphs for
 * the same binding.
 */
import { t } from '$lib/i18n';
import { isApplePlatform } from '$lib/utils/platform';
import { SHORTCUTS, type ShortcutId } from './registry';

const SYMBOLS: Readonly<Record<string, string>> = {
    ArrowUp: '↑',
    ArrowDown: '↓',
    ArrowLeft: '←',
    ArrowRight: '→',
    Escape: 'Esc',
};

/** The printed label for one {@link eventKeyName} value. */
export function keyLabel(key: string): string {
    if (key.startsWith('Shift+')) {
        return `⇧ ${keyLabel(key.slice('Shift+'.length))}`;
    }
    if (key === 'Space') {
        return t('key.space');
    }
    return SYMBOLS[key] ?? key;
}

/**
 * The printed label for a command-modifier chord on `key`
 * (`ShortcutDef.commandKey`): `⌘K` on macOS, `Ctrl+K` elsewhere.
 */
export function commandKeyLabel(key: string, apple: boolean = isApplePlatform()): string {
    const letter = key.toUpperCase();
    return apple ? `⌘${letter}` : `${t('key.ctrl')}+${letter}`;
}

/**
 * The tooltip `kbd` suffix for `id`: its global key sequence as the help
 * overlay prints it (`'a'`, `'R'`, `'g a'`, `'⌘K'`), so a control's tooltip
 * can never name a different key than the one the registry binds.
 * `undefined` for an id with no global keys (pane-only actions), which
 * renders no suffix.
 */
export function shortcutKbd(id: ShortcutId): string | undefined {
    const shortcut = SHORTCUTS.find((candidate) => candidate.id === id);
    if (shortcut?.commandKey !== undefined) {
        return commandKeyLabel(shortcut.commandKey);
    }
    const keys = shortcut?.keys ?? [];
    return keys.length === 0 ? undefined : keys.map(keyLabel).join(' ');
}

/**
 * The keys a hint advertises for `id`: its pane keys when it has them (the
 * reader's empty-state hint shows while the list owns the keyboard, where
 * the arrows and Enter are live), else its global sequence. Always keys the
 * registry actually binds — an unknown id yields none.
 */
export function advertisedKeys(id: ShortcutId): readonly string[] {
    const shortcut = SHORTCUTS.find((candidate) => candidate.id === id);
    if (shortcut === undefined) {
        return [];
    }
    return shortcut.paneKeys?.keys ?? shortcut.keys;
}
