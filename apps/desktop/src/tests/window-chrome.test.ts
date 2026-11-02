/**
 * Native window chrome (#148): under the macOS overlay title bar the three
 * pane headers are the window's drag regions while their controls stay
 * clickable, the leftmost header clears the traffic lights through the
 * `--titlebar-inset` token (`<html data-titlebar="overlay">`), and
 * fullscreen drops the inset. With native decorations (Linux, Windows)
 * nothing is marked.
 *
 * jsdom runs no Tauri drag script, so `startsDrag` restates the decision
 * Tauri's injected `drag.js` makes on mousedown (tauri 2.12,
 * `src/window/scripts/drag.js`, `isDragRegion`): walk the event path from
 * the target up; a control without the attribute blocks, `false` blocks,
 * `deep` drags from anywhere below it, a bare/`true` value only from itself.
 */
import ListPane from '$components/layout/ListPane.svelte';
import RecoveryScreen from '$components/layout/RecoveryScreen.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { events } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { connectivityStore } from '$lib/state/connectivity.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import {
    DRAG_REGION_ATTR,
    WindowChromeStore,
    windowChromeStore,
} from '$lib/state/window-chrome.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
    startupFailureFixture,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const CLICKABLE_TAGS = new Set(['A', 'BUTTON', 'INPUT', 'SELECT', 'TEXTAREA', 'LABEL', 'SUMMARY']);
const INTERACTIVE_ROLES = new Set([
    'button',
    'link',
    'menuitem',
    'tab',
    'checkbox',
    'radio',
    'switch',
    'option',
]);

function isClickable(element: HTMLElement): boolean {
    return (
        CLICKABLE_TAGS.has(element.tagName) ||
        (element.hasAttribute('contenteditable') &&
            element.getAttribute('contenteditable') !== 'false') ||
        (element.hasAttribute('tabindex') && element.getAttribute('tabindex') !== '-1') ||
        INTERACTIVE_ROLES.has(element.getAttribute('role') ?? '')
    );
}

/** One step of the walk: drag, block, or keep walking up (`null`). */
function verdict(node: HTMLElement, target: Element): boolean | null {
    const attr = node.getAttribute(DRAG_REGION_ATTR);
    if (attr === null) {
        return isClickable(node) ? false : null;
    }
    if (attr === 'deep') return true;
    if (attr === '' || attr === 'true') return node === target;
    return false;
}

/** Whether a mousedown on `target` starts a window drag (see the header). */
function startsDrag(target: Element): boolean {
    for (let node: Element | null = target; node !== null; node = node.parentElement) {
        const decided = node instanceof HTMLElement ? verdict(node, target) : null;
        if (decided !== null) return decided;
    }
    return false;
}

function emulateOverlay(fullscreen = false): void {
    windowChromeStore.overlay = true;
    windowChromeStore.fullscreen = fullscreen;
}

function shellResponders(): Record<string, Responder> {
    return {
        list_articles: [],
        list_feeds: [],
        search_articles: [],
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        get_article: articleFixture({ id: 100, title: 'The article' }),
        get_article_state: articleStateFixture(),
        mark_read: true,
    };
}

function query(container: HTMLElement, selector: string): HTMLElement {
    const element = container.querySelector<HTMLElement>(selector);
    if (element === null) throw new Error(`no ${selector}`);
    return element;
}

