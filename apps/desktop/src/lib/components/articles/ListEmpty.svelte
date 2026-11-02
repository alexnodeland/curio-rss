<script lang="ts">
/**
 * The empty article list, worded for the view it is in (see
 * `listEmptyState`): an unread-only view is "all caught up" and offers the
 * read rows back, Starred / Read later / Archived say how an article gets
 * there (naming the registry's key), and a feed whose last fetch failed says
 * why — from its classified failure, in the active locale — with a Retry and
 * a way into its health panel. Shared by the row list and both home layouts.
 */
import EmptyState from '$components/common/EmptyState.svelte';
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import { shortcutKbd } from '$lib/keyboard/labels';
import { refreshFeed } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import {
    type ListEmptyAction,
    feedErrorMessage,
    listEmptyState,
    splitAtKey,
} from '$lib/utils/empty-states';

const feed = $derived.by(() => {
    const feedId = articlesStore.filters.feedId;
    return feedId === null
        ? null
        : (feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null);
});

const empty = $derived(listEmptyState(articlesStore.filters, feed));

/** The hint line around its shortcut slot, when the state has one. */
const hintParts = $derived(empty.hint === null ? null : splitAtKey(empty.hint));
const hintKey = $derived(empty.hintKey === null ? undefined : shortcutKbd(empty.hintKey));

function run(action: ListEmptyAction): void {
    const current = feed;
    if (action === 'showRead') {
        articlesStore.toggleUnreadOnly();
    } else if (action === 'feedHealth') {
        if (current !== null) layoutStore.openEditFeed(current.id, 'health');
    } else if (current !== null) {
        refreshFeed(current.id);
    }
}

const LABELS = {
    showRead: 'list.empty.showRead',
    refreshFeed: 'list.empty.refresh',
    retryFeed: 'toast.retry',
    feedHealth: 'feedHealth.title',
} as const;
</script>

{#snippet hintLine()}
    {#if empty.kind === 'feedError' && feed !== null}
        {feedErrorMessage(feed)}
    {:else if hintParts !== null}
        {hintParts[0]}{#if hintParts[1] !== null}<kbd>{hintKey}</kbd>{hintParts[1]}{/if}
    {/if}
{/snippet}

{#snippet actionRow()}
    {#each empty.actions as action, index (action)}
        <button
            class="state-action"
            class:primary={index === 0 && action !== 'showRead'}
            type="button"
            disabled={(action === 'refreshFeed' || action === 'retryFeed') && feedsStore.refreshing}
            onclick={() => run(action)}
        >
            {#if action === 'refreshFeed' || action === 'retryFeed'}
                <Icon name="refresh" size={14} />
            {/if}
            {t(LABELS[action])}
        </button>
    {/each}
{/snippet}

<EmptyState
    icon={empty.icon}
    title={t(empty.title)}
    tone={empty.kind === 'feedError' ? 'error' : 'accent'}
    hint={hintParts !== null || empty.kind === 'feedError' ? hintLine : undefined}
    actions={empty.actions.length > 0 ? actionRow : undefined}
/>
