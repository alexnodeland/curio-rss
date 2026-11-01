/**
 * The action layer between input surfaces (keyboard shortcuts, toolbar
 * buttons, sidebar clicks) and the backend: every mutation goes through the
 * generated wrappers, failures become toasts, and list/count updates arrive
 * via the Rust-emitted invalidation events — never client-side bookkeeping.
 * Toggles read the authoritative flag first (`get_article_state`), then
 * write the flip; core's idempotency `bool` makes double-fires harmless.
 */
import {
    events,
    type ArticleSummaryDto,
    type CommandError,
    type ImportSourceDto,
    commands,
} from '$lib/bindings';
import { t } from '$lib/i18n';
import { SHORTCUTS, type ShortcutId } from '$lib/keyboard/registry';
import { commandErrorMessage } from '$lib/utils/errors';
import { isOpenableUrl, openExternal } from '$lib/utils/external';
import {
    ALL_ARTICLES,
    type ArticleFilters,
    articlesStore,
    filterKey,
    unreadScopeDto,
} from './articles.svelte';
import { destinationsStore } from './destinations.svelte';
import { feedsStore } from './feeds.svelte';
import { feedInFolder } from './folder-ops';
import { layoutStore } from './layout.svelte';
import type { CommandResult } from './query-cache.svelte';
import { searchStore } from './search.svelte';
import { selectionStore } from './selection.svelte';
import { toastStore } from './toasts.svelte';

/** Surfaces a command failure as an error toast. */
export function toastCommandError(error: CommandError): void {
    toastStore.showToast(commandErrorMessage(error), 'error');
}

/** Unwraps a command result; failures (including thrown) become toasts. */
async function run<T>(invoke: () => Promise<CommandResult<T>>): Promise<T | undefined> {
    let result: CommandResult<T>;
    try {
        result = await invoke();
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
        return undefined;
    }
    if (result.status === 'error') {
        toastCommandError(result.error);
        return undefined;
    }
    return result.data;
}

/**
 * Public unwrap for fire-and-forget mutation call sites that would otherwise
 * `void` the `CommandResult` and swallow failures (sidebar/folder ops). A
 * failure toasts; success is silent. Route every such mutation through this so
 * no failure is lost and none falsely reports success.
 */
export function runCommand<T>(invoke: () => Promise<CommandResult<T>>): Promise<T | undefined> {
    return run(invoke);
}

type ToggleField = 'starred' | 'read' | 'read_later' | 'archived';

/**
 * Per-article-field toggle serialization. A naive toggle reads the backend flag
 * then writes its negation; two fast presses both read the *pre-write* value
 * concurrently and both write the same negation, so the second press is
 * silently dropped. Serializing per key (chained promise) makes the second
 * press run only after the first settles, and `#toggleDesired` carries the
 * last-written value so the follow-up alternates without a redundant read.
 */
const toggleChain = new Map<string, Promise<unknown>>();
const toggleDesired = new Map<string, boolean>();

async function toggleFlag(
    articleId: number,
    field: ToggleField,
    write: (id: number, value: boolean) => Promise<CommandResult<boolean>>,
): Promise<void> {
    const key = `${field}:${articleId}`;
    const previous = toggleChain.get(key) ?? Promise.resolve();
    const task = previous.then(async () => {
        let current: boolean;
        if (toggleDesired.has(key)) {
            current = toggleDesired.get(key) as boolean;
        } else {
            const state = await run(() => commands.getArticleState(articleId));
            if (state === undefined) {
                return; // read failed (a toast fired); nothing to flip
            }
            current = state[field];
        }
        const target = !current;
        toggleDesired.set(key, target);
        const ok = await run(() => write(articleId, target));
        if (ok === undefined) {
            // The write failed (toast already fired) — forget the optimistic
            // value so the next press reads the real backend flag again.
            toggleDesired.delete(key);
        }
    });
    toggleChain.set(key, task);
    await task;
    // Drain once this press is the tail of the chain, so the maps don't grow.
    if (toggleChain.get(key) === task) {
        toggleChain.delete(key);
        toggleDesired.delete(key);
    }
}

/** Stars / unstars against the current backend flag. */
export async function toggleStar(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'starred', (id, value) => commands.setStarred(id, value));
}

/** Adds to / removes from the read-later queue. */
export async function toggleReadLater(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'read_later', (id, value) => commands.setReadLater(id, value));
}

