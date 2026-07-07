<script lang="ts">
/**
 * The YouTube "home page" list layout: the loaded article window rendered as
 * a responsive grid of video cards (poster + title + channel + date) that
 * scrolls like a channel page. Selection stays shared — clicking a card opens
 * it in the reader, exactly like a list row — and keyset paging extends the
 * window near the bottom. The poster is an id-derived gradient by default and
 * the real still only when the media-prefetch setting is on (VideoPoster,
 * through the policed cache) — privacy-first, no unmediated request.
 */
import { type ArticleSummaryDto, commands } from '$lib/bindings';
import { formatIntlDate, t } from '$lib/i18n';
import Icon from '$components/common/Icon.svelte';
import VideoPoster from '$components/common/VideoPoster.svelte';
import { youTubeVideoId } from '$lib/reader/view-mode';
import { articlesStore } from '$lib/state/articles.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import { untrack } from 'svelte';

untrack(() => feedsStore.prime());

const list = $derived(articlesStore.current);
const listError = $derived(list.error === null ? '' : commandErrorMessage(list.error));

function channelOf(article: ArticleSummaryDto): string | null {
    if (article.author !== null) {
        return article.author;
    }
    const feed = feedsStore.feeds.find((f) => f.id === article.feed_id);
    return feed?.title ?? null;
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

<div class="yt-scroll" onscroll={onScroll}>
    {#if list.error !== null}
        <p class="status error" role="alert">{listError}</p>
    {:else if !list.loaded}
        <p class="status">{t('list.loading')}</p>
    {:else if list.items.length === 0}
        <p class="status">{t('list.empty')}</p>
    {:else}
        <div class="grid" role="listbox" aria-label={t('list.label')}>
            {#each list.items as article (article.id)}
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
                    <span class="poster" aria-hidden="true">
                        <VideoPoster
                            videoId={youTubeVideoId(article.source_url)}
                            seed={String(article.id)}
                        />
                        <span class="play"><Icon name="play" size={16} /></span>
                    </span>
                    <span class="meta">
                        <span class="title truncate-2">{article.title}</span>
                        <span class="sub truncate">
                            {#if channelOf(article) !== null}<span class="channel"
                                    >{channelOf(article)}</span
                                >{/if}
                            {#if article.published_at !== null}<span
                                    >{formatIntlDate(new Date(article.published_at))}</span
                                >{/if}
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
    .yt-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
    }

    .grid {
        display: grid;
        grid-template-columns: repeat(auto-fill, minmax(144px, 1fr));
        gap: var(--space-4) var(--space-3);
        padding: var(--space-4) var(--space-3);
    }

    .status {
        padding: var(--space-4);
        font-size: var(--text-md);
        color: var(--fg-muted);
    }
    .status.error {
        color: var(--error);
    }
    .status.subtle {
        text-align: center;
        color: var(--fg-subtle);
    }

    .card {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding: 0;
        background: transparent;
        text-align: left;
        border-radius: var(--radius-lg);
        transition: transform var(--dur-fast) var(--ease);
    }

    .card:hover {
        transform: translateY(-2px);
    }

    .poster {
        position: relative;
        display: block;
        width: 100%;
        aspect-ratio: 16 / 9;
        border-radius: var(--radius-lg);
        overflow: hidden;
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-sm);
    }

    .play {
        position: absolute;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%) scale(0.9);
        display: grid;
        place-items: center;
        width: 44px;
        height: 44px;
        border-radius: 50%;
        background: #ff0000;
        color: #fff;
        opacity: 0.92;
        box-shadow: 0 4px 14px rgb(0 0 0 / 32%);
        transition:
            transform var(--dur-fast) var(--ease),
            opacity var(--dur-fast) var(--ease);
    }

    .card:hover .play {
        transform: translate(-50%, -50%) scale(1);
        opacity: 1;
    }

    .card.selected .poster {
        outline: 2px solid var(--accent);
        outline-offset: 2px;
    }

    .meta {
        display: flex;
        flex-direction: column;
        gap: 3px;
        padding: 0 var(--space-1);
    }

    .title {
        font-family: var(--font-family);
        font-size: var(--text-md);
        font-weight: 560;
        line-height: 1.3;
        color: var(--fg);
        letter-spacing: var(--tracking-snug);
    }

    .card.read .title {
        color: var(--fg-muted);
        font-weight: 500;
    }

    .sub {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .channel {
        font-weight: 500;
        color: var(--fg-muted);
    }

    .channel + span::before {
        content: '·';
        margin-right: var(--space-2);
    }

    .truncate-2 {
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
</style>
