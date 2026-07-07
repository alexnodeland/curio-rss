<script lang="ts">
/**
 * The middle column: a header (search + destinations entry) over either the
 * filter list or the FTS results, chosen by whether a search is active. The
 * reader reacts to selection regardless of which list is showing.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import RedditFeed from '$components/articles/RedditFeed.svelte';
import SearchResults from '$components/articles/SearchResults.svelte';
import YouTubeGrid from '$components/articles/YouTubeGrid.svelte';
import Icon from '$components/common/Icon.svelte';
import SearchBar from '$components/common/SearchBar.svelte';
import { t } from '$lib/i18n';
import { feedHomeType } from '$lib/reader/view-mode';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';

// The home layout only applies when a single YouTube/Reddit-typed feed is
// selected (not "All articles" — a mixed corpus has no one home page).
const homeType = $derived.by(() => {
    const feedId = selectionStore.selectedFeedId;
    if (feedId === null) {
        return null;
    }
    const feed = feedsStore.feeds.find((candidate) => candidate.id === feedId);
    return feed === undefined ? null : feedHomeType(feed.url);
});

const homeOn = $derived(homeType !== null && uiStore.isHomeLayout(homeType));
</script>

<div class="list-pane">
    <div class="list-header">
        <div class="search-slot">
            <SearchBar />
        </div>
        {#if !searchStore.active}
            <button
                class="list-tool"
                type="button"
                aria-pressed={articlesStore.unreadOnly}
                title={articlesStore.unreadOnly ? t('list.filter.all') : t('list.filter.unreadOnly')}
                aria-label={articlesStore.unreadOnly
                    ? t('list.filter.all')
                    : t('list.filter.unreadOnly')}
                onclick={() => articlesStore.toggleUnreadOnly()}
            >
                <Icon name="dot" />
            </button>
        {/if}
        {#if homeType !== null && !searchStore.active}
            <button
                class="list-tool"
                type="button"
                aria-pressed={homeOn}
                title={homeOn ? t('list.view.rows') : t('list.view.home')}
                aria-label={homeOn ? t('list.view.rows') : t('list.view.home')}
                onclick={() => void uiStore.setHomeLayout(homeType, !homeOn)}
            >
                <Icon name={homeOn ? 'rows' : 'grid'} />
            </button>
        {/if}
        <button
            class="list-tool"
            type="button"
            title={t('destinations.open')}
            aria-label={t('destinations.open')}
            onclick={() => uiStore.openModal('destinations')}
        >
            <Icon name="folder" />
        </button>
    </div>
    {#if searchStore.active}
        <SearchResults />
    {:else if homeOn && homeType === 'youtube'}
        <YouTubeGrid />
    {:else if homeOn && homeType === 'reddit'}
        <RedditFeed />
    {:else}
        <ArticleList />
    {/if}
</div>

<style>
    .list-pane {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--surface-list);
        border-right: 1px solid var(--hairline);
    }

    .list-header {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        height: var(--header-height);
        padding: 0 var(--space-3);
        border-bottom: 1px solid var(--hairline);
    }

    .search-slot {
        flex: 1 1 auto;
        min-width: 0;
    }

    .list-tool {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        width: 34px;
        height: 34px;
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid transparent;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .list-tool:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .list-tool[aria-pressed='true'] {
        background: var(--selected);
        color: var(--accent);
    }
</style>