/** Marks read / unread against the current backend flag. */
export async function toggleRead(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'read', (id, value) => commands.markRead(id, value));
}

/** Archives / unarchives against the current backend flag. */
export async function toggleArchived(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'archived', (id, value) => commands.setArchived(id, value));
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
 * Mark-on-scroll: a row that scrolled up out of the list marks itself read.
 * Silent by design — this is a background sweep that can fire once per row,
 * so a transient failure must never raise a toast. Core no-ops when the row
 * was already read, so re-scrolling the same rows costs nothing.
 */
export async function markReadOnScroll(articleId: number): Promise<void> {
    try {
        await commands.markRead(articleId, true);
    } catch {
        // A background read-mark that fails is not worth interrupting for.
    }
}

/**
 * Fetches the article's source page and replaces its stored content with the
 * readability-extracted full article; the reader + list refetch off the
 * emitted `ArticlesChanged`. Returns whether it succeeded (for a loading UI).
 */
export async function loadFullArticle(articleId: number): Promise<boolean> {
    return (await run(() => commands.loadFullArticle(articleId))) !== undefined;
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
    toastStore.showToast(t('toast.markAllRead', { count: changed }), 'success');
}

/** How many unread rows one mark-view-read listing page asks for. */
const MARK_VIEW_PAGE = 200;

/** Marks each id read, one command apiece; stops at (and toasts) the first failure. */
async function markEachRead(articleIds: readonly number[]): Promise<number | undefined> {
    let changed = 0;
    for (const articleId of articleIds) {
        const flipped = await run(() => commands.markRead(articleId, true));
        if (flipped === undefined) {
            return undefined;
        }
        if (flipped) {
            changed += 1;
        }
    }
    return changed;
}

/** One page of a view's unread rows strictly below `before`. */
function unreadPage(
    filters: ArticleFilters,
    before: number | null,
): Promise<ArticleSummaryDto[] | undefined> {
    return run(() => commands.listArticles(unreadScopeDto(filters, before, MARK_VIEW_PAGE)));
}

/** Every unread article id in a filtered view, paged backend-side. */
async function unreadIdsInView(filters: ArticleFilters): Promise<number[] | undefined> {
    const ids: number[] = [];
    let before: number | null = null;
    for (;;) {
        const page = await unreadPage(filters, before);
        if (page === undefined) {
            return undefined;
        }
        ids.push(...page.map((article) => article.id));
        const last = page.at(-1);
        if (page.length < MARK_VIEW_PAGE || last === undefined) {
            return ids;
        }
        before = last.id;
    }
}

/**
 * Whether the backend's bulk command covers a view: it scopes by feed or the
 * whole library, so any narrowing flag (starred, read later, a tag, the
 * Archived view) has no bulk form.
 */
function hasBulkScope(filters: ArticleFilters): boolean {
    return (
        filters.starred === null &&
        filters.readLater === null &&
        filters.tag === null &&
        filters.archived !== true
    );
}

/** Bulk-marks every feed in folder `path` read; the summed count. */
async function markFolderFeedsRead(path: string): Promise<number | undefined> {
    let changed = 0;
    for (const feed of feedsStore.feeds.filter((candidate) => feedInFolder(candidate, path))) {
        const flipped = await run(() => commands.markAllRead(feed.id));
        if (flipped === undefined) {
            return undefined;
        }
        changed += flipped;
    }
    return changed;
}

/** The count of articles a view-wide mark flipped, or `undefined` on failure. */
async function markViewReadCount(): Promise<number | undefined> {
    if (searchStore.active) {
        // Search results are a fixed FTS window: mark exactly what is listed.
        return markEachRead(articlesStore.activeItems.map((article) => article.id));
    }
    const filters = articlesStore.filters;
    if (filters.read === true) {
        return 0; // a read-only view holds nothing unread
    }
    if (hasBulkScope(filters) && filters.feedTag === null) {
        return run(() => commands.markAllRead(filters.feedId));
    }
    if (hasBulkScope(filters) && filters.feedTag !== null && filters.feedId === null) {
        return markFolderFeedsRead(filters.feedTag);
    }
    const ids = await unreadIdsInView(filters);
    return ids === undefined ? undefined : markEachRead(ids);
}

/**
 * Marks everything unread in the current view read (`Shift+A`) and toasts how
 * many changed. A feed or the whole library rides the backend's bulk
 * command, a folder its feeds' bulk marks; every narrower view (Starred,
 * Read later, Archived, a tag, search results) lists its own unread and
 * marks those, so nothing outside the view is touched.
 */
