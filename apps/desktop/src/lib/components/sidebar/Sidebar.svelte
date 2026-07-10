<script lang="ts">
/**
 * The left pane: a brand/action header, the built-in views (all / starred /
 * read later), and the subscription list, with backend-owned unread counts
 * (`get_unread_counts` — never client badge math). Clicks re-scope the
 * backend-owned filters; the article list reacts through the stores.
 */
import Icon, { type IconName } from '$components/common/Icon.svelte';
import Skeleton from '$components/common/Skeleton.svelte';
import { tooltip } from '$lib/actions/tooltip';
import { type MessageKey, t } from '$lib/i18n';
import {
    activeView,
    importFromFile,
    refreshAll,
    runCommand,
    selectFolder,
    selectView,
    type ViewId,
} from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedDnd, moveWithinGroup, rebuildGlobalOrder } from '$lib/state/feed-dnd.svelte';
import { buildFeedTree, flattenVisibleTree, type VisibleRow } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { folderPathsForTags } from '$lib/state/folder-ops';
import { selectionStore } from '$lib/state/selection.svelte';
import { sidebarTreeStore } from '$lib/state/sidebar-tree.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import { treeKeyAction } from '$lib/utils/tree-nav';
import { untrack } from 'svelte';
import FeedItem from './FeedItem.svelte';
import FolderNode from './FolderNode.svelte';

const VIEWS: readonly { id: ViewId; label: MessageKey; icon: IconName }[] = [
    { id: 'all', label: 'view.all', icon: 'inbox' },
    { id: 'starred', label: 'view.starred', icon: 'star' },
    { id: 'readLater', label: 'view.readLater', icon: 'bookmark' },
    { id: 'archived', label: 'view.archived', icon: 'archive' },
];

/** Bar widths for the subscription-list loading skeleton (percent). */
const SIDEBAR_SKELETON_WIDTHS = ['82%', '64%', '90%', '71%', '58%'];

// Prime the feed + unread-count queries during init, NOT from the template:
// state created inside the render reaction is excluded from that reaction's
// dependencies, so a fetch kicked off by a template read would never
// re-render on completion (the error/loaded transition would be lost).
feedsStore.prime();

// The folder tree derived from each feed's `/`-path tags; feeds with no tags
// fall into `ungrouped` and render flat below the folders.
const feedTree = $derived(buildFeedTree(feedsStore.feeds, feedsStore.pendingPaths));

// The flattened, depth-annotated visible rows the arrow keys walk. Reads
// collapse through feedsStore (a tracked SvelteSet) so it re-flattens on toggle
// or startup load — never a snapshot (see the WP7 reloadSet note).
const rows = $derived(flattenVisibleTree(feedTree, (path) => feedsStore.isFolderCollapsed(path)));
const activeIndex = $derived(rows.findIndex((row) => row.key === sidebarTreeStore.activeKey));
// Only point aria-activedescendant at a row that still exists.
const activeDescendant = $derived(
    activeIndex >= 0 ? (sidebarTreeStore.activeKey ?? undefined) : undefined,
);

let treeEl: HTMLUListElement | undefined = $state();

// Where the cursor lands on entry when it has none: the feed currently open in
// the list (so ← back into the sidebar returns you to where you are, not the
// top), falling back to the first row when nothing is selected.
function entrySeatKey(): string {
    const selectedId = selectionStore.selectedFeedId;
    const seat =
        selectedId === null
            ? -1
            : rows.findIndex((row) => row.kind !== 'folder' && row.id === selectedId);
    return rows[seat >= 0 ? seat : 0].key;
}

// When `g f` (or a native-menu "Go to feeds") hands the keyboard to the
// sidebar, move DOM focus to the tree and seat the cursor if it has none (or a
// stale one). Gated on the entry *nonce* only: seating the cursor and reading
// rows/activeIndex happen inside untrack, so a later activeIndex change never
// re-fires this and re-grabs focus — which is what made clicking out of the
// sidebar snap straight back.
$effect(() => {
    void selectionStore.sidebarFocusNonce;
    untrack(() => {
        if (selectionStore.focus !== 'sidebar' || treeEl === undefined || rows.length === 0) {
            return;
        }
        if (activeIndex < 0) {
            sidebarTreeStore.activeKey = entrySeatKey();
        }
        treeEl.focus();
    });
});

