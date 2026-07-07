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
import { isOpenableUrl, openExternal } from '$lib/utils/external';
import { ALL_ARTICLES, type ArticleFilters, articlesStore, filterKey } from './articles.svelte';
import { destinationsStore } from './destinations.svelte';
import { feedsStore } from './feeds.svelte';
import type { CommandResult } from './query-cache.svelte';
import { searchStore } from './search.svelte';
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

/**
 * Marks every unread article in a feed (`feedId`) — or the whole library
 * (`null`) — read, and toasts how many changed. The sidebar badges and the
 * list refresh off the Rust-emitted `ArticlesChanged`.
 */
export async function markAllRead(feedId: number | null): Promise<void> {
    const changed = await run(() => commands.markAllRead(feedId));
    if (changed === undefined) {
        return;
    }
    uiStore.showToast(t('toast.markAllRead', { count: changed }), 'success');
}

/**
 * Opens the article's source in the OS browser and records the open.
 * Non-http(s) URLs are refused outright — no open, no `article.opened`.
 */
export async function openInBrowser(articleId: number, url: string): Promise<void> {
    if (!isOpenableUrl(url)) {
        return;
    }
    try {
        await openExternal(url);
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
        return;
    }
    await run(() => commands.recordOpened(articleId, null));
}

/** The sidebar's built-in views over the backend-owned filters. */
export type ViewId = 'all' | 'starred' | 'readLater' | 'archived';

const VIEW_FILTERS: Record<ViewId, ArticleFilters> = {
    all: ALL_ARTICLES,
    starred: { ...ALL_ARTICLES, starred: true },
    readLater: { ...ALL_ARTICLES, readLater: true },
    // The one view that shows archived items (everything else hides them).
    archived: { ...ALL_ARTICLES, archived: true },
};

/** Switches the article list to a built-in view (clears feed + search). */
export function selectView(view: ViewId): void {
    searchStore.clear(); // a view switch leaves search mode
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
    // Search results are a fixed FTS window — only the filter list pages.
    if (nearEnd && !searchStore.active) {
        void articlesStore.current.loadMore();
    }
}

/**
 * Promotes the selected article to a destination by NAME. Uses the chosen
 * default (or the sole destination); with no clear target it opens the
 * destinations panel to pick one. A raw path never crosses IPC — only the
 * registry name. The outcome disposition drives the toast copy.
 */
export async function promoteSelected(): Promise<void> {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return;
    }
    const target = destinationsStore.promoteTarget;
    if (target === null) {
        uiStore.openModal('destinations');
        return;
    }
    const outcome = await run(() => destinationsStore.promote(articleId, target));
    if (outcome === undefined) {
        return;
    }
    const key =
        outcome.disposition === 'unchanged' ? 'toast.promote.unchanged' : 'toast.promote.saved';
    uiStore.showToast(t(key, { name: target }), 'success');
}

/** Focuses the search input (the `/` shortcut). */
function focusSearch(): void {
    searchStore.requestFocus();
}

/** Refreshes the selected feed, if any (the `r` shortcut). */
function refreshSelectedFeed(): void {
    const feedId = selectionStore.selectedFeedId;
    if (feedId === null) {
        return;
    }
    void run(() => feedsStore.refreshFeed(feedId));
}

/** Kicks off a full refresh sweep (the `Shift+R` shortcut / toolbar). */
export function refreshAll(): void {
    void run(() => feedsStore.refreshAll());
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
 * Executes one shortcut id. The core set (j/k, o, s, l, m, ?) landed in
 * WP3; WP4 wires the reader-breadth surfaces: `p` promote, `/` search
 * focus, `r`/`Shift+R` refresh, and the `g`-chord view switches.
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
        case 'article.promote':
            void promoteSelected();
            break;
        case 'feed.refresh':
            refreshSelectedFeed();
            break;
        case 'app.refreshAll':
            refreshAll();
            break;
        case 'app.addFeed':
            uiStore.openModal('add-feed');
            break;
        case 'app.settings':
            uiStore.openModal('settings');
            break;
        case 'search.focus':
            focusSearch();
            break;
        case 'view.all':
            selectView('all');
            break;
        case 'view.starred':
            selectView('starred');
            break;
        case 'view.readLater':
            selectView('readLater');
            break;
        case 'view.archived':
            selectView('archived');
            break;
        case 'view.feeds':
            searchStore.clear();
            selectionStore.focus = 'sidebar';
            break;
        case 'help.toggle':
            toggleHelp();
            break;
        default:
            break;
    }
}
