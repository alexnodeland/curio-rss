<script lang="ts">
/**
 * The right pane: the selected article in full — header, action toolbar
 * (read / star / read-later / archive / open-in-browser), and the sanitized
 * body. Reads ride the query cache (invalidated by the Rust-emitted
 * events); displaying an article marks it read, idempotently, core-side.
 */
import { type ArticleDto, type ArticleStateDto, type CommandError, commands } from '$lib/bindings';
import { formatIntlDateTime, t } from '$lib/i18n';
import {
    markReadOnOpen,
    openInBrowser,
    toggleArchived,
    toggleRead,
    toggleReadLater,
    toggleStar,
} from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import SanitizedHtml from './SanitizedHtml.svelte';

function article(): ArticleDto | null {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return null;
    }
    return (
        ensureQuery(queryKeys.article(articleId), () => commands.getArticle(articleId)).data ?? null
    );
}

function articleLoaded(): boolean {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return false;
    }
    return ensureQuery(queryKeys.article(articleId), () => commands.getArticle(articleId)).loaded;
}

function failure(): CommandError | null {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return null;
    }
    return ensureQuery(queryKeys.article(articleId), () => commands.getArticle(articleId)).error;
}

function failureMessage(): string {
    const error = failure();
    return error === null ? '' : commandErrorMessage(error);
}

function flags(): ArticleStateDto | null {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return null;
    }
    return (
        ensureQuery(queryKeys.articleState(articleId), () => commands.getArticleState(articleId))
            .data ?? null
    );
}

function feedTitle(current: ArticleDto): string | null {
    const feedId = current.feed_id;
    if (feedId === null) {
        return null;
    }
    const feed = feedsStore.feeds.find((candidate) => candidate.id === feedId);
    if (feed === undefined) {
        return null;
    }
    return feed.title ?? feed.url;
}

// Displaying an article marks it read — once per selection, not per render.
let lastOpenedId: number | null = null;
$effect(() => {
    const articleId = selectionStore.selectedArticleId;
    if (articleId !== null && articleId !== lastOpenedId) {
        lastOpenedId = articleId;
        void markReadOnOpen(articleId);
    }
});

function openSource(event: MouseEvent, current: ArticleDto): void {
    event.preventDefault();
    void openInBrowser(current.id, current.source_url);
}
</script>

<div class="reader">
    {#if selectionStore.selectedArticleId === null}
        <div class="reader-status">
            <p>{t('reader.empty')}</p>
        </div>
    {:else if failure() !== null}
        <div class="reader-status">
            <p class="error" role="alert">{failureMessage()}</p>
        </div>
    {:else}
        {@const current = article()}
        {#if current === null}
            <div class="reader-status">
                <p>{articleLoaded() ? t('reader.missing') : t('app.loading')}</p>
            </div>
        {:else}
            <div class="reader-toolbar" role="toolbar" aria-label={t('reader.toolbar')}>
                <button
                    class="tool"
                    aria-pressed={flags()?.read === true}
                    onclick={() => void toggleRead(current.id)}
                >
                    {flags()?.read === true
                        ? t('reader.action.markUnread')
                        : t('reader.action.markRead')}
                </button>
                <button
                    class="tool"
                    aria-pressed={flags()?.starred === true}
                    onclick={() => void toggleStar(current.id)}
                >
                    {flags()?.starred === true
                        ? t('reader.action.unstar')
                        : t('reader.action.star')}
                </button>
                <button
                    class="tool"
                    aria-pressed={flags()?.read_later === true}
                    onclick={() => void toggleReadLater(current.id)}
                >
                    {flags()?.read_later === true
                        ? t('reader.action.readLaterRemove')
                        : t('reader.action.readLater')}
                </button>
                <button
                    class="tool"
                    aria-pressed={flags()?.archived === true}
                    onclick={() => void toggleArchived(current.id)}
                >
                    {flags()?.archived === true
                        ? t('reader.action.unarchive')
                        : t('reader.action.archive')}
                </button>
                <span class="toolbar-spring"></span>
                <button
                    class="tool"
                    onclick={() => void openInBrowser(current.id, current.source_url)}
                >
                    {t('reader.action.openInBrowser')}
                </button>
            </div>
            <div class="reader-scroll">
                <article
                    class="reader-article"
                    style:font-size="{uiStore.fontSize}px"
                    style:line-height={uiStore.lineHeight}
                    style:max-width="{uiStore.measure}px"
                >
                    <header class="reader-header">
                        <h2 class="reader-title">
                            <!-- external URL: the click is intercepted and routed to the URL-scoped opener -->
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
                            <a href={current.source_url} onclick={(event) => openSource(event, current)}>
                                {current.title}
                            </a>
                        </h2>
                        <p class="reader-meta">
                            {#if feedTitle(current) !== null}
                                <span>{feedTitle(current)}</span>
                            {/if}
                            {#if current.author !== null}
                                <span>{current.author}</span>
                            {/if}
                            {#if current.published_at !== null}
                                <span>{formatIntlDateTime(new Date(current.published_at))}</span>
                            {/if}
                            {#if current.word_count !== null}
                                <span>{t('reader.meta.words', { count: current.word_count })}</span>
                            {/if}
                        </p>
                    </header>
                    <SanitizedHtml html={current.content_html} />
                </article>
            </div>
        {/if}
    {/if}
</div>

<style>
    .reader {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--bg);
    }

    .reader-status {
        flex: 1 1 auto;
        display: flex;
        align-items: center;
        justify-content: center;
        color: var(--fg-muted);
        font-size: 0.9375rem;
    }

    .reader-status .error {
        color: var(--error);
    }

    .reader-toolbar {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-4);
        border-bottom: 1px solid var(--border-subtle);
        background: var(--bg-secondary);
    }

    .toolbar-spring {
        flex: 1 1 auto;
    }

    .tool {
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        font-size: 0.8125rem;
        border: 1px solid transparent;
    }

    .tool:hover {
        background: var(--bg-hover);
        color: var(--fg);
    }

    .tool[aria-pressed='true'] {
        color: var(--accent);
        border-color: var(--accent-muted);
        background: var(--accent-muted);
    }

    .reader-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
    }

    .reader-article {
        margin: 0 auto;
        padding: var(--space-6) var(--space-5) var(--space-8);
    }

    .reader-header {
        margin-bottom: var(--space-5);
    }

    .reader-title {
        font-size: 1.375rem;
        line-height: 1.3;
        letter-spacing: -0.01em;
    }

    .reader-title a {
        color: var(--fg);
        text-decoration: none;
    }

    .reader-title a:hover {
        color: var(--link);
        text-decoration: underline;
    }

    .reader-meta {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-3);
        margin-top: var(--space-2);
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }
</style>
