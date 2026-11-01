/**
 * Keyboard registry: the data table covers the whole v1 map, and the pure
 * matcher state machine handles singles, Shift-cased keys, g-chords with a
 * timeout, and typing contexts.
 */
import { shortcutKbd } from '$lib/keyboard/labels';
import {
    CHORD_TIMEOUT_MS,
    SHORTCUTS,
    type ShortcutId,
    createMatcher,
    eventKeyName,
    paneShortcut,
    shouldIgnoreKeyEvent,
} from '$lib/keyboard/registry';
import { describe, expect, it } from 'vitest';

describe('shortcut table', () => {
    it('is data covering the full v1 keyboard map', () => {
        const byId = new Map(SHORTCUTS.map((shortcut) => [shortcut.id, shortcut.keys]));
        const expected: Record<ShortcutId, readonly string[]> = {
            'nav.nextArticle': ['j'],
            'nav.previousArticle': ['k'],
            'nav.nextUnread': ['n'],
            'nav.focusReader': [],
            'nav.focusList': [],
            'reader.pageDown': ['Space'],
            'reader.pageUp': ['Shift+Space'],
            'article.open': ['o'],
            'article.toggleStar': ['s'],
            'article.toggleReadLater': ['l'],
            'article.toggleRead': ['m'],
            'article.toggleArchived': ['e'],
            'article.promote': ['p'],
            'view.markAllRead': ['A'],
            'feed.refresh': ['r'],
            'app.refreshAll': ['R'],
            'search.focus': ['/'],
            'view.all': ['g', 'a'],
            'view.starred': ['g', 's'],
            'view.readLater': ['g', 'l'],
            'view.archived': ['g', 'e'],
            'view.feeds': ['g', 'f'],
            'app.addFeed': ['a'],
            'app.settings': [','],
            'help.toggle': ['?'],
        };
        expect(Object.fromEntries(byId)).toEqual(expected);
    });

    it('binds the ARIA pane keys: listbox arrows / Enter, reader ← / Esc', () => {
        const pane = Object.fromEntries(
            SHORTCUTS.filter((shortcut) => shortcut.paneKeys !== undefined).map((shortcut) => [
                shortcut.id,
                shortcut.paneKeys,
            ]),
        );
        expect(pane).toEqual({
            'nav.nextArticle': { scope: 'list', keys: ['ArrowDown'] },
            'nav.previousArticle': { scope: 'list', keys: ['ArrowUp'] },
            'nav.focusReader': { scope: 'list', keys: ['Enter', 'ArrowRight'] },
            'nav.focusList': { scope: 'reader', keys: ['ArrowLeft', 'Escape'] },
        });
        expect(paneShortcut('list', 'ArrowDown')).toBe('nav.nextArticle');
        expect(paneShortcut('list', 'ArrowUp')).toBe('nav.previousArticle');
        expect(paneShortcut('list', 'Enter')).toBe('nav.focusReader');
        expect(paneShortcut('list', 'ArrowRight')).toBe('nav.focusReader');
        expect(paneShortcut('reader', 'ArrowLeft')).toBe('nav.focusList');
        expect(paneShortcut('reader', 'Escape')).toBe('nav.focusList');
        // Scoped: a list key means nothing in the reader, and vice versa.
        expect(paneShortcut('reader', 'ArrowDown')).toBeNull();
        expect(paneShortcut('list', 'ArrowLeft')).toBeNull();
        expect(paneShortcut('list', 'j')).toBeNull();
    });

    it('every action is reachable: a global sequence, pane keys, or both', () => {
        for (const shortcut of SHORTCUTS) {
            const global = shortcut.keys.length;
            const pane = shortcut.paneKeys?.keys.length ?? 0;
            expect(global <= 2, shortcut.id).toBe(true);
            expect(global + pane > 0, shortcut.id).toBe(true);
        }
    });

    it('gives every shortcut a description key and a category', () => {
        for (const shortcut of SHORTCUTS) {
            expect(shortcut.description.length).toBeGreaterThan(0);
            expect(['navigation', 'actions', 'views', 'app']).toContain(shortcut.category);
        }
    });
});