// Keep the arrow-key cursor visible: when the active row changes, scroll it
// into view within the sidebar's own scroll container (jsdom lacks
// scrollIntoView, hence the optional call).
$effect(() => {
    const key = sidebarTreeStore.activeKey;
    if (key === null) {
        return;
    }
    document.getElementById(key)?.scrollIntoView?.({ block: 'nearest' });
});

/** Commits the row under the cursor (Enter/Space), then hands focus back. */
function activateRow(index: number): void {
    const row = rows[index];
    if (row === undefined) {
        return;
    }
    if (row.kind === 'folder') {
        selectFolder(row.path);
    } else {
        selectionStore.selectFeed(row.id);
    }
    // Hand focus to the article listbox (it takes DOM focus on the nonce), so
    // the keyboard lands in the list rather than on <body>.
    selectionStore.focusList();
    treeEl?.blur();
}

/**
 * Reorders the feed under the cursor by one slot within its own drag group
 * (the feeds sharing its parent folder). Returns false when the row isn't a
 * feed or the move would fall off the group's end. The cursor rides along —
 * the row key is `feed:<parentPath>:<id>`, unchanged by a within-group move.
 */
function reorderFeedRow(index: number, direction: 1 | -1): boolean {
    const row = rows[index];
    if (row === undefined || row.kind !== 'feed') {
        return false;
    }
    const rowParent = (r: VisibleRow): string =>
        r.key.slice('feed:'.length, r.key.lastIndexOf(':'));
    const parentPath = rowParent(row);
    const group = rows
        .filter((r): r is Extract<VisibleRow, { kind: 'feed' }> => r.kind === 'feed')
        .filter((r) => rowParent(r) === parentPath)
        .map((r) => r.id);
    const at = group.indexOf(row.id);
    const target = group[at + direction];
    if (target === undefined) {
        return false;
    }
    const newGroup = moveWithinGroup(group, row.id, target);
    const globalOrder = rebuildGlobalOrder(
        feedsStore.feeds.map((candidate) => candidate.id),
        newGroup,
    );
    void runCommand(() => feedsStore.reorderFeeds(globalOrder));
    return true;
}

/**
 * Alt+↑/↓ reorders the feed under the cursor within its group (keyboard DnD),
 * distinct from a plain arrow's cursor move. Returns whether it consumed the
 * event (only when a reorder actually happened — an end-of-group Alt+arrow
 * falls through so nothing surprising occurs).
 */
function tryKeyboardReorder(event: KeyboardEvent, cursorIndex: number): boolean {
    if (!event.altKey || (event.key !== 'ArrowUp' && event.key !== 'ArrowDown')) {
        return false;
    }
    const moved = reorderFeedRow(cursorIndex, event.key === 'ArrowDown' ? 1 : -1);
    if (moved) {
        event.preventDefault();
        event.stopPropagation();
    }
    return moved;
}

/** Applies a resolved (non-`none`) tree key action — move, toggle, or activate. */
function applyTreeResult(
    result: Exclude<ReturnType<typeof treeKeyAction>, { type: 'none' }>,
): void {
    if (result.type === 'move') {
        sidebarTreeStore.activeKey = rows[result.index]?.key ?? null;
    } else if (result.type === 'toggle') {
        feedsStore.toggleFolder(result.path);
    } else {
        activateRow(result.index);
    }
}

function onTreeKeydown(event: KeyboardEvent): void {
    if (event.key === 'Escape') {
        event.preventDefault();
        event.stopPropagation();
        selectionStore.focusList();
        treeEl?.blur();
        return;
    }
    // Read the cursor index straight from the store state, not the `activeIndex`
    // $derived: a fast burst of arrow keys fires several keydowns before Svelte
    // flushes the derived, so the derived stays stale and every press computes
    // the same next row — the cursor sticks. The raw `activeKey` is always
    // current, so this advances smoothly however fast the keys arrive.
    const cursorIndex = rows.findIndex((row) => row.key === sidebarTreeStore.activeKey);
    if (tryKeyboardReorder(event, cursorIndex)) {
        return;
    }
    const result = treeKeyAction(rows, cursorIndex, event.key);
    if (result.type === 'none') {
        // → at a tree dead-end (a leaf feed, or an already-expanded end) drills
        // rightward into the article list — the same "move right" that carries
        // the list into the reader — so → means one thing everywhere. It
        // activates the row first so the list shows the feed under the cursor,
        // not a stale selection.
        if (event.key === 'ArrowRight' && cursorIndex >= 0) {
            event.preventDefault();
            event.stopPropagation();
            activateRow(cursorIndex);
        }
        return;
    }
    event.preventDefault();
    event.stopPropagation();
    applyTreeResult(result);
}

