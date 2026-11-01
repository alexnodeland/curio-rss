<script lang="ts" module>
import { type ReadingDensity, prefsStore } from '$lib/state/prefs.svelte';

/**
 * Fixed virtualizer row height in px per density. The row applies the same
 * number as an inline height, so the CSS and the windowing math share one
 * source and cannot drift.
 */
export const ROW_HEIGHTS: Record<ReadingDensity, number> = {
    comfortable: 84,
    compact: 56,
};

/** The comfortable height — the fixed row height where density isn't varied (e.g. search). */
export const ROW_HEIGHT = ROW_HEIGHTS.comfortable;
</script>

<script lang="ts">
/**
 * The middle pane: the current filter combination's loaded window through
 * the virtualizer, with keyset `before`-cursor infinite scroll (the store
 * owns the cursor; nearing the bottom of the window asks it for the next
 * page). Selection lives in `selectionStore`; the reader reacts to it.
 *
 * Each filter combination is its own view session: the virtualizer is keyed
 * on the list, so switching views mounts a fresh scroll container seated at
 * that view's remembered position (the top on a first visit) instead of
 * inheriting the previous view's offset.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { markReadOnScroll } from '$lib/state/actions';
import { type ArticleListState, articlesStore, filterKey } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { capturePosition, restoreScrollTop } from '$lib/state/list-position';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import EmptyState from '$components/common/EmptyState.svelte';
import Skeleton from '$components/common/Skeleton.svelte';
import { untrack } from 'svelte';
import ArticleRow, { articleOptionId, buildArticleMenu } from './ArticleRow.svelte';
import VirtualList, { type ViewportReport } from './VirtualList.svelte';

/** How many placeholder rows the loading skeleton sketches. */
const SKELETON_ROWS = Array.from({ length: 8 }, (_, index) => index);

// Rows resolve feed titles — prime the feed queries outside the template.
untrack(() => feedsStore.prime());

/**
 * The current filter combination's list state. Created through a $derived
 * — never inline in a template expression — because svelte excludes state
 * created inside a reaction from that reaction's dependencies: a template
 * that both created and read the query would go blind to its updates.
 */
const list = $derived(articlesStore.current);

const listError = $derived(list.error === null ? '' : commandErrorMessage(list.error));

/** Row density → the fixed row height and the compact-layout flag. */
const rowHeight = $derived(ROW_HEIGHTS[prefsStore.readingDensity]);
const compact = $derived(prefsStore.readingDensity === 'compact');

/** The listbox's active option: the selected row, when one is selected. */
const activeDescendantId = $derived(
    selectionStore.selectedArticleId === null
        ? undefined
        : articleOptionId(selectionStore.selectedArticleId),
);

function selectArticle(articleId: number): void {
    selectionStore.selectedArticleId = articleId;
}

/**
 * Opens the context menu for the selected row when the keyboard menu key fires
 * on the listbox (rows are `tabindex="-1"`, so their own menu-key never fires).
 * Anchors to the selected row's element; a no-op if nothing is selected or the
 * row has scrolled out of the virtualized window.
 */
function openSelectedMenu(): void {
    const id = selectionStore.selectedArticleId;
    if (id === null) {
        return;
    }
    const article = list.items.find((item) => item.id === id);
    const el = document.getElementById(articleOptionId(id));
    if (article === undefined || el === null) {
        return;
    }
    const state = ensureQuery(queryKeys.articleState(id), () => commands.getArticleState(id)).data;
    const rect = el.getBoundingClientRect();
    menuStore.openAt(buildArticleMenu(article, state), rect.left + 8, rect.bottom - 4, el, article.title);
}

/**
 * Jump navigation while the listbox holds focus (row-by-row movement is
 * `j`/`k`): Page keys move a page; Home/End jump to the ends of the loaded
 * window.
 */
function moveSelection(to: 'first' | 'last' | 'pageDown' | 'pageUp'): void {
    const items = list.items;
    if (to === 'pageDown') {
        if (selectionStore.selectPageDown()) {
            void list.loadMore();
        }
    } else if (to === 'pageUp') {
        selectionStore.selectPageUp();
    } else if (to === 'first' && items.length > 0) {
        selectionStore.selectedArticleId = items[0].id;
        selectionStore.rememberIndex(0);
    } else if (to === 'last' && items.length > 0) {
        selectionStore.selectedArticleId = items[items.length - 1].id;
        selectionStore.rememberIndex(items.length - 1);
    }
}