describe('matcher', () => {
    it('matches every single-key shortcut directly', () => {
        for (const shortcut of SHORTCUTS.filter((s) => s.keys.length === 1)) {
            const matcher = createMatcher();
            expect(matcher.handle(shortcut.keys[0], 0)).toEqual({
                kind: 'match',
                id: shortcut.id,
            });
        }
    });

    it('distinguishes r from Shift+R by KeyboardEvent.key casing', () => {
        const matcher = createMatcher();
        expect(matcher.handle('r', 0)).toEqual({ kind: 'match', id: 'feed.refresh' });
        expect(matcher.handle('R', 0)).toEqual({ kind: 'match', id: 'app.refreshAll' });
    });

    it('resolves g-chords through a pending state', () => {
        const matcher = createMatcher();
        expect(matcher.handle('g', 0)).toEqual({ kind: 'pending', prefix: 'g' });
        expect(matcher.handle('a', 10)).toEqual({ kind: 'match', id: 'view.all' });

        matcher.handle('g', 20);
        expect(matcher.handle('s', 30)).toEqual({ kind: 'match', id: 'view.starred' });
        matcher.handle('g', 40);
        expect(matcher.handle('l', 50)).toEqual({ kind: 'match', id: 'view.readLater' });
        matcher.handle('g', 60);
        expect(matcher.handle('f', 70)).toEqual({ kind: 'match', id: 'view.feeds' });
    });

    it('a dead chord swallows the second key instead of firing its single', () => {
        const matcher = createMatcher();
        matcher.handle('g', 0);
        // `g x` is nothing — and crucially `g s` must not star an article,
        // so a chord miss never falls through to the single-key table.
        expect(matcher.handle('x', 10)).toEqual({ kind: 'none' });
        expect(matcher.handle('s', 20)).toEqual({ kind: 'match', id: 'article.toggleStar' });
    });

    it('expires the chord prefix after the timeout', () => {
        const matcher = createMatcher();
        matcher.handle('g', 0);
        // The chord expired, so `a` no longer completes `g a` (view.all) — it
        // falls through to its own single-key binding (add feed).
        expect(matcher.handle('a', CHORD_TIMEOUT_MS + 1)).toEqual({
            kind: 'match',
            id: 'app.addFeed',
        });
        // At exactly the timeout boundary the chord still fires.
        matcher.handle('g', 5000);
        expect(matcher.handle('a', 5000 + CHORD_TIMEOUT_MS)).toEqual({
            kind: 'match',
            id: 'view.all',
        });
    });

    it('reset drops an armed prefix', () => {
        const matcher = createMatcher();
        matcher.handle('g', 0);
        matcher.reset();
        // Prefix dropped: `a` is its own single-key shortcut, not `g a`.
        expect(matcher.handle('a', 1)).toEqual({ kind: 'match', id: 'app.addFeed' });
    });

    it('unknown keys are none', () => {
        const matcher = createMatcher();
        expect(matcher.handle('z', 0)).toEqual({ kind: 'none' });
        expect(matcher.handle('Escape', 0)).toEqual({ kind: 'none' });
    });
});

