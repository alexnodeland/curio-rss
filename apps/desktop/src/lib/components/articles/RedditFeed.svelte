<script lang="ts">
/**
 * The Reddit "home page" list layout: the loaded article window rendered as a
 * single-column feed of post cards (subreddit, author, title) that scrolls
 * like a subreddit front page. Selection stays shared — clicking a card opens
 * it in the reader — and keyset paging extends the window near the bottom.
 * RSS-native only (D8): subreddit + author come from the summary/URL the feed
 * already sent; no vote or comment counts (RSS doesn't carry them).
 */
import { type ArticleSummaryDto, commands } from '$lib/bindings';
import { formatIntlDate, t } from '$lib/i18n';
import ArticleThumb from '$components/common/ArticleThumb.svelte';
import Icon from '$components/common/Icon.svelte';
import { subredditOf } from '$lib/reader/view-mode';
import { articlesStore } from '$lib/state/articles.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';

const list = $derived(articlesStore.current);
const listError = $derived(list.error === null ? '' : commandErrorMessage(list.error));

function author(article: ArticleSummaryDto): string | null {
    if (article.author === null) {
        return null;
    }
    return article.author.replace(/^\/?u\//, '');
}

function isRead(article: ArticleSummaryDto): boolean {
    return (
        ensureQuery(queryKeys.articleState(article.id), () => commands.getArticleState(article.id))
            .data?.read === true
    );
}

function onScroll(event: Event & { currentTarget: HTMLElement }): void {
    const el = event.currentTarget;
    if (el.scrollTop + el.clientHeight >= el.scrollHeight - 500) {
        void list.loadMore();
    }
}
</script>

<div class="rd-scroll" onscroll={onScroll}>
    {#if list.error !== null}
        <p class="status error" role="alert">{listError}</p>
    {:else if !list.loaded}
        <p class="status">{t('list.loading')}</p>
    {:else if list.items.length === 0}
        <p class="status">{t('list.empty')}</p>
    {:else}
        <div class="feed" role="listbox" aria-label={t('list.label')}>
            {#each list.items as article (article.id)}
                {@const sub = subredditOf(article.source_url)}
                <button
                    class="card"
                    class:selected={article.id === selectionStore.selectedArticleId}
                    class:read={isRead(article)}
                    role="option"
                    aria-selected={article.id === selectionStore.selectedArticleId}
                    onclick={() => {
                        selectionStore.selectedArticleId = article.id;
                    }}
                >
                    <span class="avatar" aria-hidden="true"><Icon name="message" size={16} /></span>
                    <span class="body">
                        <span class="head truncate">
                            {#if sub !== null}<span class="sub">r/{sub}</span>{/if}
                            {#if author(article) !== null}<span class="dot">·</span><span
                                    class="author">u/{author(article)}</span
                                >{/if}
                            {#if article.published_at !== null}<span class="dot">·</span><span
                                    >{formatIntlDate(new Date(article.published_at))}</span
                                >{/if}
                        </span>
                        <span class="title">{article.title}</span>
                        <ArticleThumb variant="card" image={article.image} alt="" />
                        <span class="foot">
                            <span class="comments"><Icon name="message" size={13} /> {t('reader.reddit.openThread')}</span>
                        </span>
                    </span>
                </button>
            {/each}
        </div>
        {#if list.appending}
            <p class="status subtle">{t('list.loading')}</p>
        {/if}
    {/if}
</div>

<style>
    .rd-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
    }

    .feed {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
        padding: var(--space-4);
    }

    .status {
        padding: var(--space-4);
        font-size: var(--text-md);
        color: var(--fg-muted);
    }
    .status.error {
        color: var(--error-text);
    }
    .status.subtle {
        text-align: center;
        color: var(--fg-subtle);
    }

    .card {
        display: flex;
        gap: var(--space-3);
        padding: var(--space-4);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
        text-align: left;
        transition:
            border-color var(--dur-fast) var(--ease),
            background var(--dur-fast) var(--ease);
    }

    .card:hover {
        border-color: var(--hairline-strong);
        background: var(--surface-overlay);
    }

    .card.selected {
        border-color: color-mix(in srgb, var(--accent), transparent 45%);
        background: var(--selected);
    }

    .avatar {
        display: grid;
        place-items: center;
        width: 30px;
        height: 30px;
        flex: 0 0 auto;
        border-radius: 50%;
        color: #fff;
        background: linear-gradient(140deg, #ff6a33, #ff4500);
    }

    .body {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        min-width: 0;
        flex: 1 1 auto;
    }

    .head {
        display: flex;
        align-items: center;
        gap: var(--space-1);
        font-family: var(--font-family);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .sub {
        font-weight: 650;
        color: #ff6a33;
    }

    .author {
        color: var(--fg-muted);
    }

    .dot {
        color: var(--fg-subtle);
    }

    .title {
        font-family: var(--font-family);
        font-size: var(--text-base);
        font-weight: 600;
        line-height: 1.35;
        color: var(--fg);
        letter-spacing: var(--tracking-snug);
    }

    .card.read .title {
        color: var(--fg-muted);
        font-weight: 500;
    }

    .foot {
        display: flex;
        align-items: center;
        margin-top: 2px;
    }

    .comments {
        display: inline-flex;
        align-items: center;
        gap: 5px;
        font-family: var(--font-family);
        font-size: var(--text-xs);
        font-weight: 500;
        color: var(--fg-subtle);
    }
</style>
