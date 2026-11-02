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
 * A test pins the tauri version this copy was checked against, so a tauri
 * upgrade has to re-verify it (and the macOS checklist's drag steps).
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import ListPane from '$components/layout/ListPane.svelte';
import RecoveryScreen from '$components/layout/RecoveryScreen.svelte';
import ThreePane from '$components/layout/ThreePane.svelte';
import WelcomeView from '$components/layout/WelcomeView.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { events } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { connectivityStore } from '$lib/state/connectivity.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { PANE_LIMITS, SIDEBAR_HEADER_MIN, layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import {
    DRAG_REGION_ATTR,
    TITLEBAR_INSET_PX,
    WindowChromeStore,
    windowChromeStore,
} from '$lib/state/window-chrome.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { createRawSnippet, tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
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

/** The tauri minor `startsDrag` restates (see the header). */
const VERIFIED_TAURI = '2.12.';

const MAC_USER_AGENT =
    'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko)';

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
        vi.restoreAllMocks();
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

        it('assumes the overlay in the macOS app when the query fails', async () => {
            // tauri.conf.json overlays the macOS title bar whatever the store
            // hears: without the inset the traffic lights would sit on the
            // brand mark, and without drag regions nothing moves the window.
            vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(MAC_USER_AGENT);
            harness = installIpcHarness({
                window_chrome: rejectWith(commandErrorFixture({ kind: 'internal' })),
            });
            const failed = new WindowChromeStore();
            teardowns.push(await failed.wire());
            expect(failed.inset).toBe(true);
            expect(failed.dragRegion).toBe('deep');
            expect(document.documentElement.dataset.titlebar).toBe('overlay');

            harness.teardown();
            harness = installIpcHarness({}); // the command throws
            const unanswered = new WindowChromeStore();
            teardowns.push(await unanswered.wire());
            expect(unanswered.inset).toBe(true);
        });

        it('keeps native chrome in a plain browser on a Mac', async () => {
            vi.spyOn(navigator, 'userAgent', 'get').mockReturnValue(MAC_USER_AGENT);
            harness = installIpcHarness({});
            harness.teardown(); // no runtime left to invoke
            harness = null;
            const store = new WindowChromeStore();
            teardowns.push(await store.wire());
            expect(store.inset).toBe(false);
            expect(document.documentElement.dataset.titlebar).toBeUndefined();
        });

        it('mirrors the inset app.css sets, in px', () => {
            const css = readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8');
            const rule =
                /:root\[data-titlebar="overlay"\]\s*\{\s*--titlebar-inset:\s*(\d+)px;/.exec(css);
            expect(rule?.[1]).toBe(String(TITLEBAR_INSET_PX));
            const store = new WindowChromeStore();
            expect(store.insetPx).toBe(0);
            store.overlay = true;
            expect(store.insetPx).toBe(TITLEBAR_INSET_PX);
            store.fullscreen = true;
            expect(store.insetPx).toBe(0);
        });

        it('keeps a title bar’s words unselectable, outside its false subtrees', () => {
            const css = readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8');
            expect(css).toMatch(
                /\[data-tauri-drag-region="deep"\]\s*\{[^}]*-webkit-user-select:\s*none;[^}]*\buser-select:\s*none;/,
            );
            expect(css).toMatch(
                /\[data-tauri-drag-region="deep"\] \[data-tauri-drag-region="false"\]\s*\{[^}]*\buser-select:\s*text;/,
            );
        });

        it('restates the drag rule of the tauri it was checked against', () => {
            // Cargo.lock sits at the workspace root, two levels above the app.
            const lock = readFileSync(resolve(process.cwd(), '../../Cargo.lock'), 'utf8');
            const version = /\[\[package\]\]\nname = "tauri"\nversion = "([^"]+)"/.exec(lock)?.[1];
            expect(
                version?.startsWith(VERIFIED_TAURI),
                `tauri ${version}: re-check startsDrag against its drag.js, then bump VERIFIED_TAURI`,
            ).toBe(true);
        });
    });

    describe('sidebar width under the overlay title bar', () => {
        const pane = (name: string) =>
            createRawSnippet(() => ({ render: () => `<div class="${name}-stub"></div>` }));

        function renderFrame() {
            return render(ThreePane, {
                sidebar: pane('sidebar'),
                list: pane('list'),
                reader: pane('reader'),
            });
        }

        const overlayMin = SIDEBAR_HEADER_MIN + TITLEBAR_INSET_PX;

        it('never lets the sidebar narrow past what its header needs', async () => {
            harness = installIpcHarness({ set_setting: null });
            layoutStore.sidebarWidth = PANE_LIMITS.sidebar.min; // saved before the inset
            emulateOverlay();
            const { container, getByRole } = renderFrame();
            const frame = query(container, '.three-pane');
            const sidebar = query(container, '.pane.sidebar');
            const handle = getByRole('separator', { name: 'Resize sidebar' });

            expect(overlayMin).toBeGreaterThan(PANE_LIMITS.sidebar.min);
            expect(handle.getAttribute('aria-valuemin')).toBe(String(overlayMin));
            expect(handle.getAttribute('aria-valuenow')).toBe(String(overlayMin));
            expect(sidebar.style.width).toBe(`${overlayMin}px`);
            // The floor also outranks the narrow-window max-width caps.
            expect(frame.style.getPropertyValue('--sidebar-floor')).toBe(`${overlayMin}px`);
            // The stored width is left alone until the user resizes.
            expect(layoutStore.sidebarWidth).toBe(PANE_LIMITS.sidebar.min);

            layoutStore.sidebarWidth = 300;
            await tick();
            await fireEvent.keyDown(handle, { key: 'Home' });
            expect(layoutStore.sidebarWidth).toBe(overlayMin);
            await fireEvent.keyDown(handle, { key: 'ArrowLeft' });
            expect(layoutStore.sidebarWidth).toBe(overlayMin);
        });

        it('keeps the usual bounds with native chrome and in fullscreen', async () => {
            harness = installIpcHarness({ set_setting: null });
            layoutStore.sidebarWidth = PANE_LIMITS.sidebar.min;
            emulateOverlay(true);
            const { container, getByRole } = renderFrame();
            const handle = getByRole('separator', { name: 'Resize sidebar' });
            expect(handle.getAttribute('aria-valuemin')).toBe(String(PANE_LIMITS.sidebar.min));
            expect(query(container, '.pane.sidebar').style.width).toBe(
                `${PANE_LIMITS.sidebar.min}px`,
            );
            expect(query(container, '.three-pane').style.getPropertyValue('--sidebar-floor')).toBe(
                `${SIDEBAR_HEADER_MIN}px`,
            );

            // Leaving fullscreen brings the traffic lights, and the floor, back.
            windowChromeStore.fullscreen = false;
            await tick();
            expect(handle.getAttribute('aria-valuemin')).toBe(String(overlayMin));
            expect(query(container, '.pane.sidebar').style.width).toBe(`${overlayMin}px`);
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

        it('the reader toolbar stays the top row under a refetch-failure banner', async () => {
            harness = installIpcHarness(shellResponders());
            const unwire = await wireInvalidation();
            teardowns.push(unwire);
            emulateOverlay();
            selectionStore.selectedArticleId = 100;
            const { container, getByRole } = render(ReaderPane);
            await flushIpc();

            harness.respond('get_article', rejectWith(commandErrorFixture({ kind: 'user' })));
            await events.articlesChanged.emit({ feed_id: null, flags: null });
            await flushIpc();

            const banner = getByRole('alert');
            const reader = query(container, '.reader');
            expect(reader.firstElementChild?.classList.contains('reader-toolbar')).toBe(true);
            expect(query(container, '.reader-toolbar').nextElementSibling).toBe(
                banner.closest('.reader-banner'),
            );
            expect(startsDrag(query(container, '.toolbar-spring'))).toBe(true);
        });

        it('the empty reader keeps a header-high drag strip', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container, getByText } = render(ReaderPane);
            const strip = query(container, '.drag-strip');
            expect(startsDrag(strip)).toBe(true);
            expect(startsDrag(getByText('Select an article to read'))).toBe(false);
        });

        it('the first-run welcome drags by its top edge and leads with the sidebar away', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container, getByText, getByLabelText } = render(WelcomeView);
            const bar = query(container, '.welcome-bar');
            expect(bar.getAttribute(DRAG_REGION_ATTR)).toBe('deep');
            expect(startsDrag(bar)).toBe(true);
            expect(startsDrag(getByText('Welcome to Curio'))).toBe(false);
            expect(bar.classList.contains('leading')).toBe(false);

            layoutStore.toggleSidebar();
            await tick();
            expect(bar.classList.contains('leading')).toBe(true);
            // The toggle it carries then is a control, never a drag handle.
            expect(startsDrag(getByLabelText('Show sidebar'))).toBe(false);
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

        it('a narrow window folding the sidebar away makes the list header lead', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            const { container } = render(ListPane);
            layoutStore.setNarrow(true);
            await tick();
            expect(query(container, '.list-header').classList.contains('leading')).toBe(true);
            // Peeked back open, the sidebar's own header holds the lights again.
            layoutStore.toggleSidebar();
            await tick();
            expect(query(container, '.list-header').classList.contains('leading')).toBe(false);
        });

        it('in focus mode the reader toolbar leads, past the traffic lights', async () => {
            harness = installIpcHarness(shellResponders());
            emulateOverlay();
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            expect(query(container, '.reader-toolbar').classList.contains('leading')).toBe(false);
            layoutStore.toggleFocusMode();
            await tick();
            expect(query(container, '.reader-toolbar').classList.contains('leading')).toBe(true);
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
                const welcome = render(WelcomeView);
                await flushIpc();
                for (const { container } of [sidebar, list, reader, welcome]) {
                    expect(container.querySelector(`[${DRAG_REGION_ATTR}]`)).toBeNull();
                }
            });
        }
    });
});
