<script lang="ts">
/**
 * The right pane: the selected article in full — header, action toolbar
 * (read / star / read-later / archive / open-in-browser), and the sanitized
 * body. Reads ride the query cache (invalidated by the Rust-emitted
 * events); displaying an article marks it read, idempotently, core-side.
 */
import {
    type ArticleDto,
    type ArticleStateDto,
    type CommandError,
    type FeedDto,
    commands,
} from '$lib/bindings';
import { formatIntlDateTime, t } from '$lib/i18n';
import {
    loadFullArticle,
    markReadOnOpen,
    openInBrowser,
    promoteSelected,
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
import Icon from '$components/common/Icon.svelte';
import { tooltip } from '$lib/actions/tooltip';
import ArticleTags from './ArticleTags.svelte';
import TypographyControls from './TypographyControls.svelte';
import ViewModeDispatch from './ViewModeDispatch.svelte';

/** A stable per-feed hue for the monogram chip, derived from the title. */
function feedHue(label: string): number {
    let hash = 0;
    for (let i = 0; i < label.length; i += 1) {
        hash = (hash * 31 + label.charCodeAt(i)) % 360;
    }
    return hash;
}

let showTypography = $state(false);
let hydrating = $state(false);

/** Loads the full readability-extracted article; content updates via events. */
async function hydrate(articleId: number): Promise<void> {
    hydrating = true;
    try {
        await loadFullArticle(articleId);
    } finally {
        hydrating = false;
    }
}

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

function currentFeed(current: ArticleDto): FeedDto | null {
    const feedId = current.feed_id;
    if (feedId === null) {
        return null;
    }
    return feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null;
}

function feedTitle(current: ArticleDto): string | null {
    const feed = currentFeed(current);
    if (feed === null) {
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
        <div class="reader-empty">
            <div class="empty-mark" aria-hidden="true">
                <Icon name="sparkle" size={30} strokeWidth={1.5} />
            </div>
            <p class="empty-title">{t('reader.empty')}</p>
            <p class="empty-hint">
                <kbd>j</kbd><kbd>k</kbd> to move · <kbd>Enter</kbd> to open · <kbd>?</kbd> for shortcuts
            </p>
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
                    class:on={flags()?.read === true}
                    aria-pressed={flags()?.read === true}
                    aria-label={flags()?.read === true ? t('reader.action.markUnread') : t('reader.action.markRead')}
                    use:tooltip={{
                        text: flags()?.read === true ? t('reader.action.markUnread') : t('reader.action.markRead'),
                        kbd: 'M',
                    }}
                    onclick={() => void toggleRead(current.id)}
                >
                    <Icon name="check" />
                </button>
                <button
                    class="tool"
                    class:on={flags()?.starred === true}
                    aria-pressed={flags()?.starred === true}
                    aria-label={flags()?.starred === true ? t('reader.action.unstar') : t('reader.action.star')}
                    use:tooltip={{
                        text: flags()?.starred === true ? t('reader.action.unstar') : t('reader.action.star'),
                        kbd: 'S',
                    }}
                    onclick={() => void toggleStar(current.id)}
                >
                    <Icon name={flags()?.starred === true ? 'star-filled' : 'star'} />
                </button>
                <button
                    class="tool"
                    class:on={flags()?.read_later === true}
                    aria-pressed={flags()?.read_later === true}
                    aria-label={flags()?.read_later === true ? t('reader.action.readLaterRemove') : t('reader.action.readLater')}
                    use:tooltip={{
                        text: flags()?.read_later === true ? t('reader.action.readLaterRemove') : t('reader.action.readLater'),
                        kbd: 'L',
                    }}
                    onclick={() => void toggleReadLater(current.id)}
                >
                    <Icon name="bookmark" />
                </button>
                <button
                    class="tool"
                    class:on={flags()?.archived === true}
                    aria-pressed={flags()?.archived === true}
                    aria-label={flags()?.archived === true ? t('reader.action.unarchive') : t('reader.action.archive')}
                    use:tooltip={{
                        text: flags()?.archived === true ? t('reader.action.unarchive') : t('reader.action.archive'),
                    }}
                    onclick={() => void toggleArchived(current.id)}
                >
                    <Icon name="archive" />
                </button>
                <span class="toolbar-spring"></span>
                <button
                    class="tool"
                    aria-label={t('reader.action.promote')}
                    use:tooltip={{ text: t('reader.action.promote'), kbd: 'P' }}
                    onclick={() => void promoteSelected()}
                >
                    <Icon name="save" />
                </button>
                <div class="typography-anchor">
                    <button
                        class="tool"
                        class:on={showTypography}
                        aria-haspopup="dialog"
                        aria-expanded={showTypography}
                        aria-label={t('reader.action.typography')}
                        use:tooltip={t('reader.action.typography')}
                        onclick={() => {
                            showTypography = !showTypography;
                        }}
                    >
                        <Icon name="type" />
                    </button>
                    {#if showTypography}
                        <div
                            class="typography-popover"
                            role="dialog"
                            aria-label={t('reader.action.typography')}
                            tabindex="-1"
                            onkeydown={(event) => {
                                if (event.key === 'Escape') {
                                    showTypography = false;
                                }
                            }}
                        >
                            <TypographyControls />
                        </div>
                    {/if}
                </div>
                <button
                    class="tool"
                    disabled={hydrating}
                    aria-busy={hydrating}
                    aria-label={t('reader.action.loadFull')}
                    use:tooltip={t('reader.action.loadFull')}
                    onclick={() => void hydrate(current.id)}
                >
                    <Icon name="article" />
                </button>
                <button
                    class="tool"
                    aria-label={t('reader.action.openInBrowser')}
                    use:tooltip={t('reader.action.openInBrowser')}
                    onclick={() => void openInBrowser(current.id, current.source_url)}
                >
                    <Icon name="external" />
                </button>
            </div>
            <!-- Key on the article id so switching articles remounts the scroll
                 body at the top instead of inheriting the previous scroll offset. -->
            {#key current.id}
            <div class="reader-scroll">
                <article
                    class="reader-article"
                    style:font-size="{uiStore.fontSize}px"
                    style:line-height={uiStore.lineHeight}
                    style:max-width="{uiStore.measure}px"
                    style:font-family={uiStore.readerFontStack}
                >
                    <header class="reader-header">
                        {#if feedTitle(current) !== null}
                            {@const label = feedTitle(current) ?? ''}
                            <div class="reader-kicker">
                                <span
                                    class="feed-mono"
                                    style:--mono-hue={feedHue(label)}
                                    aria-hidden="true">{label.slice(0, 1).toUpperCase()}</span
                                >
                                <span class="feed-name">{label}</span>
                            </div>
                        {/if}
                        <h1 class="reader-title">
                            <!-- external URL: the click is intercepted and routed to the URL-scoped opener -->
                            <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
                            <a href={current.source_url} onclick={(event) => openSource(event, current)}>
                                {current.title}
                            </a>
                        </h1>
                        <p class="reader-meta">
                            {#if current.author !== null}
                                <span>{current.author}</span>
                            {/if}
                            {#if current.published_at !== null}
                                {@const published = formatIntlDateTime(new Date(current.published_at))}
                                {#if published !== ''}
                                    <span>{published}</span>
                                {/if}
                            {/if}
                            {#if current.word_count !== null}
                                <span>{t('reader.meta.words', { count: current.word_count })}</span>
                            {/if}
                        </p>
                        <ArticleTags articleId={current.id} />
                    </header>
                    <ViewModeDispatch article={current} feed={currentFeed(current)} />
                </article>
            </div>
            {/key}
        {/if}
    {/if}
</div>

<style>
    .reader {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--surface-app);
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
        color: var(--error-text);
    }

    /* Empty state */
    .reader-empty {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-4);
        padding: var(--space-8);
        text-align: center;
    }

    .empty-mark {
        display: grid;
        place-items: center;
        width: 68px;
        height: 68px;
        border-radius: var(--radius-xl);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 70%);
    }

    .empty-title {
        font-size: var(--text-lg);
        font-weight: 560;
        color: var(--fg);
        letter-spacing: var(--tracking-snug);
    }

    .empty-hint {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
    }

    .empty-hint kbd {
        display: inline-block;
        min-width: 1.5em;
        padding: 0.1em 0.4em;
        margin: 0 0.1em;
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        box-shadow: 0 1px 0 var(--hairline-strong);
        font-family: var(--font-mono);
        font-size: 0.82em;
        color: var(--fg-muted);
        text-align: center;
    }

    .reader-toolbar {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: var(--space-1);
        height: var(--header-height);
        padding: 0 var(--space-4);
        border-bottom: 1px solid var(--hairline);
        background: color-mix(in srgb, var(--surface-app), var(--surface-chrome) 45%);
    }

    .toolbar-spring {
        flex: 1 1 auto;
    }

    .tool {
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

    .tool:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .tool.on {
        color: var(--accent);
        background: var(--selected);
    }

    .typography-anchor {
        position: relative;
        display: flex;
    }

    .typography-popover {
        position: absolute;
        top: calc(100% + var(--space-2));
        right: 0;
        z-index: 50;
        border-radius: var(--radius-lg);
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-lg);
    }

    .reader-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
    }

    .reader-article {
        margin: 0 auto;
        padding: var(--space-10) var(--space-8) var(--space-12);
    }

    .reader-header {
        margin-bottom: var(--space-8);
        padding-bottom: var(--space-5);
        border-bottom: 1px solid var(--hairline);
    }

    .reader-kicker {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        margin-bottom: var(--space-4);
    }

    .feed-mono {
        display: grid;
        place-items: center;
        width: 22px;
        height: 22px;
        flex: 0 0 auto;
        border-radius: var(--radius-sm);
        font-family: var(--font-family);
        font-size: 0.72rem;
        font-weight: 700;
        color: hsl(var(--mono-hue) 65% 82%);
        background: hsl(var(--mono-hue) 45% 32% / 0.5);
        box-shadow: inset 0 0 0 1px hsl(var(--mono-hue) 55% 60% / 0.35);
    }

    .feed-name {
        font-family: var(--font-family);
        font-size: var(--text-sm);
        font-weight: 560;
        letter-spacing: var(--tracking-wide);
        color: var(--fg-muted);
    }

    .reader-title {
        font-family: var(--font-family);
        font-size: var(--text-2xl);
        font-weight: 680;
        line-height: 1.16;
        letter-spacing: var(--tracking-tight);
        text-wrap: balance;
    }

    .reader-title a {
        color: var(--fg);
        text-decoration: none;
    }

    .reader-title a:hover {
        text-decoration: underline;
        text-decoration-thickness: 1px;
        text-underline-offset: 0.12em;
        text-decoration-color: var(--fg-subtle);
    }

    .reader-meta {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-3);
        margin-top: var(--space-3);
        font-family: var(--font-family);
        font-size: var(--text-sm);
        color: var(--fg-subtle);
    }

    .reader-meta span + span::before {
        content: '·';
        margin-right: var(--space-3);
        color: var(--fg-subtle);
    }
</style>