// Keep the selection store's last-present index current: whenever a real row is
// selected (j/k, click, first/last), remember where it sits so that when the
// row later leaves the window (archived out of the view; a read row is held
// until the view changes), j/k resume from here instead of teleporting to the top.
$effect(() => {
    const index = selectionStore.selectedIndex;
    if (index >= 0) {
        selectionStore.rememberIndex(index);
    }
});

/**
 * Mark-on-scroll bookkeeping for one view session, by article id (never by
 * index, so rows shifting — a refetch, rows arriving above — can't move the
 * mark onto rows the reader never saw). `seen` holds every row that has been
 * on screen; `marked` the rows already marked. A row is marked once, when a
 * row that was *seen* has scrolled up past the top — so opening a view at a
 * restored offset never marks the rows above it, which were never on screen.
 */
interface ScrollSession {
    readonly list: ArticleListState;
    readonly seen: Set<number>;
    readonly marked: Set<number>;
}

let session: ScrollSession | null = null;

/** The session for `current`, starting a fresh one when the list changed. */
function sessionFor(current: ArticleListState): ScrollSession {
    if (session === null || session.list !== current) {
        session = { list: current, seen: new Set(), marked: new Set() };
    }
    return session;
}

/** The offset the list opens at for the current view (read once per mount). */
function openingScrollTop(): number {
    return untrack(() =>
        restoreScrollTop(
            articlesStore.positions.get(filterKey(list.filters)),
            list.items,
            rowHeight,
        ),
    );
}

$effect(() => {
    // Re-key on the list identity: a new filter is a fresh reading session.
    void list;
    // If the prior selection isn't in the new window (an unread-only toggle or
    // feed switch dropped it), restart the resume anchor at the top so j/k
    // can't teleport off a now-stale index into an unrelated row.
    untrack(() => {
        if (selectionStore.selectedIndex === -1) {
            selectionStore.rememberIndex(0);
        }
    });
});

/** The final "no articles" branch (loaded, no error, no rows). */
const showingEmpty = $derived(list.items.length === 0 && list.loaded && list.error === null);

/** Whether the virtualizer branch is showing (it remounts per view). */
const hasRows = $derived(list.items.length > 0);

/** The pane's root — DOM focus inside it is the list owning the keyboard. */
let pane = $state<HTMLElement | null>(null);

/**
 * The focus request handed to the virtualizer: bumped to pull DOM focus onto
 * the listbox, 0 when a freshly mounted listbox must leave focus alone. Kept
 * here, not read off `selectionStore.focus` (which is `'list'` by default and
 * after any click, so it says nothing about where the keyboard actually is).
 */
let focusRequest = $state(0);
let focusRequests = 0;

/** A view switch away from a focused list whose listbox hasn't mounted yet. */
let pendingHandoff = false;

// Entering the list explicitly (`selectionStore.focusList`, e.g. Escape out of
// the sidebar) focuses the listbox — or the next one to mount, while rows are
// still loading.
$effect(() => {
    if (selectionStore.listFocusNonce > 0) {
        untrack(() => {
            pendingHandoff = !hasRows;
            focusRequests += 1;
            focusRequest = focusRequests;
        });
    }
});

// A view switch remounts the listbox (`{#key list}`), and the outgoing one
// takes DOM focus down with it. Before the DOM updates, check whether focus
// was inside this pane: if it was, the incoming listbox takes it over (held
// across a loading skeleton until rows mount); if it was anywhere else (the
// sidebar row or toggle that triggered the switch), it is left there.
$effect.pre(() => {
    void list;
    const rows = hasRows;
    untrack(() => {
        const active = document.activeElement;
        const owned = pane !== null && active !== null && pane.contains(active);
        const orphaned = active === null || active === document.body;
        const handoff = owned || (pendingHandoff && orphaned);
        pendingHandoff = handoff && !rows;
        if (handoff) {
            focusRequests += 1;
            focusRequest = focusRequests;
        } else {
            focusRequest = 0;
        }
    });
});

/**
 * The empty state is a focus holder: when the list empties while it owned the
 * keyboard (mark-all-read in an unread view, or `g f` → open an empty feed),
 * the virtualizer unmounts and focus would fall to `<body>`, stranding every
 * listbox shortcut. Moving focus here keeps the list "in focus" so navigation
 * resumes the moment rows return.
 */
let emptyRegion = $state<HTMLElement | null>(null);

$effect(() => {
    void selectionStore.listFocusNonce;
    if (showingEmpty && selectionStore.focus === 'list' && emptyRegion !== null) {
        emptyRegion.focus();
    }
});