describe('eventKeyName', () => {
    const named = (init: KeyboardEventInit) => eventKeyName(new KeyboardEvent('keydown', init));

    it('spells Space out and prefixes shifted named keys', () => {
        expect(named({ key: ' ' })).toBe('Space');
        expect(named({ key: ' ', shiftKey: true })).toBe('Shift+Space');
        expect(named({ key: 'ArrowDown', shiftKey: true })).toBe('Shift+ArrowDown');
        expect(named({ key: 'Enter' })).toBe('Enter');
    });

    it('leaves printable keys as their own (already shifted) character', () => {
        expect(named({ key: 'A', shiftKey: true })).toBe('A');
        expect(named({ key: '?', shiftKey: true })).toBe('?');
        expect(named({ key: 'Shift', shiftKey: true })).toBe('Shift');
    });

    it('reads a letter’s case from Shift, never Caps Lock', () => {
        // Caps Lock on, no Shift: the key reports 'A', but it is a plain a.
        expect(named({ key: 'A' })).toBe('a');
        expect(named({ key: 'R' })).toBe('r');
        // Caps Lock on + Shift may report 'a'; Shift still makes it A.
        expect(named({ key: 'a', shiftKey: true })).toBe('A');
        expect(named({ key: 'é', shiftKey: true })).toBe('É');
        // Uncased characters are left alone.
        expect(named({ key: '1', shiftKey: true })).toBe('1');
        expect(named({ key: ',' })).toBe(',');
    });
});

describe('shortcutKbd', () => {
    it('prints the global key the registry binds, as the help overlay does', () => {
        // Add feed is a plain a; A (Shift+a) is the bulk mark-view-read.
        expect(shortcutKbd('app.addFeed')).toBe('a');
        expect(shortcutKbd('view.markAllRead')).toBe('A');
        expect(shortcutKbd('app.refreshAll')).toBe('R');
        expect(shortcutKbd('article.toggleStar')).toBe('s');
        expect(shortcutKbd('view.all')).toBe('g a');
    });

    it('is undefined for a pane-only action (no tooltip suffix)', () => {
        expect(shortcutKbd('nav.focusReader')).toBeUndefined();
    });
});

describe('shouldIgnoreKeyEvent', () => {
    function eventOn(target: EventTarget, init: KeyboardEventInit = {}): KeyboardEvent {
        const captured: { event: KeyboardEvent | null } = { event: null };
        const listener = (event: Event): void => {
            captured.event = event as KeyboardEvent;
        };
        target.addEventListener('keydown', listener);
        target.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', bubbles: true, ...init }));
        target.removeEventListener('keydown', listener);
        if (captured.event === null) {
            throw new Error('keydown never dispatched');
        }
        return captured.event;
    }

    it('ignores typing contexts', () => {
        const input = document.createElement('input');
        const textarea = document.createElement('textarea');
        document.body.append(input, textarea);

        expect(shouldIgnoreKeyEvent(eventOn(input))).toBe(true);
        expect(shouldIgnoreKeyEvent(eventOn(textarea))).toBe(true);

        input.remove();
        textarea.remove();
    });

    it('ignores ctrl/alt/meta combinations, passes plain keys', () => {
        const div = document.createElement('div');
        document.body.append(div);

        expect(shouldIgnoreKeyEvent(eventOn(div, { ctrlKey: true }))).toBe(true);
        expect(shouldIgnoreKeyEvent(eventOn(div, { altKey: true }))).toBe(true);
        expect(shouldIgnoreKeyEvent(eventOn(div, { metaKey: true }))).toBe(true);
        expect(shouldIgnoreKeyEvent(eventOn(div))).toBe(false);
        expect(shouldIgnoreKeyEvent(eventOn(div, { shiftKey: true }))).toBe(false);

        div.remove();
    });

    it('leaves Space to a control Space activates, but not to a listbox row', () => {
        const button = document.createElement('button');
        const toggle = document.createElement('div');
        toggle.setAttribute('role', 'switch');
        const option = document.createElement('div');
        option.setAttribute('role', 'option');
        document.body.append(button, toggle, option);

        expect(shouldIgnoreKeyEvent(eventOn(button, { key: ' ' }))).toBe(true);
        expect(shouldIgnoreKeyEvent(eventOn(toggle, { key: ' ' }))).toBe(true);
        // Space over a focused article row pages the reader.
        expect(shouldIgnoreKeyEvent(eventOn(option, { key: ' ' }))).toBe(false);
        // Only Space: other shortcuts still fire over a focused button.
        expect(shouldIgnoreKeyEvent(eventOn(button, { key: 'j' }))).toBe(false);

        button.remove();
        toggle.remove();
        option.remove();
    });
});
