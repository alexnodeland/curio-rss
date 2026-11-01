/**
 * Display labels for registry key names — the one place the help overlay and
 * the reader's empty-state hint turn `'ArrowDown'` into `↓` and `'Space'`
 * into the localized key name, so both surfaces print the same glyphs for
 * the same binding.
 */
import { t } from '$lib/i18n';
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
