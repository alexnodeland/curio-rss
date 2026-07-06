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
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-3);
        height: 84px; /* = ROW_HEIGHT in ArticleList */
        padding: 0 var(--space-4);
        margin: 0 var(--space-2);
        border-radius: var(--radius-lg);
        cursor: pointer;
        transition: background var(--dur-fast) var(--ease);
    }

    /* Hairline between rows, inset so it doesn't touch the rounded edges. */
    .article-row::after {
        content: '';
        position: absolute;
        left: var(--space-4);
        right: var(--space-4);
        bottom: 0;
        height: 1px;
        background: var(--hairline);
    }

    .article-row:hover {
        background: var(--hover);
    }

    .article-row:hover::after,
    .article-row.selected::after {
        opacity: 0;
    }

    .article-row.selected {
        background: var(--selected);
    }

    /* Accent spine on the selected row. */
    .article-row.selected::before {
        content: '';
        position: absolute;
        left: 0;
        top: 12px;
        bottom: 12px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .row-dot {
        flex: 0 0 auto;
        width: 7px;
        height: 7px;
        border-radius: 50%;
        background: transparent;
        transition: background var(--dur-fast) var(--ease);
    }

    .article-row.unread .row-dot {
        background: var(--unread);
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--unread), transparent 82%);
    }

    .row-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 3px;
    }

    .row-title {
        font-size: var(--text-base);
        font-weight: 500;
        line-height: 1.35;
        color: var(--fg-muted);
        letter-spacing: var(--tracking-snug);
    }

    .article-row.unread .row-title {
        font-weight: 600;
        color: var(--fg);
    }

    .row-meta {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .row-feed {
        color: var(--fg-muted);
        font-weight: 500;
    }

    .row-meta .row-feed + span::before {
        content: '·';
        margin-right: var(--space-2);
        color: var(--fg-subtle);
    }

    .row-star {
        flex: 0 0 auto;
        color: var(--warning);
        font-size: 0.8125rem;
    }
</style>
