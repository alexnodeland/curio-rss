/**
 * The ⌘K command palette (#152): the fuzzy matcher and the ranking behind it,
 * the catalogue it builds from live state (commands only when they apply,
 * every feed, folder, settings tab and theme), and the shell wiring — Ctrl+K
 * (⌘K on macOS) opens it from anywhere, even the search field; ↑/↓ and Enter
 * run a row; Escape or a run hands focus back to where it was; axe passes.
 */
import CommandPalette from '$components/modals/CommandPalette.svelte';
import { activeView, routeMenuAction } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import {
    type PaletteItem,
    highlightRuns,
    paletteItems,
    rankPaletteItems,
} from '$lib/state/palette';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { themeStore } from '$lib/state/theme.svelte';
import { fuzzyMatch } from '$lib/utils/fuzzy';
import { isApplePlatform } from '$lib/utils/platform';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import axe from 'axe-core';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Page from '../routes/+page.svelte';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('fuzzyMatch', () => {
    it('matches the query in order, folding case and diacritics', () => {
        expect(fuzzyMatch('gts', 'Go to starred')?.indices).toEqual([0, 3, 6]);
        expect(fuzzyMatch('CAFE', 'Café notes')?.indices).toEqual([0, 1, 2, 3]);
        expect(fuzzyMatch('zürich', 'Zurich news')).not.toBeNull();
        expect(fuzzyMatch('sta', 'Go to archived')).toBeNull();
        expect(fuzzyMatch('ab', 'ba')).toBeNull();
    });

    it('matches everything with a blank query, and ignores spaces in it', () => {
        expect(fuzzyMatch('   ', 'anything')).toEqual({ score: 0, indices: [] });
        expect(fuzzyMatch('go star', 'Go to starred')).not.toBeNull();
    });

    it('scores a run at a word start above a scattered or mid-word one', () => {
        const score = (query: string, text: string) => fuzzyMatch(query, text)?.score ?? -1;
        expect(score('star', 'Go to starred')).toBeGreaterThan(score('star', 'Restart all'));
        expect(score('star', 'Starred')).toBeGreaterThan(score('star', 'Go to starred'));
        expect(score('rl', 'Read later')).toBeGreaterThan(score('rl', 'Unread lists'));
        // Prefers the word starts it can reach: R(ead) L(ater), not "re-l".
        expect(fuzzyMatch('rl', 'Read later')?.indices).toEqual([0, 5]);
        // camelCase counts as a word boundary too.
        expect(fuzzyMatch('rl', 'readLater')?.indices).toEqual([0, 4]);
    });

    it('keeps a word-start jump only when the rest of the query still fits', () => {
        // "a" could jump to "all", but then "d" (only in "read") no longer fits.
        expect(fuzzyMatch('rad', 'read all')?.indices).toEqual([0, 2, 3]);
        // With a "d" after it, the word start wins.
        expect(fuzzyMatch('rad', 'read all d')?.indices).toEqual([0, 5, 9]);
    });
});

/** A catalogue item for the ranking tests. */
function item(id: string, label: string, group: PaletteItem['group'] = 'command'): PaletteItem {
    return { id, group, label, run: () => {} };
}

describe('rankPaletteItems', () => {
    const items = [
        item('a', 'Add a feed'),
        item('b', 'Refresh all feeds'),
        item('c', 'Go to starred', 'view'),
        item('d', 'Nord', 'theme'),
        item('e', 'Restart', 'feed'),
    ];

    it('keeps the whole catalogue in order for a blank query', () => {
        expect(rankPaletteItems(items, '').map((ranked) => ranked.item.id)).toEqual([
            'a',
            'b',
            'c',
            'd',
            'e',
        ]);
        expect(rankPaletteItems(items, ' ', 2)).toHaveLength(2);
    });

    it('drops non-matches and puts the best match first', () => {
        expect(rankPaletteItems(items, 'star').map((ranked) => ranked.item.id)).toEqual(['c', 'e']);
        expect(rankPaletteItems(items, 'feed').map((ranked) => ranked.item.id)).toEqual(['a', 'b']);
        expect(rankPaletteItems(items, 'qqq')).toEqual([]);
    });

    it('narrows to a group by a leading tag word, highlighting only the label', () => {
        expect(rankPaletteItems(items, 'theme nord')).toEqual([
            { item: items[3], indices: [0, 1, 2, 3] },
        ]);
        expect(rankPaletteItems(items, 'the nord')[0].item.id).toBe('d');
        expect(rankPaletteItems(items, 'feed rest').map((ranked) => ranked.item.id)).toEqual(['e']);
        // The tag word must start the group's name, and a lone word is never a tag.
        expect(rankPaletteItems(items, 'heme nord')).toEqual([]);
        expect(rankPaletteItems(items, 'theme')).toEqual([]);
    });

    it('cuts a label into plain and matched runs', () => {
        expect(highlightRuns('Go to starred', [0, 6, 7])).toEqual([
            { text: 'G', match: true },
            { text: 'o to ', match: false },
            { text: 'st', match: true },
            { text: 'arred', match: false },
        ]);
        expect(highlightRuns('Nord', [])).toEqual([{ text: 'Nord', match: false }]);
    });
});

