/**
 * The shell's pane layout over mockIPC: the sidebar toggle (button, `\`,
 * the native menu id), focus mode (`f`, the reader's toolbar button), the
 * narrow-window auto-collapse — and the rule under all of them, that the
 * keyboard never lands on a pane that is not on screen. Plus the native
 * menu's View items: every id it emits is a registered shortcut, so the
 * menu, the `?` overlay and the keys can never disagree.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { t } from '$lib/i18n';
import { SHORTCUTS } from '$lib/keyboard/registry';
import { routeMenuAction } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { evacuateHiddenPanes, inPane } from '$lib/state/pane-focus';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
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

function shellResponders(): Record<string, Responder> {
    return {
        list_feeds: [feedFixture({ id: 1, title: 'Alpha' })],
        get_unread_counts: unreadCountsFixture({ total: 3, by_feed: [[1, 3]] }),
        list_articles: (args) =>
            (args.params as { before: number | null }).before === null
                ? [300, 299, 298].map((id) => articleSummaryFixture({ id, feed_id: 1 }))
                : [],
        get_article: (args) => articleFixture({ id: args.articleId as number }),
        get_article_state: articleStateFixture(),
        mark_read: true,
        set_setting: null,
        record_opened: null,
    };
}

async function mountShell() {
    const mounted = render(Page);
    await flushIpc();
    return mounted;
}

function slot(container: HTMLElement, pane: 'sidebar' | 'list'): HTMLElement {
    const element = container.querySelector(`[data-pane="${pane}"]`);
    if (!(element instanceof HTMLElement)) {
        throw new Error(`no ${pane} slot`);
    }
    return element;
}

function isHidden(element: HTMLElement): boolean {
    // `inert` as the property: jsdom does not reflect it to the attribute.
    return element.inert && element.classList.contains('parked');
}

/** Whether DOM focus sits inside `pane`'s slot. */
function focusIn(pane: 'sidebar' | 'list'): boolean {
    return inPane(document.activeElement, pane);
}