export async function markViewRead(): Promise<void> {
    const changed = await markViewReadCount();
    if (changed !== undefined) {
        toastStore.showToast(t('toast.markAllRead', { count: changed }), 'success');
    }
}

/** The reader's scroll body while one is mounted (registered by ReaderPane). */
let readerBody: HTMLElement | null = null;

/**
 * Registers the mounted reader scroll body as the Space-paging target.
 * Returns the unregister (a no-op once a newer body took over).
 */
export function registerReaderBody(element: HTMLElement): () => void {
    readerBody = element;
    return () => {
        if (readerBody === element) {
            readerBody = null;
        }
    };
}

/** A Space page is 7/8 of the viewport — the browser's own step, keeping a line of context. */
const PAGE_FRACTION = 0.875;

/** Sub-pixel slack when deciding the reader sits at the article's end. */
const END_SLACK_PX = 2;

/**
 * Space / Shift+Space: pages the reader body down / up. Space at the end of
 * the article — or with nothing selected — advances to the next unread
 * (Reeder / NetNewsWire). While the selected article is still loading there
 * is no body to page, and the press is dropped rather than skipping an
 * article the reader never saw.
 */
export function pageReader(direction: 'down' | 'up'): void {
    if (selectionStore.selectedArticleId === null) {
        if (direction === 'down') {
            void goToNextUnread();
        }
        return;
    }
    const body = readerBody;
    if (body === null) {
        return;
    }
    const atEnd = body.scrollTop + body.clientHeight >= body.scrollHeight - END_SLACK_PX;
    if (direction === 'down' && atEnd) {
        void goToNextUnread();
        return;
    }
    const step = Math.round(body.clientHeight * PAGE_FRACTION);
    // A plain scrollTop write honours the body's CSS `scroll-behavior`
    // (smooth unless the user asked for reduced motion).
    body.scrollTop = Math.max(0, body.scrollTop + (direction === 'down' ? step : -step));
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
        toastStore.showToast(t('app.error.internal'), 'error');
        return;
    }
    await run(() => commands.recordOpened(articleId, null));
}

/**
 * Runs the native import flow (OPML by default): a Rust-side file picker
 * returns an opaque PathToken, the chosen source's parser ingests it, and
 * the outcome is toasted. Invalidation rides the Rust-emitted events, so the
 * list and sidebar refresh on their own. Shared by the Settings import panel
 * and the new-user empty state.
 */