/** The responders the whole shell needs, with two feeds in two folders. */
function shellResponders(): Record<string, Responder> {
    return {
        list_feeds: [
            feedFixture({ id: 1, title: 'Alpha Weekly', tags: ['Tech/Databases'] }),
            feedFixture({ id: 2, title: null, url: 'https://beta.example/rss', tags: ['News'] }),
        ],
        get_unread_counts: unreadCountsFixture({ total: 2, by_feed: [[1, 2]] }),
        list_articles: [
            articleSummaryFixture({ id: 12, feed_id: 1, title: 'Twelve' }),
            articleSummaryFixture({ id: 11, feed_id: 1, title: 'Eleven' }),
        ],
        list_tags: [],
        get_article: (args) => articleFixture({ id: args.articleId as number }),
        get_article_state: articleStateFixture(),
        get_article_tags: [],
        mark_read: true,
        set_starred: true,
        set_setting: null,
        record_opened: null,
        search_articles: [],
    };
}

function palette(): HTMLElement | null {
    return document.querySelector('[data-testid="command-palette"]');
}

function field(): HTMLInputElement {
    const input = palette()?.querySelector('input[role="combobox"]');
    if (!(input instanceof HTMLInputElement)) throw new Error('no palette field');
    return input;
}

function optionLabels(): string[] {
    return [...document.querySelectorAll('[role="option"] .option-label')].map(
        (label) => label.textContent ?? '',
    );
}

function activeOption(): HTMLElement | null {
    const id = field().getAttribute('aria-activedescendant');
    return id === null ? null : document.getElementById(id);
}

async function openWithChord(target: Element | Window = window): Promise<void> {
    await fireEvent.keyDown(target, { key: 'k', ctrlKey: true });
    await tick();
    await tick();
}