describe('pane layout — sidebar collapse', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('the header button hides and shows the sidebar, persisted', async () => {
        harness = installIpcHarness(shellResponders());
        const { container, getByRole } = await mountShell();
        expect(isHidden(slot(container, 'sidebar'))).toBe(false);

        const hide = getByRole('button', { name: t('pane.sidebar.hide') });
        expect(hide.getAttribute('aria-expanded')).toBe('true');
        await fireEvent.click(hide);
        await flushIpc();

        expect(isHidden(slot(container, 'sidebar'))).toBe(true);
        expect(isHidden(slot(container, 'list'))).toBe(false);
        const show = getByRole('button', { name: t('pane.sidebar.show') });
        expect(show.getAttribute('aria-expanded')).toBe('false');
        expect(harness.callsFor('set_setting')).toContainEqual({
            key: 'ui.panes.sidebar-collapsed',
            value: 'true',
        });

        await fireEvent.click(show);
        expect(isHidden(slot(container, 'sidebar'))).toBe(false);
    });

    it('\\ toggles the sidebar from the keyboard, and so does the native menu', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();

        await fireEvent.keyDown(window, { key: '\\' });
        expect(isHidden(slot(container, 'sidebar'))).toBe(true);
        routeMenuAction('view.toggleSidebar');
        await tick();
        expect(isHidden(slot(container, 'sidebar'))).toBe(false);
    });

    it('slides only once the layout changes in this session', async () => {
        harness = installIpcHarness(shellResponders());
        const { getByTestId } = await mountShell();
        const frame = getByTestId('three-pane');
        expect(frame.classList.contains('motion')).toBe(false);

        await fireEvent.keyDown(window, { key: '\\' });
        expect(frame.classList.contains('motion')).toBe(true);
    });

    it('hiding the sidebar while it owns the keyboard hands it to the list', async () => {
        harness = installIpcHarness(shellResponders());
        const { container, getByRole } = await mountShell();
        await fireEvent.keyDown(window, { key: 'g' });
        await fireEvent.keyDown(window, { key: 'f' });
        await tick();
        const tree = getByRole('tree');
        expect(document.activeElement).toBe(tree);
        expect(selectionStore.focus).toBe('sidebar');

        // The tree swallows global keys, but the layout toggles get through.
        await fireEvent.keyDown(tree, { key: '\\' });
        await flushIpc();

        expect(isHidden(slot(container, 'sidebar'))).toBe(true);
        expect(focusIn('sidebar')).toBe(false);
        expect(selectionStore.focus).toBe('list');
        expect(document.activeElement).toBe(container.querySelector('[role="listbox"]'));
        // …and the shortcuts work again (the zone is no longer the sidebar).
        await fireEvent.keyDown(document.activeElement ?? window, { key: 'j' });
        expect(selectionStore.selectedArticleId).toBe(300);
    });

    it('g f reveals a collapsed sidebar before handing it the keyboard', async () => {
        harness = installIpcHarness(shellResponders());
        const { container, getByRole } = await mountShell();
        await fireEvent.keyDown(window, { key: '\\' });
        expect(isHidden(slot(container, 'sidebar'))).toBe(true);

        await fireEvent.keyDown(window, { key: 'g' });
        await fireEvent.keyDown(window, { key: 'f' });
        await tick();

        expect(isHidden(slot(container, 'sidebar'))).toBe(false);
        expect(document.activeElement).toBe(getByRole('tree'));
        expect(layoutStore.sidebarCollapsed).toBe(false);
    });

    it('a narrow window folds the sidebar away on its own', async () => {
        harness = installIpcHarness(shellResponders());
        const listeners: Array<(event: MediaQueryListEvent) => void> = [];
        vi.stubGlobal('matchMedia', (media: string) => ({
            media,
            matches: false,
            addEventListener: (_: string, listener: (event: MediaQueryListEvent) => void) =>
                listeners.push(listener),
            removeEventListener: () => {},
        }));
        try {
            const { container } = await mountShell();
            expect(isHidden(slot(container, 'sidebar'))).toBe(false);
            for (const listener of listeners) {
                listener({ matches: true } as MediaQueryListEvent);
            }
            await tick();
            expect(isHidden(slot(container, 'sidebar'))).toBe(true);
            // Nothing was persisted: widening the window brings it back.
            expect(harness.callsFor('set_setting')).toEqual([]);
            for (const listener of listeners) {
                listener({ matches: false } as MediaQueryListEvent);
            }
            await tick();
            expect(isHidden(slot(container, 'sidebar'))).toBe(false);
        } finally {
            vi.unstubAllGlobals();
        }
    });
});

describe('pane layout — focus mode', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('f hides the list and the sidebar, persisted, and f brings them back', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();

        await fireEvent.keyDown(window, { key: 'f' });
        await flushIpc();
        expect(isHidden(slot(container, 'list'))).toBe(true);
        expect(isHidden(slot(container, 'sidebar'))).toBe(true);
        expect(harness.callsFor('set_setting')).toContainEqual({
            key: 'ui.panes.focus-mode',
            value: 'true',
        });

        await fireEvent.keyDown(window, { key: 'f' });
        expect(isHidden(slot(container, 'list'))).toBe(false);
        expect(isHidden(slot(container, 'sidebar'))).toBe(false);
    });

    it('entering focus mode from the list hands the keyboard to the reader', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        const listbox = container.querySelector('[role="listbox"]');
        if (!(listbox instanceof HTMLElement)) {
            throw new Error('no listbox');
        }
        listbox.focus();
        expect(focusIn('list')).toBe(true);

        await fireEvent.keyDown(listbox, { key: 'f' });
        await flushIpc();

        expect(focusIn('list')).toBe(false);
        expect(document.activeElement).toBe(container.querySelector('.reader-scroll'));
    });

    it('← / Esc in the reader never aim the keyboard at the hidden list', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();
        await fireEvent.keyDown(document.body, { key: 'Enter' });
        await flushIpc();
        const body = container.querySelector('.reader-scroll');
        if (!(body instanceof HTMLElement)) {
            throw new Error('no reader body');
        }
        await fireEvent.keyDown(body, { key: 'f' });
        await flushIpc();
        const nonce = selectionStore.listFocusNonce;

        await fireEvent.keyDown(body, { key: 'ArrowLeft' });
        await fireEvent.keyDown(body, { key: 'Escape' });
        await flushIpc();

        expect(selectionStore.listFocusNonce).toBe(nonce);
        expect(focusIn('list')).toBe(false);
        expect(document.activeElement).toBe(body);
        // Esc stayed with the reader: the article is still open.
        expect(selectionStore.selectedArticleId).toBe(300);
    });

    it('the reader toolbar button enters and leaves focus mode', async () => {
        harness = installIpcHarness(shellResponders());
        const { container, getByRole } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();

        const enter = getByRole('button', { name: t('reader.action.focusMode') });
        expect(enter.getAttribute('aria-pressed')).toBe('false');
        await fireEvent.click(enter);
        expect(isHidden(slot(container, 'list'))).toBe(true);

        const exit = getByRole('button', { name: t('reader.action.exitFocusMode') });
        expect(exit.getAttribute('aria-pressed')).toBe('true');
        await fireEvent.click(exit);
        expect(isHidden(slot(container, 'list'))).toBe(false);
    });

    it('the empty reader names the key out of focus mode', async () => {
        harness = installIpcHarness(shellResponders());
        const { getByText, queryByText } = await mountShell();
        expect(queryByText(t('reader.hint.exitFocus'))).toBeNull();

        await fireEvent.keyDown(window, { key: 'f' });

        const hint = getByText(t('reader.hint.exitFocus'));
        expect(hint.querySelector('kbd')?.textContent).toBe('f');
    });
});