/**
 * Each viewport report: remember this view's position, record the rows on
 * screen as seen, and — when mark-on-scroll is on — mark the seen rows that
 * have scrolled up past the top (each once per session). Untracked: the
 * virtualizer's own effect drives this, the bookkeeping is not reactive.
 */
function onViewport(report: ViewportReport): void {
    untrack(() => {
        const current = list;
        const items = current.items;
        articlesStore.positions.save(
            filterKey(current.filters),
            capturePosition(items, report.scrollTop, rowHeight),
        );
        const { seen, marked } = sessionFor(current);
        for (let index = report.first; index < report.end; index++) {
            seen.add(items[index].id);
        }
        if (!prefsStore.markOnScroll) {
            return;
        }
        for (let index = 0; index < report.first; index++) {
            const id = items[index].id;
            if (seen.has(id) && !marked.has(id)) {
                marked.add(id);
                void markReadOnScroll(id);
            }
        }
    });
}
</script>

<div class="article-list" bind:this={pane}>
    {#if list.items.length > 0}
        <!-- Stale-while-revalidate: keep showing the loaded rows even if a
             refetch just errored; surface the failure as a non-destructive
             banner instead of blanking a populated list. -->
        {#if list.error !== null}
            <p class="list-banner" role="alert">{listError}</p>
        {/if}
        <!-- Keyed on the list: a different view is a fresh scroll container,
             seated at that view's remembered position. It takes DOM focus
             only when the listbox it replaces had it (see focusRequest), so a
             remount never pulls focus out of the control that switched views. -->
        {#key list}
            <VirtualList
                items={list.items}
                {rowHeight}
                key={(article) => article.id}
                selectedIndex={selectionStore.selectedIndex}
                initialScrollTop={openingScrollTop()}
                label={t('list.label')}
                {activeDescendantId}
                focusNonce={focusRequest}
                onnearend={() => void list.loadMore()}
                onviewport={onViewport}
                onmove={moveSelection}
                onmenukey={openSelectedMenu}
            >
                {#snippet row(article, index)}
                    <ArticleRow
                        {article}
                        {index}
                        {rowHeight}
                        {compact}
                        setsize={list.items.length}
                        selected={article.id === selectionStore.selectedArticleId}
                        onselect={selectArticle}
                    />
                {/snippet}
            </VirtualList>
        {/key}
    {:else if !list.loaded}
        <div class="list-skeleton" aria-hidden="true">
            {#each SKELETON_ROWS as index (index)}
                <div class="skeleton-row" style="height: {rowHeight}px">
                    <div class="skeleton-row-main">
                        <Skeleton width="65%" height="0.9rem" />
                        {#if !compact}
                            <Skeleton width="92%" height="0.75rem" />
                        {/if}
                        <Skeleton width="35%" height="0.7rem" />
                    </div>
                    {#if !compact}
                        <Skeleton width="52px" height="52px" radius="var(--radius-md)" />
                    {/if}
                </div>
            {/each}
        </div>
        <span class="sr-only">{t('list.loading')}</span>
    {:else if list.error !== null}
        <!-- No rows to preserve: the first load itself failed. -->
        <p class="list-status error" role="alert">{listError}</p>
    {:else}
        <div class="empty-region" bind:this={emptyRegion} tabindex="-1">
            <EmptyState icon="inbox" title={t('list.empty')} />
        </div>
    {/if}
</div>

<style>
    .article-list {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--bg);
    }

    .list-status {
        padding: var(--space-4);
        font-size: 0.875rem;
        color: var(--fg-muted);
    }

    .list-status.error {
        color: var(--error-text);
    }

    /* Non-destructive refetch-failure strip above the retained rows. */
    .list-banner {
        flex: 0 0 auto;
        padding: var(--space-2) var(--space-4);
        margin: var(--space-2) var(--space-2) 0;
        border-radius: var(--radius-md);
        background: var(--error-bg);
        color: var(--error-text);
        font-size: var(--text-xs);
    }

    /* Focus holder for the empty list (see the emptyRegion effect). Fills the
       pane so the shared EmptyState centres, and stays outline-free since focus
       here is programmatic, never a visible tab landing. */
    .empty-region {
        flex: 1 1 auto;
        display: flex;
        min-height: 0;
        outline: none;
    }

    .list-skeleton {
        display: flex;
        flex-direction: column;
    }

    .skeleton-row {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: 0 var(--space-4);
        margin: 0 var(--space-2);
    }

    .skeleton-row-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
</style>
