<script lang="ts">
/**
 * One subscription row: a select button (monogram, title) carrying the
 * backend-owned unread count, plus a health dot that opens the edit-feed
 * modal at its Health section. Right-click (or the context-menu key) opens
 * the feed actions menu (including Edit feed…); the row supports inline
 * rename and drag-to-reorder.
 */
import type { FeedDto } from '$lib/bindings';
import { contextMenu } from '$lib/actions/context-menu';
import { tooltip } from '$lib/actions/tooltip';
import { t } from '$lib/i18n';
import { markAllRead, toastCommandError } from '$lib/state/actions';
import { feedDnd, moveWithinGroup, rebuildGlobalOrder } from '$lib/state/feed-dnd.svelte';
import { feedRowKey } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { allFolderPaths } from '$lib/state/folder-ops';
import type { MenuItem } from '$lib/state/menu.svelte';
import { sidebarTreeStore } from '$lib/state/sidebar-tree.svelte';
import type { EditFeedSection } from '$lib/state/ui.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { copyText } from '$lib/utils/clipboard';
import { openExternal } from '$lib/utils/external';

let {
    feed,
    unread,
    selected,
    parentPath,
    level,
    siblings,
    onselect,
    onedit,
}: {
    feed: FeedDto;
    unread: number;
    selected: boolean;
    /** Enclosing folder path (`''` when ungrouped) — part of the tree row id. */
    parentPath: string;
    /** 1-based tree depth for `aria-level`. */
    level: number;
    /** The ordered ids of this feed's drag group; enables reordering when set. */
    siblings?: number[];
    onselect: (feedId: number) => void;
    onedit: (feedId: number, section?: EditFeedSection) => void;
} = $props();

const label = $derived(feed.title ?? feed.url);
/** This row's stable tree id (matches the flattened `VisibleRow.key`). */
const rowId = $derived(feedRowKey(parentPath, feed.id));
const isTreeActive = $derived(sidebarTreeStore.activeKey === rowId);

let dropTarget = $state(false);

// ─── inline rename ───────────────────────────────────────────────────────────
let renaming = $state(false);
let draft = $state('');
let inputEl = $state<HTMLInputElement>();

$effect(() => {
    if (renaming && inputEl !== undefined) {
        inputEl.focus();
        inputEl.select();
    }
});

function startRename(): void {
    draft = feed.title ?? '';
    renaming = true;
}

function commitRename(): void {
    if (!renaming) return;
    renaming = false;
    const title = draft.trim();
    void feedsStore.setFeedTitle(feed.id, title.length > 0 ? title : null);
}

function onRenameKeydown(event: KeyboardEvent): void {
    if (event.key === 'Enter') {
        event.preventDefault();
        commitRename();
    } else if (event.key === 'Escape') {
        event.preventDefault();
        renaming = false;
    }
}

// ─── context menu ────────────────────────────────────────────────────────────
async function copyUrl(): Promise<void> {
    const ok = await copyText(feed.url);
    uiStore.showToast(t(ok ? 'sidebar.urlCopied' : 'app.error.internal'), ok ? 'success' : 'error');
}

async function unsubscribe(): Promise<void> {
    const result = await feedsStore.removeFeed(feed.id);
    if (result.status === 'error') {
        toastCommandError(result.error);
        return;
    }
    uiStore.showToast(t('feed.unsubscribed', { name: label }), 'info');
}

function feedMenu(): MenuItem[] {
    const paused = feed.status === 'paused';
    const moveTo: MenuItem[] = allFolderPaths(feedsStore.feeds).map((path) => ({
        id: `move:${path}`,
        label: path,
        onSelect: () => void feedsStore.moveFeedToFolder(feed.id, path),
    }));
    moveTo.push({
        id: 'ungroup',
        labelKey: 'feed.menu.ungroup',
        separatorBefore: moveTo.length > 0,
        onSelect: () => void feedsStore.ungroupFeed(feed.id),
    });
    return [
        { id: 'open', labelKey: 'feed.menu.openSite', onSelect: () => void openExternal(feed.url) },
        {
            id: 'refresh',
            labelKey: 'feed.menu.refresh',
            onSelect: () => void feedsStore.refreshFeed(feed.id),
        },
        { id: 'rename', labelKey: 'feed.menu.rename', onSelect: startRename },
        {
            id: 'markread',
            labelKey: 'feed.menu.markAllRead',
            onSelect: () => void markAllRead(feed.id),
        },
        { id: 'move', labelKey: 'feed.menu.moveToFolder', children: moveTo },
        {
            id: 'edit',
            labelKey: 'feed.menu.edit',
            separatorBefore: true,
            onSelect: () => onedit(feed.id),
        },
        { id: 'copy', labelKey: 'feed.menu.copyUrl', onSelect: () => void copyUrl() },
        {
            id: 'status',
            labelKey: paused ? 'feed.menu.resume' : 'feed.menu.pause',
            onSelect: () => void feedsStore.setFeedStatus(feed.id, paused ? 'active' : 'paused'),
        },
        {
            id: 'unsub',
            labelKey: 'feed.menu.unsubscribe',
            danger: true,
            separatorBefore: true,
            onSelect: () => void unsubscribe(),
        },
    ];
}

// ─── drag reorder ────────────────────────────────────────────────────────────
function isValidDrag(): boolean {
    const dragging = feedDnd.draggingId;
    return (
        siblings !== undefined &&
        dragging !== null &&
        dragging !== feed.id &&
        siblings.includes(dragging)
    );
}

function onDragOver(event: DragEvent): void {
    if (isValidDrag()) {
        event.preventDefault(); // permit the drop
        dropTarget = true;
    }
}