describe('window chrome', () => {
    let harness: IpcHarness | null = null;
    const teardowns: Array<() => void> = [];

    afterEach(() => {
        cleanup();
        for (const teardown of teardowns.splice(0)) teardown();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        searchStore.reset();
        feedsStore.reset();
        connectivityStore.reset();
        startupStore.reset();
        windowChromeStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    describe('store', () => {
        it('turns the inset on for an overlay title bar and off in fullscreen', async () => {
            harness = installIpcHarness({
                window_chrome: { overlay_title_bar: true, fullscreen: false },
            });
            const store = new WindowChromeStore();
            teardowns.push(await store.wire());
            expect(store.inset).toBe(true);
            expect(store.dragRegion).toBe('deep');
            expect(document.documentElement.dataset.titlebar).toBe('overlay');

            await events.fullscreenChanged.emit({ fullscreen: true });
            expect(store.inset).toBe(false);
            expect(store.dragRegion).toBeUndefined();
            expect(document.documentElement.dataset.titlebar).toBeUndefined();

            await events.fullscreenChanged.emit({ fullscreen: false });
            expect(document.documentElement.dataset.titlebar).toBe('overlay');
        });

        it('starts without the inset when the window opens fullscreen', async () => {
            harness = installIpcHarness({
                window_chrome: { overlay_title_bar: true, fullscreen: true },
            });
            const store = new WindowChromeStore();
            teardowns.push(await store.wire());
            expect(store.overlay).toBe(true);
            expect(store.inset).toBe(false);
            expect(document.documentElement.dataset.titlebar).toBeUndefined();
        });

        it('keeps native chrome with native decorations', async () => {
            harness = installIpcHarness({
                window_chrome: { overlay_title_bar: false, fullscreen: false },
            });
            const store = new WindowChromeStore();
            teardowns.push(await store.wire());
            await events.fullscreenChanged.emit({ fullscreen: false });
            expect(store.dragRegion).toBeUndefined();
            expect(store.dragSurface).toBeUndefined();
            expect(document.documentElement.dataset.titlebar).toBeUndefined();
        });

        it('never lets a slower seed answer overwrite a newer fullscreen event', async () => {
            let answer: (value: unknown) => void = () => {};
            harness = installIpcHarness({
                window_chrome: () =>
                    new Promise((resolve) => {
                        answer = resolve;
                    }),
            });
            const store = new WindowChromeStore();
            const wired = store.wire();
            await flushIpc();

            await events.fullscreenChanged.emit({ fullscreen: true });
            answer({ overlay_title_bar: true, fullscreen: false }); // read before the event
            teardowns.push(await wired);
            expect(store.overlay).toBe(true);
            expect(store.fullscreen).toBe(true);
            expect(store.inset).toBe(false);
        });

        it('falls back to native chrome when the query fails or nothing answers', async () => {
            harness = installIpcHarness({
                window_chrome: rejectWith(commandErrorFixture({ kind: 'internal' })),
            });
            const failed = new WindowChromeStore();
            teardowns.push(await failed.wire());
            expect(failed.inset).toBe(false);

            harness.teardown();
            harness = installIpcHarness({}); // the command throws: no runtime
            const unanswered = new WindowChromeStore();
            teardowns.push(await unanswered.wire());
            expect(unanswered.inset).toBe(false);
            expect(document.documentElement.dataset.titlebar).toBeUndefined();
        });
    });

    describe('pane headers under the overlay title bar', () => {
        it('the sidebar header drags the window and its buttons stay clickable', async () => {
            harness = installIpcHarness({ ...shellResponders(), connectivity_online: false });
            teardowns.push(await connectivityStore.wire()); // the badge is a tab stop
            emulateOverlay();
            const { container, getByLabelText, getByRole } = render(Sidebar);
            await flushIpc();

            const header = query(container, '.sidebar-brand');
            expect(header.getAttribute(DRAG_REGION_ATTR)).toBe('deep');
            expect(startsDrag(header)).toBe(true);
            expect(startsDrag(query(container, '.brand-name'))).toBe(true);
            const badge = getByRole('img', { name: 'Offline' });
            expect(startsDrag(badge)).toBe(false);

            const buttons = [...header.querySelectorAll('button')];
            expect(buttons).toHaveLength(3);
            for (const button of buttons) {
                expect(startsDrag(button), button.getAttribute('aria-label') ?? '').toBe(false);
                const icon = button.querySelector('svg');
                if (icon !== null) expect(startsDrag(icon)).toBe(false);
            }

            await fireEvent.click(getByLabelText('Add feed'));
            expect(layoutStore.activeModal).toBe('add-feed');
        });

        it('the list header drags the window; search and its tools stay controls', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container, getByLabelText } = render(ListPane);
            await flushIpc();

            const header = query(container, '.list-header');
            expect(header.getAttribute(DRAG_REGION_ATTR)).toBe('deep');
            expect(startsDrag(header)).toBe(true);
            const slot = query(container, '.search-slot');
            expect(startsDrag(slot)).toBe(false);
            for (const element of slot.querySelectorAll('*')) {
                expect(startsDrag(element)).toBe(false);
            }

            const toggle = getByLabelText('Show unread only');
            expect(startsDrag(toggle)).toBe(false);
            await fireEvent.click(toggle);
            expect(articlesStore.filters.read).toBe(false);
        });

        it('the reader toolbar drags the window; its buttons and popover do not', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            selectionStore.selectedArticleId = 100;
            const { container, getByLabelText } = render(ReaderPane);
            await flushIpc();

            const toolbar = query(container, '.reader-toolbar');
            expect(toolbar.getAttribute(DRAG_REGION_ATTR)).toBe('deep');
            expect(startsDrag(toolbar)).toBe(true);
            expect(startsDrag(query(toolbar, '.toolbar-spring'))).toBe(true);
            for (const button of toolbar.querySelectorAll('button')) {
                expect(startsDrag(button)).toBe(false);
            }
            // The typography popover lives in the toolbar: its padding is
            // part of the panel, not a drag handle.
            await fireEvent.click(getByLabelText('Typography'));
            const popover = query(container, '.typography-popover');
            expect(startsDrag(popover)).toBe(false);
            for (const element of popover.querySelectorAll('*')) {
                expect(startsDrag(element)).toBe(false);
            }
            // No article content is ever a drag handle.
            expect(startsDrag(query(container, 'article'))).toBe(false);
        });

        it('the empty reader keeps a header-high drag strip', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container, getByText } = render(ReaderPane);
            const strip = query(container, '.drag-strip');
            expect(startsDrag(strip)).toBe(true);
            expect(startsDrag(getByText('Select an article to read'))).toBe(false);
        });

        it('with the sidebar collapsed the list header leads, past the traffic lights', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container } = render(ListPane);
            expect(query(container, '.list-header').classList.contains('leading')).toBe(false);
            layoutStore.toggleSidebar();
            await tick();
            expect(query(container, '.list-header').classList.contains('leading')).toBe(true);
        });

        it('the recovery screen’s backdrop drags the window, its card does not', async () => {
            harness = installIpcHarness({});
            emulateOverlay();
            const { getByRole } = render(RecoveryScreen, {
                failure: startupFailureFixture(),
                onopen: () => {},
            });
            const main = getByRole('main');
            expect(startsDrag(main)).toBe(true);
            expect(startsDrag(getByRole('heading', { level: 1 }))).toBe(false);
        });
    });

    describe('native decorations and fullscreen', () => {
        for (const [name, setup] of [
            ['native decorations', () => {}],
            ['fullscreen', () => emulateOverlay(true)],
        ] as const) {
            it(`marks no drag regions with ${name}`, async () => {
                harness = installIpcHarness(shellResponders());
                setup();
                selectionStore.selectedArticleId = 100;
                const sidebar = render(Sidebar);
                const list = render(ListPane);
                const reader = render(ReaderPane);
                await flushIpc();
                for (const { container } of [sidebar, list, reader]) {
                    expect(container.querySelector(`[${DRAG_REGION_ATTR}]`)).toBeNull();
                }
            });
        }
    });
});