describe('evacuateHiddenPanes', () => {
    afterEach(() => {
        document.body.replaceChildren();
        selectionStore.reset();
        layoutStore.reset();
    });

    /** A focusable control inside a `data-pane` slot. */
    function controlIn(pane: 'sidebar' | 'list'): HTMLButtonElement {
        const wrapper = document.createElement('div');
        wrapper.dataset.pane = pane;
        const button = document.createElement('button');
        wrapper.append(button);
        document.body.append(wrapper);
        return button;
    }

    it('leaves focus alone while its pane is on screen', () => {
        const button = controlIn('sidebar');
        button.focus();
        evacuateHiddenPanes();
        expect(document.activeElement).toBe(button);
        expect(selectionStore.listFocusNonce).toBe(0);
    });

    it('moves focus out of a hidden sidebar to the list', () => {
        const button = controlIn('sidebar');
        button.focus();
        layoutStore.sidebarCollapsed = true;
        evacuateHiddenPanes();
        expect(document.activeElement).toBe(document.body);
        expect(selectionStore.focus).toBe('list');
        expect(selectionStore.listFocusNonce).toBe(1);
    });

    it('clears a sidebar focus zone even when DOM focus already fell away', () => {
        selectionStore.focusSidebar();
        layoutStore.sidebarCollapsed = true;
        evacuateHiddenPanes();
        expect(selectionStore.focus).toBe('list');
    });

    it('sends a hidden list’s focus to the reader, never the list', () => {
        const button = controlIn('list');
        button.focus();
        layoutStore.focusMode = true;
        evacuateHiddenPanes();
        expect(document.activeElement).toBe(document.body);
        expect(selectionStore.readerFocusNonce).toBe(1);
        expect(selectionStore.listFocusNonce).toBe(0);
    });
});

describe('native menu — View items', () => {
    // Read at run time from the checkout this test runs in (vitest runs
    // from apps/desktop).
    const menuSource = readFileSync(resolve(process.cwd(), 'src-tauri/src/menu.rs'), 'utf8');
    const ids = [...menuSource.matchAll(/with_id\("([^"]+)"/g)].map((match) => match[1]);

    it('carries the layout toggles, with modifier chords', () => {
        expect(ids).toContain('view.toggleSidebar');
        expect(ids).toContain('view.toggleFocusMode');
        expect(menuSource).toMatch(
            /with_id\("view\.toggleSidebar"[^;]*\.accelerator\("CmdOrCtrl\+Alt\+S"\)/,
        );
        expect(menuSource).toMatch(
            /with_id\("view\.toggleFocusMode"[^;]*\.accelerator\("CmdOrCtrl\+Alt\+F"\)/,
        );
    });

    it('emits only registered shortcut ids (or the two menu-only links)', () => {
        const known = new Set<string>([
            ...SHORTCUTS.map((shortcut) => shortcut.id),
            'menu.docs',
            'menu.reportIssue',
        ]);
        expect(ids.length).toBeGreaterThan(0);
        for (const id of ids) {
            expect(known.has(id), id).toBe(true);
        }
    });
});