describe('the palette catalogue', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        searchStore.clear();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('offers article commands only with an article selected, and feed refresh only in a feed', async () => {
        harness = installIpcHarness(shellResponders());
        void feedsStore.feeds;
        await flushIpc();
        const ids = () => paletteItems().map((entry) => entry.id);

        expect(ids()).toContain('command:app.addFeed');
        expect(ids()).not.toContain('command:article.toggleStar');
        expect(ids()).not.toContain('command:feed.refresh');
        // Pure navigation keys and the palette itself are never offered.
        expect(ids()).not.toContain('command:nav.nextArticle');
        expect(ids()).not.toContain('command:app.commandPalette');

        selectionStore.selectFeed(1);
        void articlesStore.activeItems; // the list the reader would show
        await flushIpc();
        selectionStore.selectedArticleId = 12;
        expect(ids()).toContain('command:article.toggleStar');
        expect(ids()).toContain('command:article.open');
        expect(ids()).toContain('command:feed.refresh');

        // Several rows selected: toggles yes, open / promote (one article) no.
        selectionStore.toggleSelected(11);
        expect(selectionStore.multiActive).toBe(true);
        expect(ids()).toContain('command:article.toggleStar');
        expect(ids()).not.toContain('command:article.open');
        expect(ids()).not.toContain('command:article.promote');
    });

    it('lists every view, feed, folder, settings tab and theme, marking what is current', async () => {
        harness = installIpcHarness(shellResponders());
        void feedsStore.feeds;
        await flushIpc();
        const byId = new Map(paletteItems().map((entry) => [entry.id, entry]));

        expect(byId.get('view:starred')).toMatchObject({ label: 'Go to starred', kbd: 'g s' });
        expect(byId.get('view:all')?.current).toBe(true);
        expect(byId.get('feed:1')).toMatchObject({
            label: 'Alpha Weekly',
            detail: 'Tech / Databases',
            current: false,
        });
        // An untitled feed is named by its address, as in the sidebar.
        expect(byId.get('feed:2')?.label).toBe('https://beta.example/rss');
        expect(byId.get('folder:Tech')?.label).toBe('Tech');
        expect(byId.get('folder:Tech/Databases')?.label).toBe('Tech / Databases');
        expect(byId.get('folder:News')).toBeDefined();
        expect(byId.get('settings:appearance')?.label).toBe('Appearance');
        expect(byId.get('theme:system')).toMatchObject({ label: 'System', current: true });
        expect(byId.get('theme:nord')?.current).toBe(false);
        // A descriptive built-in name is translated; a proper noun is verbatim.
        expect(byId.get('theme:dark')?.label).toBe('Dark');
        expect(byId.get('theme:nord')?.label).toBe('Nord');
        expect(byId.get('command:app.settings')?.kbd).toBe(',');

        selectionStore.selectFeed(1);
        expect(paletteItems().find((entry) => entry.id === 'feed:1')?.current).toBe(true);
        expect(paletteItems().find((entry) => entry.id === 'view:all')?.current).toBe(false);
    });

    it('runs each kind of item through the action layer', async () => {
        harness = installIpcHarness(shellResponders());
        void feedsStore.feeds;
        await flushIpc();
        const run = (id: string) =>
            paletteItems()
                .find((entry) => entry.id === id)
                ?.run();

        layoutStore.focusMode = true;
        run('view:readLater');
        expect(activeView(articlesStore.filters)).toBe('readLater');
        expect(layoutStore.focusMode).toBe(false); // the list it opened is shown

        run('feed:2');
        expect(selectionStore.selectedFeedId).toBe(2);
        run('folder:Tech');
        expect(articlesStore.filters.feedTag).toBe('Tech');
        run('settings:data');
        expect(layoutStore.activeModal).toBe('settings');
        expect(layoutStore.settingsSection).toBe('data');
        layoutStore.closeModal();
        run('theme:dracula');
        expect(themeStore.themePreference).toBe('dracula');
        run('command:app.addFeed');
        expect(layoutStore.activeModal).toBe('add-feed');
    });
});

