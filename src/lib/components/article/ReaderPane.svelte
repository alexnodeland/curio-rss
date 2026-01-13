<script lang="ts">
    import type { Article } from '$lib/types';
    import { toggleArticleStarred, markArticlesUnread } from '$lib/stores/feeds';
    import { readerFontSize, readerLineHeight, readerMaxWidth } from '$lib/stores/ui';
    import { formatDate } from '$lib/utils/format';

    export let article: Article;

    async function handleToggleStarred() {
        await toggleArticleStarred(article.id);
    }

    async function handleMarkUnread() {
        await markArticlesUnread([article.id]);
    }

    function openInBrowser() {
        if (article.url) {
            window.open(article.url, '_blank');
        }
    }
</script>

<article
    class="reader-pane"
    style="--reader-font-size: {$readerFontSize}px; --reader-line-height: {$readerLineHeight}; --reader-max-width: {$readerMaxWidth}px;"
>
    <header class="reader-header">
        <div class="reader-actions">
            <button
                class="action-btn"
                class:active={article.is_starred}
                on:click={handleToggleStarred}
                title="Toggle starred"
            >
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill={article.is_starred ? 'currentColor' : 'none'} stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                </svg>
            </button>
            <button class="action-btn" on:click={handleMarkUnread} title="Mark as unread">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"/>
                </svg>
            </button>
            {#if article.url}
                <button class="action-btn" on:click={openInBrowser} title="Open in browser">
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M18 13v6a2 2 0 01-2 2H5a2 2 0 01-2-2V8a2 2 0 012-2h6"/>
                        <polyline points="15 3 21 3 21 9"/>
                        <line x1="10" y1="14" x2="21" y2="3"/>
                    </svg>
                </button>
            {/if}
        </div>
    </header>

    <div class="reader-content">
        <div class="content-wrapper">
            <h1 class="article-title">{article.title}</h1>

            <div class="article-meta">
                {#if article.author}
                    <span class="author">By {article.author}</span>
                {/if}
                {#if article.published_at}
                    <time datetime={article.published_at}>
                        {formatDate(article.published_at)}
                    </time>
                {/if}
                {#if article.url}
                    <a href={article.url} target="_blank" rel="noopener noreferrer" class="source-link">
                        View Original
                    </a>
                {/if}
            </div>

            <div class="article-body">
                {#if article.content_html}
                    {@html article.content_html}
                {:else if article.summary}
                    <p>{article.summary}</p>
                {:else}
                    <p class="no-content">No content available.</p>
                {/if}
            </div>
        </div>
    </div>
</article>

<style>
    .reader-pane {
        flex: 1;
        height: 100%;
        background: var(--bg);
        display: flex;
        flex-direction: column;
        overflow: hidden;
    }

    .reader-header {
        display: flex;
        justify-content: flex-end;
        padding: var(--space-3) var(--space-4);
        border-bottom: 1px solid var(--border);
    }

    .reader-actions {
        display: flex;
        gap: var(--space-2);
    }

    .action-btn {
        padding: var(--space-2);
        color: var(--fg-muted);
        border-radius: var(--radius-md);
    }

    .action-btn:hover {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    .action-btn.active {
        color: var(--warning);
    }

    .reader-content {
        flex: 1;
        overflow-y: auto;
        padding: var(--space-6);
    }

    .content-wrapper {
        max-width: var(--reader-max-width);
        margin: 0 auto;
        font-size: var(--reader-font-size);
        line-height: var(--reader-line-height);
    }

    .article-title {
        font-size: calc(var(--reader-font-size) * 1.75);
        font-weight: 700;
        line-height: 1.3;
        color: var(--fg);
        margin-bottom: var(--space-4);
    }

    .article-meta {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-3);
        margin-bottom: var(--space-6);
        padding-bottom: var(--space-4);
        border-bottom: 1px solid var(--border);
        font-size: 14px;
        color: var(--fg-muted);
    }

    .source-link {
        color: var(--accent);
    }

    .article-body {
        color: var(--fg);
    }

    .article-body :global(p) {
        margin-bottom: 1.5em;
    }

    .article-body :global(h1),
    .article-body :global(h2),
    .article-body :global(h3),
    .article-body :global(h4),
    .article-body :global(h5),
    .article-body :global(h6) {
        margin-top: 2em;
        margin-bottom: 0.75em;
        font-weight: 600;
        line-height: 1.3;
    }

    .article-body :global(h2) {
        font-size: 1.5em;
    }

    .article-body :global(h3) {
        font-size: 1.25em;
    }

    .article-body :global(a) {
        color: var(--link);
    }

    .article-body :global(img) {
        max-width: 100%;
        height: auto;
        border-radius: var(--radius-md);
        margin: var(--space-4) 0;
    }

    .article-body :global(pre) {
        background: var(--bg-secondary);
        padding: var(--space-4);
        border-radius: var(--radius-md);
        overflow-x: auto;
        font-family: var(--font-mono);
        font-size: 0.9em;
    }

    .article-body :global(code) {
        background: var(--bg-secondary);
        padding: 0.2em 0.4em;
        border-radius: var(--radius-sm);
        font-family: var(--font-mono);
        font-size: 0.9em;
    }

    .article-body :global(pre code) {
        background: none;
        padding: 0;
    }

    .article-body :global(blockquote) {
        border-left: 4px solid var(--accent);
        margin: var(--space-4) 0;
        padding-left: var(--space-4);
        color: var(--fg-muted);
        font-style: italic;
    }

    .article-body :global(ul),
    .article-body :global(ol) {
        margin-bottom: 1.5em;
        padding-left: 1.5em;
    }

    .article-body :global(li) {
        margin-bottom: 0.5em;
    }

    .no-content {
        color: var(--fg-muted);
        font-style: italic;
    }
</style>
