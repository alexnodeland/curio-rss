<script lang="ts">
/**
 * The bulk bar: floats over the bottom of the article list while two or more
 * rows are selected, and runs one batch command per action over the whole
 * selection (`setFlagForSelection` — one IPC call, one transaction, with an
 * Undo toast). Star, read-later and archive take the flag off inside their
 * own view (everything listed there has it) and put it on elsewhere — the
 * same direction `s`/`l`/`e` take over a multi-selection, so each tooltip's
 * key does exactly what its button does.
 */
import type { ArticleFlagDto } from '$lib/bindings';
import Icon, { type IconName } from '$components/common/Icon.svelte';
import { tooltip } from '$lib/actions/tooltip';
import { type MessageKey, t } from '$lib/i18n';
import { keyLabel, shortcutKbd } from '$lib/keyboard/labels';
import type { ShortcutId } from '$lib/keyboard/registry';
import { bulkValue, setFlagForSelection } from '$lib/state/actions';
import { selectionStore } from '$lib/state/selection.svelte';

interface BulkButton {
    readonly id: string;
    readonly flag: ArticleFlagDto;
    readonly value: boolean;
    readonly label: MessageKey;
    readonly icon: IconName;
    /** The single-article shortcut that runs this over a multi-selection. */
    readonly shortcut?: ShortcutId;
}

/** The buttons for the current view (see the header for the directions). */
const buttons = $derived.by((): BulkButton[] => {
    const star = bulkValue('starred');
    const later = bulkValue('read_later');
    const archive = bulkValue('archived');
    return [
        {
            id: 'read',
            flag: 'read',
            value: true,
            label: 'reader.action.markRead',
            icon: 'check',
            shortcut: 'article.toggleRead',
        },
        {
            id: 'unread',
            flag: 'read',
            value: false,
            label: 'reader.action.markUnread',
            icon: 'circle-dot',
        },
        {
            id: 'star',
            flag: 'starred',
            value: star,
            label: star ? 'reader.action.star' : 'reader.action.unstar',
            icon: star ? 'star' : 'star-filled',
            shortcut: 'article.toggleStar',
        },
        {
            id: 'readLater',
            flag: 'read_later',
            value: later,
            label: later ? 'reader.action.readLater' : 'reader.action.readLaterRemove',
            icon: 'bookmark',
            shortcut: 'article.toggleReadLater',
        },
        {
            id: 'archive',
            flag: 'archived',
            value: archive,
            label: archive ? 'reader.action.archive' : 'reader.action.unarchive',
            icon: 'archive',
            shortcut: 'article.toggleArchived',
        },
    ];
});

const count = $derived(selectionStore.selectedIds.length);
</script>

<div class="bulk-bar" role="toolbar" aria-label={t('bulk.toolbar')} data-testid="bulk-bar">
    <span class="bulk-count" aria-live="polite">{t('bulk.count', { count })}</span>
    <span class="bulk-sep" aria-hidden="true"></span>
    {#each buttons as button (button.id)}
        <button
            class="bulk-tool"
            type="button"
            data-action={button.id}
            aria-label={t(button.label)}
            use:tooltip={{
                text: t(button.label),
                kbd: button.shortcut === undefined ? undefined : shortcutKbd(button.shortcut),
            }}
            onclick={() => void setFlagForSelection(button.flag, button.value)}
        >
            <Icon name={button.icon} />
        </button>
    {/each}
    <span class="bulk-sep" aria-hidden="true"></span>
    <button
        class="bulk-tool"
        type="button"
        data-action="clear"
        aria-label={t('bulk.clear')}
        use:tooltip={{ text: t('bulk.clear'), kbd: keyLabel('Escape') }}
        onclick={() => selectionStore.clearMulti()}
    >
        <Icon name="close" />
    </button>
</div>

<style>
    /* Floats over the list's bottom edge so rows never shift when it appears
       (a Shift-click must not move the row under the pointer). */
    .bulk-bar {
        position: absolute;
        left: 50%;
        bottom: var(--space-4);
        transform: translateX(-50%);
        z-index: 5;
        display: flex;
        align-items: center;
        gap: var(--space-1);
        max-width: calc(100% - 2 * var(--space-3));
        padding: var(--space-1) var(--space-2) var(--space-1) var(--space-4);
        border-radius: var(--radius-pill);
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-lg);
    }

    .bulk-count {
        flex: 0 1 auto;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
        font-size: var(--text-sm);
        font-weight: 600;
        color: var(--fg);
        font-variant-numeric: tabular-nums;
    }

    .bulk-sep {
        flex: 0 0 auto;
        width: 1px;
        height: 18px;
        margin: 0 var(--space-1);
        background: var(--hairline);
    }

    .bulk-tool {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        width: 34px;
        height: 34px;
        border-radius: var(--radius-pill);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid transparent;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .bulk-tool:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .bulk-tool:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: 1px;
    }
</style>