describe('the command palette', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        searchStore.clear();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('Ctrl+K opens it with the field focused (jsdom is not a Mac)', async () => {
        expect(isApplePlatform()).toBe(false);
        harness = installIpcHarness(shellResponders());
        render(Page);
        await flushIpc();

        await openWithChord();
        expect(layoutStore.activeModal).toBe('palette');
        expect(palette()?.getAttribute('role')).toBe('dialog');
        expect(document.activeElement).toBe(field());
        expect(field().getAttribute('aria-expanded')).toBe('true');
        // The app frame behind it is inert (Svelte sets the property).
        expect((document.getElementById('main-content') as HTMLElement).inert).toBe(true);
        // Pressing the chord again keeps it open (the menu may fire it too).
        await openWithChord();
        expect(layoutStore.activeModal).toBe('palette');
    });

    it('searches as you type, moves with ↑/↓, and runs the active row on Enter', async () => {
        harness = installIpcHarness(shellResponders());
        render(Page);
        await flushIpc();
        await openWithChord();

        await fireEvent.input(field(), { target: { value: 'alpha' } });
        expect(optionLabels()[0]).toBe('Alpha Weekly');
        expect(activeOption()?.getAttribute('data-item')).toBe('feed:1');
        expect(activeOption()?.getAttribute('aria-selected')).toBe('true');
        expect(activeOption()?.querySelector('mark')?.textContent).toBe('Alpha');

        // Equal scores keep group order (commands, then views) and catalogue order.
        await fireEvent.input(field(), { target: { value: 'go to' } });
        expect(optionLabels()).toEqual([
            'Go to the feed list',
            'Go to all articles',
            'Go to starred',
            'Go to read later',
            'Go to archived',
        ]);
        // The open view is marked, and each view shows the keys that open it.
        const all = document.querySelector('[data-item="view:all"]');
        expect(all?.textContent).toContain('current');
        expect(all?.querySelector('kbd')?.textContent).toBe('g a');

        await fireEvent.keyDown(field(), { key: 'ArrowDown' });
        await fireEvent.keyDown(field(), { key: 'ArrowDown' });
        expect(activeOption()?.getAttribute('data-item')).toBe('view:starred');
        await fireEvent.keyDown(field(), { key: 'ArrowUp' });
        await fireEvent.keyDown(field(), { key: 'ArrowUp' });
        await fireEvent.keyDown(field(), { key: 'ArrowUp' }); // wraps to the last row
        expect(activeOption()?.getAttribute('data-item')).toBe('view:archived');
        await fireEvent.keyDown(field(), { key: 'ArrowDown' }); // and back to the first
        expect(activeOption()?.getAttribute('data-item')).toBe('command:view.feeds');
        await fireEvent.keyDown(field(), { key: 'PageDown' }); // clamps at the end
        expect(activeOption()?.getAttribute('data-item')).toBe('view:archived');
        await fireEvent.keyDown(field(), { key: 'PageUp' });
        await fireEvent.keyDown(field(), { key: 'ArrowDown' });
        await fireEvent.keyDown(field(), { key: 'ArrowDown' });
        // Hovering a row makes it the active one.
        await fireEvent.pointerMove(
            document.querySelector('[data-item="view:starred"]') as Element,
        );
        expect(activeOption()?.getAttribute('data-item')).toBe('view:starred');
        await fireEvent.keyDown(field(), { key: 'Enter' });
        await flushIpc();

        expect(palette()).toBeNull();
        expect(activeView(articlesStore.filters)).toBe('starred');
    });

    it('says so when nothing matches, and counts the matches for a screen reader', async () => {
        harness = installIpcHarness(shellResponders());
        render(Page);
        await flushIpc();
        await openWithChord();

        await fireEvent.input(field(), { target: { value: 'nord' } });
        const live = palette()?.querySelector('[aria-live="polite"]');
        expect(live?.textContent).toBe('1 match');
        await fireEvent.input(field(), { target: { value: 'zzzzqq' } });
        expect(palette()?.querySelector('[role="listbox"]')).toBeNull();
        expect(palette()?.textContent).toContain('No matches');
        expect(field().getAttribute('aria-expanded')).toBe('false');
        expect(field().hasAttribute('aria-activedescendant')).toBe(false);
        expect(live?.textContent).toBe('0 matches');
    });

    it('Escape closes it and focus goes back where it was', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = render(Page);
        await flushIpc();
        const listbox = container.querySelector('[role="listbox"]') as HTMLElement;
        listbox.focus();

        await openWithChord();
        expect(document.activeElement).toBe(field());
        await fireEvent.keyDown(field(), { key: 'Escape' });
        await tick();
        expect(palette()).toBeNull();
        expect(document.activeElement).toBe(listbox);
    });

    it('opens from the search field and, run there, leaves focus with the action', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = render(Page);
        await flushIpc();
        const search = container.querySelector('input[type="search"]') as HTMLInputElement;
        search.focus();

        // The typing guard would swallow a plain key here; the chord gets through.
        await openWithChord(search);
        expect(layoutStore.activeModal).toBe('palette');
        expect(document.activeElement).toBe(field());

        // A click runs a row too; the theme switch keeps focus where it was.
        await fireEvent.input(field(), { target: { value: 'theme dracula' } });
        const option = document.querySelector('[data-item="theme:dracula"]') as HTMLElement;
        await fireEvent.click(option);
        await flushIpc();
        expect(palette()).toBeNull();
        expect(themeStore.themePreference).toBe('dracula');
        expect(document.activeElement).toBe(search);
        expect(harness.callsFor('set_setting')).toContainEqual({
            key: 'ui.theme',
            value: 'dracula',
        });
    });

    it('opens from the native View menu item too', async () => {
        harness = installIpcHarness(shellResponders());
        render(Page);
        await flushIpc();
        routeMenuAction('app.commandPalette');
        await tick();
        expect(palette()).not.toBeNull();
    });

    it('never replaces another open dialog', async () => {
        harness = installIpcHarness(shellResponders());
        render(Page);
        await flushIpc();
        layoutStore.openModal('help');
        await tick();

        await openWithChord();
        expect(layoutStore.activeModal).toBe('help');
    });

    it('an open palette passes axe', async () => {
        harness = installIpcHarness(shellResponders());
        render(CommandPalette, { props: { onclose: vi.fn() } });
        await flushIpc();
        await fireEvent.input(field(), { target: { value: 'go' } });

        const results = await axe.run(palette() as Element, {
            rules: { 'color-contrast': { enabled: false }, region: { enabled: false } },
        });
        expect(results.violations.map((violation) => violation.id)).toEqual([]);
    });
});