function currentView(): ViewId | null {
    if (selectionStore.selectedFeedId !== null) {
        return null;
    }
    return activeView(articlesStore.filters);
}

function feedsError(): string {
    const failure = feedsStore.error;
    return failure === null ? '' : commandErrorMessage(failure);
}

// The dragged feed sits in a folder ⇒ offer the "remove from folder" drop zone.
// (Dragging an already-ungrouped feed onto it would be a no-op, so it stays hidden.)
const draggedInFolder = $derived.by(() => {
    const id = feedDnd.draggingId;
    if (id === null) {
        return false;
    }
    const dragged = feedsStore.feeds.find((candidate) => candidate.id === id);
    return dragged !== undefined && folderPathsForTags(dragged.tags).length > 0;
});

let ungroupDropActive = $state(false);

/** Drops a folder-feed onto the ungroup zone — the one-way trip back out of a folder. */
function onUngroupDrop(event: DragEvent): void {
    const dragged = feedDnd.draggingId;
    ungroupDropActive = false;
    if (dragged === null) {
        return;
    }
    event.preventDefault();
    void runCommand(() => feedsStore.ungroupFeed(dragged));
    feedDnd.clear();
}

/**
 * Edge auto-scroll while dragging: nudges the sidebar's own scroll container
 * when the pointer nears its top/bottom, so a feed can be dragged to a folder
 * currently off-screen. Rides the bubbling dragover from the rows below.
 */
function onScrollAreaDragOver(event: DragEvent): void {
    if (feedDnd.draggingId === null) {
        return;
    }
    const el = event.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const EDGE = 48;
    if (event.clientY < rect.top + EDGE) {
        el.scrollBy({ top: -14 });
    } else if (event.clientY > rect.bottom - EDGE) {
        el.scrollBy({ top: 14 });
    }
}

/** Creates a new top-level folder with a unique default name (rename inline). */
function newFolder(): void {
    const existing = feedTree.folders.map((folder) => folder.name);
    const base = t('folder.defaultName');
    let name = base;
    let n = 2;
    while (existing.includes(name)) {
        name = `${base} ${n}`;
        n += 1;
    }
    feedsStore.createFolder(name);
}
</script>

