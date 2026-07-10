<script lang="ts">
/**
 * One node of the sidebar folder tree. The header has two targets: a
 * disclosure chevron that expands/collapses, and the folder name, which
 * scopes the article list to the folder. Right-click opens the folder actions
 * menu (new subfolder / rename / mark read / delete). The header is a drop
 * target: dropping a dragged feed moves it into this folder (retag).
 */
import Icon from '$components/common/Icon.svelte';
import { contextMenu } from '$lib/actions/context-menu';
import { tooltip } from '$lib/actions/tooltip';
import { t } from '$lib/i18n';
import { runCommand, selectFolder } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedDnd } from '$lib/state/feed-dnd.svelte';
import { type FeedFolder, folderRowKey } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { allFolderPaths } from '$lib/state/folder-ops';
import type { MenuItem } from '$lib/state/menu.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { sidebarTreeStore } from '$lib/state/sidebar-tree.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import FeedItem from './FeedItem.svelte';
import FolderNode from './FolderNode.svelte';

let { folder, depth = 0 }: { folder: FeedFolder; depth?: number } = $props();

const collapsed = $derived(feedsStore.isFolderCollapsed(folder.path));
const unread = $derived(feedsStore.folderUnread(folder));
const selected = $derived(
    selectionStore.selectedFeedId === null && articlesStore.filters.feedTag === folder.path,
);
/** This folder's tree id + `aria-level`, and whether the keyboard cursor is on it. */
const rowId = $derived(folderRowKey(folder.path));
const level = $derived(depth + 1);
// Cursor ring only while the tree holds focus (see FeedItem) — no stray ring.
const isTreeActive = $derived(sidebarTreeStore.activeKey === rowId && sidebarTreeStore.focused);

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
    draft = folder.name;
    renaming = true;
}

