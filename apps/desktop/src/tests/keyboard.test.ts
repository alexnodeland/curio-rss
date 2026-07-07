/**
 * Keyboard registry: the data table covers the whole v1 map, and the pure
 * matcher state machine handles singles, Shift-cased keys, g-chords with a
 * timeout, and typing contexts.
 */
import {
    CHORD_TIMEOUT_MS,
    SHORTCUTS,
    type ShortcutId,
    createMatcher,
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
            'article.open': ['o'],
            'article.toggleStar': ['s'],
            'article.toggleReadLater': ['l'],
            'article.toggleRead': ['m'],
            'article.promote': ['p'],
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
});