<nav class="sidebar" aria-label={t('sidebar.label')}>
    <div class="sidebar-brand">
        <div class="brand">
            <span class="brand-mark" aria-hidden="true"><Icon name="sparkle" size={16} /></span>
            <span class="brand-name">Curio</span>
        </div>
        <div class="sidebar-toolbar" role="toolbar" aria-label={t('sidebar.label')}>
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.addFeed')}
                use:tooltip={{ text: t('toolbar.addFeed'), kbd: 'A' }}
                onclick={() => uiStore.openModal('add-feed')}><Icon name="plus" /></button
            >
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.refreshAll')}
                use:tooltip={{ text: t('toolbar.refreshAll'), kbd: 'R' }}
                aria-busy={feedsStore.refreshing}
                disabled={feedsStore.refreshing}
                onclick={() => refreshAll()}><Icon name="refresh" /></button
            >
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.settings')}
                use:tooltip={{ text: t('toolbar.settings'), kbd: ',' }}
                onclick={() => uiStore.openModal('settings')}><Icon name="settings" /></button
            >
        </div>
    </div>

    <!-- ondragover drives edge auto-scroll from the bubbling drag over rows below;
         the scroll container isn't itself an interactive target. -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="sidebar-scroll" ondragover={onScrollAreaDragOver}>
        <section class="sidebar-group">
            <ul class="sidebar-list">
                {#each VIEWS as view (view.id)}
                    <li>
                        <button
                            class="view-item"
                            class:active={currentView() === view.id}
                            aria-current={currentView() === view.id ? 'true' : undefined}
                            onclick={() => selectView(view.id)}
                        >
                            <Icon name={view.icon} size={17} />
                            <span class="truncate">{t(view.label)}</span>
                            {#if view.id === 'all' && feedsStore.unreadTotal > 0}
                                <span class="view-count" aria-hidden="true"
                                    >{feedsStore.unreadTotal}</span
                                >
                                <span class="sr-only"
                                    >{t('shell.unread.count', { count: feedsStore.unreadTotal })}</span
                                >
                            {/if}
                        </button>
                    </li>
                {/each}
            </ul>
        </section>

        <section class="sidebar-group sidebar-feeds">
            <div class="sidebar-feeds-head">
                <h2 class="sidebar-heading">{t('sidebar.feeds')}</h2>
                <button
                    class="chrome-button chrome-button-sm"
                    type="button"
                    aria-label={t('folder.menu.newFolder')}
                    use:tooltip={t('folder.menu.newFolder')}
                    onclick={newFolder}><Icon name="plus" size={14} /></button
                >
            </div>
            {#if feedsStore.error !== null}
                <p class="sidebar-status error" role="alert">{feedsError()}</p>
            {:else if !feedsStore.loaded}
                <div class="sidebar-skeleton" aria-hidden="true">
                    {#each SIDEBAR_SKELETON_WIDTHS as width (width)}
                        <Skeleton {width} height="0.85rem" />
                    {/each}
                </div>
                <span class="sr-only">{t('shell.feeds.loading')}</span>
            {:else if feedsStore.feeds.length === 0}
                <div class="sidebar-empty">
                    <div class="sidebar-empty-mark" aria-hidden="true">
                        <Icon name="rss" size={22} strokeWidth={1.5} />
                    </div>
                    <p class="sidebar-empty-title">{t('shell.feeds.empty')}</p>
                    <p class="sidebar-empty-hint">{t('emptyState.hint')}</p>
                    <div class="sidebar-empty-actions">
                        <button
                            class="empty-action empty-action-primary"
                            type="button"
                            onclick={() => uiStore.openModal('add-feed')}
                        >
                            {t('emptyState.addFeed')}
                        </button>
                        <button
                            class="empty-action"
                            type="button"
                            onclick={() => void importFromFile('opml')}
                        >
                            {t('emptyState.importOpml')}
                        </button>
                    </div>
                </div>
            {:else}
                <ul
                    class="sidebar-list"
                    role="tree"
                    tabindex="0"
                    aria-label={t('sidebar.feeds')}
                    aria-activedescendant={activeDescendant}
                    bind:this={treeEl}
                    onkeydown={onTreeKeydown}
                    onfocus={() => {
                        sidebarTreeStore.focused = true;
                    }}
                    onblur={() => {
                        // Losing focus hands the keyboard back to the list — without
                        // handing `focus` off, it stayed 'sidebar' and the window
                        // keydown handler swallowed every shortcut (the global
                        // deadlock). Only the focus *flag* clears (so the cursor ring
                        // stops painting); the cursor *position* is kept so ← back
                        // into the sidebar returns exactly where you were.
                        sidebarTreeStore.focused = false;
                        if (selectionStore.focus === 'sidebar') {
                            selectionStore.focus = 'list';
                        }
                    }}
                >
                    {#each feedTree.folders as folder (folder.path)}
                        <FolderNode {folder} depth={0} />
                    {/each}
                    {#each feedTree.ungrouped as feed (feed.id)}
                        <li role="none">
                            <FeedItem
                                {feed}
                                parentPath=""
                                level={1}
                                unread={feedsStore.unreadFor(feed.id)}
                                selected={selectionStore.selectedFeedId === feed.id}
                                siblings={feedTree.ungrouped.map((candidate) => candidate.id)}
                                onselect={(feedId) => selectionStore.selectFeed(feedId)}
                                onedit={(feedId, section) => uiStore.openEditFeed(feedId, section)}
                            />
                        </li>
                    {/each}
                    {#if draggedInFolder}
                        <!-- Pointer-only ungroup target (keyboard/menu users have
                             "Remove from folder"); DnD out of a folder was one-way. -->
                        <li aria-hidden="true">
                            <!-- svelte-ignore a11y_no_static_element_interactions -->
                            <div
                                class="ungroup-zone"
                                class:drop-active={ungroupDropActive}
                                ondragover={(event) => {
                                    event.preventDefault();
                                    ungroupDropActive = true;
                                }}
                                ondragleave={() => {
                                    ungroupDropActive = false;
                                }}
                                ondrop={onUngroupDrop}
                            >
                                {t('feed.menu.ungroup')}
                            </div>
                        </li>
                    {/if}
                </ul>
            {/if}
        </section>
    </div>
</nav>

<style>
    .sidebar {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--surface-chrome);
        border-right: 1px solid var(--hairline);
    }

    .sidebar-brand {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2);
        height: var(--header-height);
        padding: 0 var(--space-2) 0 var(--space-4);
        border-bottom: 1px solid var(--hairline);
    }

    .brand {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        min-width: 0;
    }

    .brand-mark {
        display: grid;
        place-items: center;
        width: 26px;
        height: 26px;
        border-radius: var(--radius-md);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 72%);
    }

    .brand-name {
        font-size: var(--text-lg);
        font-weight: 660;
        letter-spacing: var(--tracking-tight);
        color: var(--fg);
    }

    .sidebar-toolbar {
        display: flex;
        align-items: center;
        gap: 2px;
    }

    .chrome-button {
        display: inline-grid;
        place-items: center;
        width: 30px;
        height: 30px;
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .chrome-button:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .sidebar-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        padding: var(--space-3) var(--space-2) var(--space-4);
        display: flex;
        flex-direction: column;
        gap: var(--space-5);
    }

    .sidebar-group {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .sidebar-feeds-head {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding-right: var(--space-1);
        margin-bottom: var(--space-1);
    }

    .sidebar-heading {
        padding: 0 var(--space-3);
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .chrome-button-sm {
        width: 24px;
        height: 24px;
        opacity: 0;
        transition:
            opacity var(--dur-fast) var(--ease),
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .sidebar-feeds:hover .chrome-button-sm,
    .chrome-button-sm:focus-visible {
        opacity: 1;
    }

    .sidebar-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    /* The tree container itself is the tab stop; the cursor ring lives on the
       active row (aria-activedescendant), so suppress the container outline. */
    .sidebar-list[role='tree']:focus,
    .sidebar-list[role='tree']:focus-visible {
        outline: none;
    }

    /* A dashed drop target that appears only while a folder-feed is dragged, to
       pull it back out to the top level (the DnD mirror of "Remove from folder"). */
    .ungroup-zone {
        margin: var(--space-1) var(--space-2) 0;
        padding: var(--space-2) var(--space-3);
        border: 1px dashed var(--hairline-strong);
        border-radius: var(--radius-md);
        color: var(--fg-subtle);
        font-size: var(--text-sm);
        text-align: center;
        transition:
            background var(--dur-fast) var(--ease),
            border-color var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .ungroup-zone.drop-active {
        background: var(--selected);
        border-color: color-mix(in srgb, var(--accent), transparent 40%);
        color: var(--fg);
    }

    .sidebar-status {
        padding: var(--space-2) var(--space-3);
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .sidebar-status.error {
        color: var(--error-text);
    }

    .sidebar-skeleton {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-3);
    }

    .sidebar-empty {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-6) var(--space-4);
        text-align: center;
    }

    .sidebar-empty-mark {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 48px;
        height: 48px;
        border-radius: var(--radius-xl);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .sidebar-empty-title {
        font-size: var(--text-md);
        font-weight: 560;
        color: var(--fg);
    }

    .sidebar-empty-hint {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.45;
    }

    .sidebar-empty-actions {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        width: 100%;
        margin-top: var(--space-2);
    }

    .empty-action {
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        font-weight: 500;
        cursor: pointer;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .empty-action:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .empty-action-primary {
        background: var(--accent);
        color: var(--accent-fg);
        border-color: transparent;
    }

    .empty-action-primary:hover {
        background: var(--accent-hover);
        color: var(--accent-fg);
    }

    .view-item {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-2);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .view-item :global(svg) {
        flex: 0 0 auto;
        color: var(--fg-subtle);
    }

    .view-item:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .view-item.active {
        background: var(--selected);
        color: var(--fg);
    }

    .view-item.active :global(svg) {
        color: var(--accent);
    }

    .view-item.active::before {
        content: '';
        position: absolute;
        left: 0;
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .view-count {
        margin-left: auto;
        min-width: 1.35rem;
        padding: 1px var(--space-1);
        border-radius: var(--radius-pill);
        background: var(--accent);
        color: var(--accent-fg);
        font-size: 0.7rem;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        text-align: center;
        line-height: 1.5;
    }
</style>