function commitRename(): void {
    if (!renaming) return;
    renaming = false;
    const name = draft.trim();
    if (name.length === 0 || name === folder.name) return;
    const parent = folder.path.split('/').slice(0, -1).join('/');
    const newPath = parent === '' ? name : `${parent}/${name}`;
    // Renaming onto an existing sibling would silently *merge* the two folders
    // (irreversible). Refuse it and keep the old name — the tag rewrite that
    // renameFolder performs has no undo.
    if (newPath !== folder.path && allFolderPaths(feedsStore.feeds).includes(newPath)) {
        uiStore.showToast(t('folder.renameCollision', { name }), 'error');
        return;
    }
    void runCommand(() => feedsStore.renameFolder(folder.path, newPath));
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
function uniqueChildName(): string {
    const existing = folder.subfolders.map((sub) => sub.name);
    const base = t('folder.defaultName');
    if (!existing.includes(base)) return base;
    let n = 2;
    while (existing.includes(`${base} ${n}`)) n += 1;
    return `${base} ${n}`;
}

async function deleteFolder(): Promise<void> {
    // Only announce success when it actually succeeded (a partial/failed retag
    // toasts its own error via runCommand) — no more false "deleted" on failure.
    const result = await runCommand(() => feedsStore.deleteFolder(folder.path));
    if (result !== undefined) {
        uiStore.showToast(t('folder.deleted', { name: folder.name }), 'info');
    }
}

function folderMenu(): MenuItem[] {
    return [
        {
            id: 'newsub',
            labelKey: 'folder.menu.newSubfolder',
            onSelect: () => feedsStore.createFolder(`${folder.path}/${uniqueChildName()}`),
        },
        { id: 'rename', labelKey: 'folder.menu.rename', onSelect: startRename },
        {
            id: 'markread',
            labelKey: 'folder.menu.markRead',
            onSelect: () => void runCommand(() => feedsStore.markFolderRead(folder)),
        },
        {
            id: 'delete',
            labelKey: 'folder.menu.delete',
            danger: true,
            separatorBefore: true,
            onSelect: () => void deleteFolder(),
        },
    ];
}

// ─── folder as a move-into drop target ───────────────────────────────────────
let dropActive = $state(false);
let expandTimer: ReturnType<typeof setTimeout> | undefined;

function onFolderDragOver(event: DragEvent): void {
    if (feedDnd.draggingId === null) return;
    event.preventDefault();
    dropActive = true;
    // Hover over a collapsed folder briefly to auto-expand it.
    if (feedsStore.isFolderCollapsed(folder.path) && expandTimer === undefined) {
        expandTimer = setTimeout(() => {
            if (feedsStore.isFolderCollapsed(folder.path)) feedsStore.toggleFolder(folder.path);
            expandTimer = undefined;
        }, 600);
    }
}

function clearDrop(): void {
    dropActive = false;
    clearTimeout(expandTimer);
    expandTimer = undefined;
}

function onFolderDrop(event: DragEvent): void {
    const dragged = feedDnd.draggingId;
    clearDrop();
    if (dragged === null) return;
    event.preventDefault();
    void runCommand(() => feedsStore.moveFeedToFolder(dragged, folder.path));
    feedDnd.clear();
}
</script>

<li class="folder" role="none">
    <div
        id={rowId}
        class="folder-header"
        class:drop-active={dropActive}
        class:tree-active={isTreeActive}
        style:--depth={depth}
        ondragover={onFolderDragOver}
        ondragleave={clearDrop}
        ondrop={onFolderDrop}
        role="treeitem"
        tabindex="-1"
        aria-level={level}
        aria-expanded={!collapsed}
        aria-selected={selected}
        aria-label={folder.name}
        use:contextMenu={{ items: folderMenu, ariaLabel: folder.name }}
    >
        <button
            class="folder-disclosure"
            type="button"
            tabindex="-1"
            aria-expanded={!collapsed}
            aria-label={t('folder.toggle', { name: folder.name })}
            onclick={() => feedsStore.toggleFolder(folder.path)}
        >
            <span class="chevron" class:open={!collapsed}><Icon name="chevron" size={14} /></span>
        </button>
        {#if renaming}
            <input
                class="rename-input"
                spellcheck="false"
                bind:this={inputEl}
                bind:value={draft}
                onkeydown={onRenameKeydown}
                onblur={commitRename}
                aria-label={t('folder.menu.rename')}
            />
        {:else}
            <button
                class="folder-select"
                class:active={selected}
                tabindex="-1"
                aria-current={selected ? 'true' : undefined}
                ondblclick={startRename}
                onclick={(event) => {
                    // The second click of a rename double-click shouldn't re-scope
                    // the article list (event.detail === 2 is that second click).
                    if (event.detail < 2) selectFolder(folder.path);
                }}
            >
                <span class="folder-name truncate" use:tooltip={folder.name}>{folder.name}</span>
                {#if unread > 0}
                    <span class="folder-count" aria-hidden="true">{unread}</span>
                    <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
                {/if}
            </button>
        {/if}
    </div>
    {#if !collapsed}
        <ul class="folder-children" role="group">
            {#each folder.subfolders as subfolder (subfolder.path)}
                <FolderNode folder={subfolder} depth={depth + 1} />
            {/each}
            {#each folder.feeds as feed (feed.id)}
                <li class="folder-feed" role="none" style:--depth={depth + 1}>
                    <FeedItem
                        {feed}
                        parentPath={folder.path}
                        level={depth + 2}
                        unread={feedsStore.unreadFor(feed.id)}
                        selected={selectionStore.selectedFeedId === feed.id}
                        siblings={folder.feeds.map((candidate) => candidate.id)}
                        onselect={(feedId) => selectionStore.selectFeed(feedId)}
                        onedit={(feedId, section) => uiStore.openEditFeed(feedId, section)}
                    />
                </li>
            {/each}
            {#if folder.subfolders.length === 0 && folder.feeds.length === 0}
                <!-- A user-made folder that's still empty (e.g. just created, or
                     everything moved out) — say so rather than showing a blank gap. -->
                <li class="folder-empty" role="none" style:--depth={depth + 1}>
                    {t('folder.empty')}
                </li>
            {/if}
        </ul>
    {/if}
</li>

<style>
    .folder {
        list-style: none;
    }

    .folder-header {
        position: relative;
        display: flex;
        align-items: center;
        gap: 2px;
        padding-left: calc(var(--depth) * var(--space-4));
        border-radius: var(--radius-md);
    }

    /* A feed is being dragged over this folder — highlight it as a move target. */
    .folder-header.drop-active {
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 40%);
    }

    /* The keyboard-tree cursor ring (aria-activedescendant target). */
    .folder-header.tree-active {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    .folder-disclosure {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 22px;
        height: 28px;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--fg-subtle);
    }

    .folder-disclosure:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .chevron {
        display: inline-flex;
        transition: transform var(--dur-fast) var(--ease);
    }

    .chevron.open {
        transform: rotate(90deg);
    }

    .folder-select {
        position: relative;
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-2);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .folder-select:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .folder-select.active {
        background: var(--selected);
        color: var(--fg);
    }

    .folder-select.active::before {
        content: '';
        position: absolute;
        left: calc(-1 * var(--space-1));
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
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

    .folder-name {
        flex: 1 1 auto;
        min-width: 0;
    }

    .folder-count {
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

    .folder-select.active .folder-count {
        background: color-mix(in srgb, var(--accent), transparent 15%);
        color: var(--accent-fg);
    }

    .folder-children {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .folder-feed {
        padding-left: calc(var(--depth) * var(--space-4));
    }

    .folder-empty {
        padding: var(--space-1) var(--space-3);
        padding-left: calc(var(--depth) * var(--space-4) + var(--space-3));
        font-size: var(--text-sm);
        font-style: italic;
        color: var(--fg-subtle);
    }
</style>
