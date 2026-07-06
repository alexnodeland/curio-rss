/**
 * The action layer between input surfaces (keyboard shortcuts, toolbar
 * buttons, sidebar clicks) and the backend: every mutation goes through the
 * generated wrappers, failures become toasts, and list/count updates arrive
 * via the Rust-emitted invalidation events — never client-side bookkeeping.
 * Toggles read the authoritative flag first (`get_article_state`), then
 * write the flip; core's idempotency `bool` makes double-fires harmless.
 */
import { type CommandError, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import type { ShortcutId } from '$lib/keyboard/registry';
import { commandErrorMessage } from '$lib/utils/errors';
import { openExternal } from '$lib/utils/external';
import { ALL_ARTICLES, type ArticleFilters, articlesStore, filterKey } from './articles.svelte';
import type { CommandResult } from './query-cache.svelte';
import { selectionStore } from './selection.svelte';
import { uiStore } from './ui.svelte';

/** Surfaces a command failure as an error toast. */
export function toastCommandError(error: CommandError): void {
    uiStore.showToast(commandErrorMessage(error), 'error');
}

/** Unwraps a command result; failures (including thrown) become toasts. */
async function run<T>(invoke: () => Promise<CommandResult<T>>): Promise<T | undefined> {
    let result: CommandResult<T>;
    try {
        result = await invoke();
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
        return undefined;
    }
    if (result.status === 'error') {
        toastCommandError(result.error);
        return undefined;
    }
    return result.data;
}

/** Stars / unstars against the current backend flag. */
export async function toggleStar(articleId: number): Promise<void> {
    const state = await run(() => commands.getArticleState(articleId));
    if (state === undefined) {
        return;
    }
    await run(() => commands.setStarred(articleId, !state.starred));
}

/** Adds to / removes from the read-later queue. */
export async function toggleReadLater(articleId: number): Promise<void> {
    const state = await run(() => commands.getArticleState(articleId));
    if (state === undefined) {
        return;
    }
    await run(() => commands.setReadLater(articleId, !state.read_later));
}

/** Marks read / unread against the current backend flag. */
export async function toggleRead(articleId: number): Promise<void> {
    const state = await run(() => commands.getArticleState(articleId));
    if (state === undefined) {
        return;
    }
    await run(() => commands.markRead(articleId, !state.read));
}

/** Archives / unarchives against the current backend flag. */
export async function toggleArchived(articleId: number): Promise<void> {
    const state = await run(() => commands.getArticleState(articleId));
    if (state === undefined) {
        return;
    }
    await run(() => commands.setArchived(articleId, !state.archived));
}

/**
 * Displaying an article in the reader marks it read (idempotent — core
 * emits nothing when it already was). The `article.opened` contract event
 * is reserved for a real open-in-browser, so the log stays truthful under
 * j/k scanning.
 */
export async function markReadOnOpen(articleId: number): Promise<void> {
    await run(() => commands.markRead(articleId, true));
}

/** Opens the article's source in the OS browser and records the open. */
export async function openInBrowser(articleId: number, url: string): Promise<void> {
    try {
        await openExternal(url);
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
        return;
    }
    await run(() => commands.recordOpened(articleId, null));
}

/** The sidebar's built-in views over the backend-owned filters. */
export type ViewId = 'all' | 'starred' | 'readLater';

const VIEW_FILTERS: Record<ViewId, ArticleFilters> = {
    all: ALL_ARTICLES,
    starred: { ...ALL_ARTICLES, starred: true },
    readLater: { ...ALL_ARTICLES, readLater: true },
};

/** Switches the article list to a built-in view (clears feed selection). */
export function selectView(view: ViewId): void {
    selectionStore.selectedFeedId = null;
    selectionStore.selectedArticleId = null;
    articlesStore.filters = VIEW_FILTERS[view];
}

/** The built-in view a filter set corresponds to, if any (highlighting). */
export function activeView(filters: ArticleFilters): ViewId | null {
    const key = filterKey(filters);
    for (const view of Object.keys(VIEW_FILTERS) as ViewId[]) {
        if (filterKey(VIEW_FILTERS[view]) === key) {
            return view;
        }
    }
    return null;
}

function selectNext(): void {
    const nearEnd = selectionStore.selectNextArticle();
    if (nearEnd) {
        void articlesStore.current.loadMore();
    }
}

function openSelected(): void {
    const selected = selectionStore.selectedArticle;
    if (selected !== null) {
        void openInBrowser(selected.id, selected.source_url);
    }
}

function withSelected(action: (articleId: number) => Promise<void>): void {
    const articleId = selectionStore.selectedArticleId;
    if (articleId !== null) {
        void action(articleId);
    }
}

function toggleHelp(): void {
    if (uiStore.activeModal === 'help') {
        uiStore.closeModal();
    } else {
        uiStore.openModal('help');
    }
}

/**
 * Executes one shortcut id. WP3 wires the core set (j/k, o, s, l, m, ?);
 * the rest of the registry (promote, refresh, search, g-chords) arrives
 * with the WP4/WP5 surfaces and is a deliberate no-op until then.
 */
export function handleShortcut(id: ShortcutId): void {
    switch (id) {
        case 'nav.nextArticle':
            selectNext();
            break;
        case 'nav.previousArticle':
            selectionStore.selectPreviousArticle();
            break;
        case 'article.open':
            openSelected();
            break;
        case 'article.toggleStar':
            withSelected(toggleStar);
            break;
        case 'article.toggleReadLater':
            withSelected(toggleReadLater);
            break;
        case 'article.toggleRead':
            withSelected(toggleRead);
            break;
        case 'help.toggle':
            toggleHelp();
            break;
        default:
            break;
    }
}
