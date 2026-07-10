<script lang="ts" module>
import type { ArticleStateDto, ArticleSummaryDto } from '$lib/bindings';
import {
    markAllRead,
    openInBrowser,
    toggleArchived,
    toggleRead,
    toggleReadLater,
    toggleStar,
} from '$lib/state/actions';
import type { MenuItem } from '$lib/state/menu.svelte';

/**
 * The DOM id of the row for one article — the listbox points at it through
 * `aria-activedescendant`, so the id scheme lives here next to the row it
 * names (ArticleList and VirtualList reference the same helper).
 */
export function articleOptionId(articleId: number): string {
    return `article-option-${articleId}`;
}

/**
 * The context-menu items for one article row — star, read, read-later, archive,
 * open-in-browser, and mark-all-read-in-feed. Labels flip on the live state
 * (from the per-article state query). Shared by the row's own right-click and
 * the list's keyboard menu key so both surfaces stay identical.
 */
export function buildArticleMenu(
    article: ArticleSummaryDto,
    state: ArticleStateDto | undefined,
): MenuItem[] {
    const items: MenuItem[] = [
        {
            id: 'star',
            labelKey: state?.starred ? 'reader.action.unstar' : 'reader.action.star',
            onSelect: () => void toggleStar(article.id),
        },
        {
            id: 'read',
            labelKey: state?.read ? 'reader.action.markUnread' : 'reader.action.markRead',
            onSelect: () => void toggleRead(article.id),
        },
        {
            id: 'readLater',
            labelKey: state?.read_later
                ? 'reader.action.readLaterRemove'
                : 'reader.action.readLater',
            onSelect: () => void toggleReadLater(article.id),
        },
        {
            id: 'archive',
            labelKey: state?.archived ? 'reader.action.unarchive' : 'reader.action.archive',
            separatorBefore: true,
            onSelect: () => void toggleArchived(article.id),
        },
        {
            id: 'open',
            labelKey: 'reader.action.openInBrowser',
            separatorBefore: true,
            onSelect: () => void openInBrowser(article.id, article.source_url),
        },
    ];
    const feedId = article.feed_id;
    if (feedId !== null) {
        items.push({
            id: 'markAllRead',
            labelKey: 'feed.menu.markAllRead',
            separatorBefore: true,
            onSelect: () => void markAllRead(feedId),
        });
    }
    return items;
}
</script>

<script lang="ts">
/**
 * One virtualized list row. Summaries carry no state flags by design (list
 * payloads stay small), so the unread dot / star come from the per-article
 * state query — cached, and refreshed by the Rust-emitted invalidation
 * events like every other read. The row applies `rowHeight` inline so its
 * height always equals the virtualizer's fixed row height (one source, no
 * drift). `compact` density drops the body preview and thumbnail.
 */
import { commands } from '$lib/bindings';
import ArticleThumb from '$components/common/ArticleThumb.svelte';
import { contextMenu } from '$lib/actions/context-menu';
import { formatIntlDate, t } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { highlightSegments } from '$lib/utils/highlight';

let {
    article,
    index,
    rowHeight,
    compact,
    setsize,
    selected,
    onselect,
    highlight,
}: {
    article: ArticleSummaryDto;
    index: number;
    rowHeight: number;
    compact: boolean;
    setsize: number;
    selected: boolean;
    onselect: (articleId: number) => void;
    /** Active search query — when set, its terms are `<mark>`-ed in title/snippet. */
    highlight?: string;
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

<!-- Renders text with the active search query's terms wrapped in <mark>. Each
     segment is plain, Svelte-escaped text — no raw-HTML injection is involved. -->
{#snippet marked(text: string)}
    {#if highlight}
        {#each highlightSegments(text, highlight) as segment, i (i)}
            {#if segment.mark}<mark class="row-mark">{segment.text}</mark>{:else}{segment.text}{/if}
        {/each}
    {:else}
        {text}
    {/if}
{/snippet}

<div
    id={articleOptionId(article.id)}
    class="article-row"
    class:selected
    class:compact
    class:unread={flags()?.read === false}
    style="height: {rowHeight}px"
    role="option"
    aria-selected={selected}
    aria-posinset={index + 1}
    aria-setsize={setsize}
    tabindex="-1"
    onclick={() => onselect(article.id)}
    onkeydown={onKeyDown}
    use:contextMenu={{ items: () => buildArticleMenu(article, flags()), ariaLabel: article.title }}
>
    <span class="row-dot" aria-hidden="true"></span>
    {#if flags()?.read === false}
        <span class="sr-only">{t('list.row.unread')}</span>
    {/if}
    <div class="row-main">
        <span class="row-title truncate">{@render marked(article.title)}</span>
        {#if !compact && article.snippet !== null}
            <span class="row-snippet truncate">{@render marked(article.snippet)}</span>
        {/if}
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
    {#if !compact}
        <ArticleThumb variant="row" image={article.image} alt="" />
    {/if}
</div>

<style>
    .article-row {
        /* height is applied inline from `rowHeight` (= the virtualizer's
           fixed row height for the current density) so the two can't drift. */
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-3);
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

    /* Compact rows tighten the title to a single visual line. */
    .article-row.compact .row-title {
        line-height: 1.25;
    }

    .row-snippet {
        font-size: var(--text-xs);
        line-height: 1.4;
        color: var(--fg-subtle);
    }

    /* Search-term hit: a soft accent wash that reads on every theme without
       recolouring the surrounding text. */
    .row-mark {
        background: color-mix(in srgb, var(--accent), transparent 72%);
        color: inherit;
        border-radius: 3px;
        padding: 0 1px;
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
        color: var(--warning-text);
        font-size: 0.8125rem;
    }
</style>