export async function importFromFile(source: ImportSourceDto = 'opml'): Promise<void> {
    try {
        const picked = await commands.pickImportFile();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            toastStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const result = await commands.importFile(picked.data.token, source);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        const {
            feeds_added,
            articles_added,
            feeds_skipped,
            articles_skipped,
            feeds_invalid,
            articles_invalid,
        } = result.data;
        const counts = {
            feeds: feeds_added,
            articles: articles_added,
            skipped: feeds_skipped + articles_skipped,
        };
        // Unusable rows (a broken outline, a webcal: or junk URL) never fail
        // the import, but they are not silent either: the toast says how many
        // were left out, as a warning rather than a success.
        const invalid = feeds_invalid + articles_invalid;
        if (invalid > 0) {
            toastStore.showToast(t('import.doneWithInvalid', { ...counts, invalid }), 'warning');
        } else {
            toastStore.showToast(t('import.done', counts), 'success');
        }
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    }
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

/**
 * Scopes the article list to a folder — every article whose feed carries this
 * `/`-path tag or a tag nested beneath it (backend-owned via `feed_tag`).
 * Clears the feed scope + search so the folder is the active selection.
 */
export function selectFolder(path: string): void {
    searchStore.clear();
    selectionStore.selectedFeedId = null;
    selectionStore.selectedArticleId = null;
    articlesStore.filters = { ...ALL_ARTICLES, feedTag: path };
}

/**
 * Jumps to the next unread article, crossing feed boundaries. The next unread
 * strictly below the selection is chosen *within the current view* — the
 * displayed filter is left untouched (an unread row is present in an
 * all/unread view, so no silent switch to unread-only is needed). When the
 * current scope is out of unread, it hops to the next feed with unread and
 * takes its first; when nothing unread remains anywhere, it says so.
 */
export async function goToNextUnread(): Promise<void> {
    const before = selectionStore.selectedArticleId;
    const next = await run(() =>
        commands.listArticles(unreadScopeDto(articlesStore.filters, before)),
    );
    if (next !== undefined && next.length > 0) {
        selectionStore.selectedArticleId = next[0].id;
        return;
    }
    const feed = feedsStore.nextFeedWithUnread(selectionStore.selectedFeedId);
    if (feed === null) {
        toastStore.showToast(t('nav.noMoreUnread'), 'info');
        return;
    }
    searchStore.clear();
    selectionStore.selectedFeedId = feed;
    articlesStore.filters = { ...ALL_ARTICLES, feedId: feed };
    const first = await run(() =>
        commands.listArticles(unreadScopeDto({ ...ALL_ARTICLES, feedId: feed }, null)),
    );
    if (first !== undefined && first.length > 0) {
        selectionStore.selectedArticleId = first[0].id;
    }
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
    await destinationsStore.ensureLoaded();
    const target = destinationsStore.promoteTarget;
    if (target === null) {
        layoutStore.openModal('destinations');
        return;
    }
    const outcome = await run(() => destinationsStore.promote(articleId, target));
    if (outcome === undefined) {
        return;
    }
    const key =
        outcome.disposition === 'unchanged' ? 'toast.promote.unchanged' : 'toast.promote.saved';
    toastStore.showToast(t(key, { name: target }), 'success');
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

/**
 * Enter / → from the list: hands the keyboard to the reader body. With
 * nothing selected it selects first (the row j would pick), so the key is
 * never a dead end.
 */
function focusReader(): void {
    if (selectionStore.selectedArticleId === null) {
        selectNext();
    }
    if (selectionStore.selectedArticleId !== null) {
        selectionStore.focusReader();
    }
}

function toggleHelp(): void {
    if (layoutStore.activeModal === 'help') {
        layoutStore.closeModal();
    } else {
        layoutStore.openModal('help');
    }
}

/**
 * Executes one shortcut id. The core set (j/k, o, s, l, m, ?) landed in
 * WP3; WP4 wires the reader-breadth surfaces: `p` promote, `/` search
 * focus, `r`/`Shift+R` refresh, and the `g`-chord view switches. The pane
 * keys (listbox ↑/↓/Enter/→, reader ←/Esc) land here too, through the
 * panes' own handlers.
 */
export function handleShortcut(id: ShortcutId): void {
    switch (id) {
        case 'nav.nextArticle':
            selectNext();
            break;
        case 'nav.previousArticle':
            selectionStore.selectPreviousArticle();
            break;
        case 'nav.nextUnread':
            void goToNextUnread();
            break;
        case 'nav.focusReader':
            focusReader();
            break;
        case 'nav.focusList':
            selectionStore.focusList();
            break;
        case 'reader.pageDown':
            pageReader('down');
            break;
        case 'reader.pageUp':
            pageReader('up');
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
        case 'article.toggleArchived':
            withSelected(toggleArchived);
            break;
        case 'view.markAllRead':
            void markViewRead();
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
            layoutStore.openModal('add-feed');
            break;
        case 'app.settings':
            layoutStore.openModal('settings');
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
            selectionStore.focusSidebar();
            break;
        case 'help.toggle':
            toggleHelp();
            break;
        default:
            break;
    }
}

/** The public docs site + issue tracker, opened from the Help menu. */
const DOCS_URL = 'https://alexnodeland.github.io/curio-rss/';
const ISSUES_URL = 'https://github.com/alexnodeland/curio-rss/issues/new';
const SHORTCUT_IDS: ReadonlySet<string> = new Set(SHORTCUTS.map((shortcut) => shortcut.id));

/**
 * Runs a native-menu item id through the same action layer as a keyboard
 * shortcut: a `ShortcutId` runs `handleShortcut`; the two menu-only ids open
 * the docs / issue tracker; anything else (predefined native items like
 * copy/paste) is ignored.
 */
export function routeMenuAction(id: string): void {
    if (id === 'menu.docs') {
        void openExternal(DOCS_URL);
    } else if (id === 'menu.reportIssue') {
        void openExternal(ISSUES_URL);
    } else if (SHORTCUT_IDS.has(id)) {
        handleShortcut(id as ShortcutId);
    }
}

/**
 * Subscribes to the Rust `MenuAction` event and routes each click through
 * {@link routeMenuAction}. Returns an unsubscribe for teardown.
 */
export async function wireMenuActions(): Promise<() => void> {
    return events.menuAction.listen((event) => routeMenuAction(event.payload.id));
}