function onDrop(event: DragEvent): void {
    dropTarget = false;
    const dragged = feedDnd.draggingId;
    if (siblings === undefined || dragged === null || !isValidDrag()) {
        return;
    }
    event.preventDefault();
    const newGroup = moveWithinGroup(siblings, dragged, feed.id);
    const globalOrder = rebuildGlobalOrder(
        feedsStore.feeds.map((candidate) => candidate.id),
        newGroup,
    );
    void feedsStore.reorderFeeds(globalOrder);
    feedDnd.clear();
}

/** A stable per-feed hue for the monogram, derived from the title. */
function hue(text: string): number {
    let hash = 0;
    for (let i = 0; i < text.length; i += 1) {
        hash = (hash * 31 + text.charCodeAt(i)) % 360;
    }
    return hash;
}
</script>

<div
    id={rowId}
    class="feed-item"
    class:active={selected}
    class:tree-active={isTreeActive}
    class:unhealthy={feed.status !== 'active'}
    class:drop-target={dropTarget}
    class:dragging={feedDnd.draggingId === feed.id}
    draggable={siblings !== undefined && !renaming}
    ondragstart={() => feedDnd.start(feed.id)}
    ondragover={onDragOver}
    ondragleave={() => {
        dropTarget = false;
    }}
    ondrop={onDrop}
    ondragend={() => feedDnd.clear()}
    role="treeitem"
    tabindex="-1"
    aria-level={level}
    aria-selected={selected}
    aria-label={label}
    use:contextMenu={{ items: feedMenu, ariaLabel: label }}
>
    {#if renaming}
        <input
            class="rename-input"
            bind:this={inputEl}
            bind:value={draft}
            onkeydown={onRenameKeydown}
            onblur={commitRename}
            aria-label={t('feed.menu.rename')}
        />
    {:else}
        <button
            class="feed-select"
            tabindex="-1"
            aria-current={selected ? 'true' : undefined}
            ondblclick={startRename}
            onclick={() => onselect(feed.id)}
        >
            <span class="feed-mono" style:--mono-hue={hue(label)} aria-hidden="true"
                >{label.slice(0, 1).toUpperCase()}</span
            >
            <span class="feed-title truncate" use:tooltip={label}>{label}</span>
        </button>
        {#if unread > 0}
            <span class="unread-badge" aria-hidden="true">{unread}</span>
            <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
        {/if}
        <button
            class="feed-health"
            type="button"
            tabindex="-1"
            aria-label={t('feedHealth.open', { name: label })}
            onclick={() => onedit(feed.id, 'health')}
        >
            <span class="health-dot health-{feed.status}" aria-hidden="true"></span>
        </button>
    {/if}
</div>

<style>
    .feed-item {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-1);
        padding-right: var(--space-1);
        border-radius: var(--radius-md);
        transition: background var(--dur-fast) var(--ease);
    }

    .feed-item:hover {
        background: var(--hover);
    }

    .feed-item.active {
        background: var(--selected);
    }

    /* The keyboard-tree cursor ring (aria-activedescendant target). */
    .feed-item.tree-active {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    .feed-item[draggable='true'] {
        cursor: grab;
    }

    .feed-item.dragging {
        opacity: 0.5;
    }

    /* A line where the dragged feed would land. */
    .feed-item.drop-target {
        box-shadow: inset 0 2px 0 var(--accent);
    }

    .feed-item.active::before {
        content: '';
        position: absolute;
        left: 0;
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .feed-select {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-2);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
    }

    .feed-item.active .feed-select {
        color: var(--fg);
    }

    .rename-input {
        flex: 1 1 auto;
        min-width: 0;
        margin: var(--space-1) var(--space-2);
        padding: 2px var(--space-2);
        border-radius: var(--radius-sm);
        background: var(--surface-inset);
        color: var(--fg);
        border: 1px solid color-mix(in srgb, var(--accent), transparent 40%);
        font-size: var(--text-md);
    }

    .rename-input:focus-visible {
        outline: none;
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 82%);
    }

    .feed-mono {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 20px;
        height: 20px;
        border-radius: var(--radius-sm);
        font-size: 0.68rem;
        font-weight: 700;
        color: hsl(var(--mono-hue) 60% 80%);
        background: hsl(var(--mono-hue) 45% 34% / 0.5);
        box-shadow: inset 0 0 0 1px hsl(var(--mono-hue) 50% 58% / 0.3);
    }

    .feed-title {
        flex: 1 1 auto;
        min-width: 0;
    }

    .feed-item.unhealthy .feed-title {
        color: var(--fg-subtle);
    }

    .unread-badge {
        flex: 0 0 auto;
        min-width: 1.35rem;
        padding: 1px var(--space-1);
        border-radius: var(--radius-pill);
        background: var(--surface-raised);
        color: var(--fg-muted);
        font-size: 0.7rem;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        text-align: center;
        line-height: 1.5;
    }

    .feed-item.active .unread-badge {
        background: color-mix(in srgb, var(--accent), transparent 15%);
        color: var(--accent-fg);
    }

    .feed-health {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 1.25rem;
        height: 1.25rem;
        border-radius: var(--radius-sm);
        background: transparent;
        opacity: 0;
        transition: opacity var(--dur-fast) var(--ease);
    }

    /* Healthy feeds hide the dot until hover; unhealthy ones always show it. */
    .feed-item:hover .feed-health,
    .feed-item.unhealthy .feed-health {
        opacity: 1;
    }

    .feed-health:hover {
        background: var(--active);
    }

    .health-dot {
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 50%;
        background: var(--fg-subtle);
    }

    .health-active {
        background: var(--success);
    }

    .health-paused {
        background: var(--warning);
    }

    .health-dead {
        background: var(--error);
    }
</style>
