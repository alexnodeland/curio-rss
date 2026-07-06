<script lang="ts">
/**
 * One virtualized list row. Summaries carry no state flags by design (list
 * payloads stay small), so the unread dot / star come from the per-article
 * state query — cached, and refreshed by the Rust-emitted invalidation
 * events like every other read. Height must match `ROW_HEIGHT` in
 * ArticleList (the virtualizer's fixed row height).
 */
import { type ArticleSummaryDto, commands } from '$lib/bindings';
import { formatIntlDate, t } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';

let {
    article,
    index,
    setsize,
    selected,
    onselect,
}: {
    article: ArticleSummaryDto;
    index: number;
    setsize: number;
    selected: boolean;
    onselect: (articleId: number) => void;
} = $props();

function flags() {
    const articleId = article.id;
    return ensureQuery(queryKeys.articleState(articleId), () => commands.getArticleState(articleId))
        .data;
}

function feedTitle(): string | null {
    const feedId = article.feed_id;
    if (feedId === null) {
        return null;
    }
    const feed = feedsStore.feeds.find((candidate) => candidate.id === feedId);
    if (feed === undefined) {
        return null;
    }
    return feed.title ?? feed.url;
}

function publishedLabel(): string | null {
    const published = article.published_at;
    return published === null ? null : formatIntlDate(new Date(published));
}

function onKeyDown(event: KeyboardEvent): void {
    if (event.key === 'Enter' || event.key === ' ') {
        event.preventDefault();
        onselect(article.id);
    }
}
</script>

<div
    class="article-row"
    class:selected
    class:unread={flags()?.read === false}
    role="option"
    aria-selected={selected}
    aria-posinset={index + 1}
    aria-setsize={setsize}
    tabindex="-1"
    onclick={() => onselect(article.id)}
    onkeydown={onKeyDown}
>
    <span class="row-dot" aria-hidden="true"></span>
    {#if flags()?.read === false}
        <span class="sr-only">{t('list.row.unread')}</span>
    {/if}
    <div class="row-main">
        <span class="row-title truncate">{article.title}</span>
        <span class="row-meta truncate">
            {#if feedTitle() !== null}
                <span class="row-feed">{feedTitle()}</span>
            {/if}
            {#if publishedLabel() !== null}
                <span>{publishedLabel()}</span>
            {/if}
        </span>
    </div>
    {#if flags()?.starred === true}
        <span class="row-star" aria-hidden="true">★</span>
        <span class="sr-only">{t('list.row.starred')}</span>
    {/if}
</div>

<style>
    .article-row {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        height: 84px; /* = ROW_HEIGHT in ArticleList */
        padding: 0 var(--space-3);
        border-bottom: 1px solid var(--border-subtle);
        cursor: pointer;
    }

    .article-row:hover {
        background: var(--bg-hover);
    }

    .article-row.selected {
        background: var(--accent-muted);
    }

    .row-dot {
        flex: 0 0 auto;
        width: 8px;
        height: 8px;
        border-radius: 50%;
        background: transparent;
    }

    .article-row.unread .row-dot {
        background: var(--unread);
    }

    .row-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .row-title {
        font-size: 0.9375rem;
        font-weight: 500;
        color: var(--fg);
    }

    .article-row.unread .row-title {
        font-weight: 600;
    }

    .row-meta {
        display: flex;
        gap: var(--space-2);
        font-size: 0.75rem;
        color: var(--fg-muted);
    }

    .row-feed {
        color: var(--fg-subtle);
    }

    .row-star {
        flex: 0 0 auto;
        color: var(--warning);
        font-size: 0.875rem;
    }
</style>
