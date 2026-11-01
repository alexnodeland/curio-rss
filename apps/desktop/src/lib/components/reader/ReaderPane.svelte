<script lang="ts">
/**
 * The right pane: the selected article in full — header, action toolbar
 * (read / star / read-later / archive / open-in-browser), and the sanitized
 * body. Reads ride the query cache (invalidated by the Rust-emitted
 * events); displaying an article marks it read, idempotently, core-side.
 *
 * The toolbar and header stay mounted across j/k: while a body is still
 * loading, the header renders from the list row already in hand
 * (`selectionStore.selectedArticle`) over a body skeleton, and the bodies of
 * the neighbouring rows are prefetched so the next step usually lands on a
 * cached article.
 *
 * Keyboard: the scroll body is a focusable region. Enter / → in the list
 * (`selectionStore.focusReader`) moves focus here, ← / Esc hand it back, and
 * Space / Shift+Space page it (the shell's global keys, via
 * `registerReaderBody`). The empty-state hint is built from the registry, so
 * it only ever names keys that are bound.
 */
import {
    type ArticleDto,
    type ArticleStateDto,
    type CommandError,
    type FeedDto,
    commands,
} from '$lib/bindings';
import { type MessageKey, formatIntlDateTime, t } from '$lib/i18n';
import { advertisedKeys, keyLabel, shortcutKbd } from '$lib/keyboard/labels';
import { type ShortcutId, eventKeyName, paneShortcut } from '$lib/keyboard/registry';
import {
    handleShortcut,
    loadFullArticle,
    markReadOnOpen,
    openInBrowser,
    promoteSelected,
    registerReaderBody,
    toggleArchived,
    toggleRead,
    toggleReadLater,
    toggleStar,
} from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { articlePrefetcher, neighbourIds } from '$lib/state/prefetch';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { typographyStore } from '$lib/state/typography.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import EmptyState from '$components/common/EmptyState.svelte';
import Icon from '$components/common/Icon.svelte';
import Skeleton from '$components/common/Skeleton.svelte';
import { dismissable } from '$lib/actions/dismissable';
import { tooltip } from '$lib/actions/tooltip';
import ArticleTags from './ArticleTags.svelte';
import TypographyControls from './TypographyControls.svelte';
import ViewModeDispatch from './ViewModeDispatch.svelte';
import { untrack } from 'svelte';

/** The empty-state hint, as registry actions: each part prints its bound keys. */
const HINT: readonly { readonly ids: readonly ShortcutId[]; readonly label: MessageKey }[] = [
    { ids: ['nav.previousArticle', 'nav.nextArticle'], label: 'reader.hint.move' },
    { ids: ['nav.focusReader'], label: 'reader.hint.read' },
    { ids: ['help.toggle'], label: 'reader.hint.help' },
];

/**
 * What the header needs — carried by both the full `ArticleDto` and the list
 * row's `ArticleSummaryDto`, so the header can render before the body lands.
 */
type ReaderHead = Pick<
    ArticleDto,
    'id' | 'feed_id' | 'title' | 'source_url' | 'author' | 'published_at' | 'word_count'
>;

/** Body placeholder line widths: a few paragraphs of ragged prose. */
const BODY_SKELETON = ['100%', '96%', '98%', '72%', '', '100%', '94%', '88%', '97%', '54%'];

/** A stable per-feed hue for the monogram chip, derived from the title. */
function feedHue(label: string): number {
    let hash = 0;
    for (let i = 0; i < label.length; i += 1) {
        hash = (hash * 31 + label.charCodeAt(i)) % 360;
    }
    return hash;
}

/** The mounted scroll body (re-bound per article by the `{#key}` below). */
let scrollBody = $state<HTMLElement | null>(null);

/**
 * The reader body owns the keyboard. Switching articles (j/k, Space at the
 * end) remounts the body, dropping DOM focus onto `<body>`; while this holds,
 * the new body takes it back. Any focus landing outside the body clears it,
 * and so does the selection clearing (`g a`, a feed hop, Esc): the body
 * unmounts with no successor and focus drops to `<body>` without a
 * `focusin`, so the list's keys own the page again.
 */
let holdFocus = false;
/** A `focusReader` request not yet honoured (the article was still loading). */
let focusPending = false;
let seenFocusNonce = untrack(() => selectionStore.readerFocusNonce);

$effect(() => {
    const nonce = selectionStore.readerFocusNonce;
    const body = scrollBody;
    if (nonce !== seenFocusNonce) {
        seenFocusNonce = nonce;
        focusPending = true;
        holdFocus = true;
    }
    if (selectionStore.selectedArticleId === null) {
        holdFocus = false;
        focusPending = false;
        return;
    }
    if (body === null || !holdFocus) {
        return;
    }
    const active = document.activeElement;
    // Reclaim only a request or focus that was dropped — never steal it from
    // the search field or a dialog that took it in the meantime.
    if (focusPending || active === null || active === document.body) {
        focusPending = false;
        body.focus();
    }
});

$effect(() => {
    const body = scrollBody;
    return body === null ? undefined : registerReaderBody(body);
});

function onWindowFocusIn(event: FocusEvent): void {
    const target = event.target;
    const inside = scrollBody !== null && target instanceof Node && scrollBody.contains(target);
    holdFocus = inside;
    if (!inside) {
        focusPending = false;
    }
}

/**
 * The reader's pane keys (← / Esc back to the list). ← only from the body
 * itself — a focused link or media control inside keeps its own arrows; Esc
 * from anywhere in it, and it stops there so the shell's Escape step-back
 * doesn't also deselect the article.
 */
function onBodyKeydown(event: KeyboardEvent): void {
    if (event.ctrlKey || event.altKey || event.metaKey) {
        return;
    }
    if (event.key !== 'Escape' && event.target !== event.currentTarget) {
        return;
    }
    const id = paneShortcut('reader', eventKeyName(event));
    if (id === null) {
        return;
    }
    event.preventDefault();
    event.stopPropagation();
    handleShortcut(id);
}

let showTypography = $state(false);
let typographyToggle = $state<HTMLButtonElement>();
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

/**
 * The selected article's query, created in a $derived — never first-created
 * inside a template expression. Svelte excludes state created during a reaction
 * from that reaction's own dependencies, so a template branch that both created
 * and read this query would go blind to the article loading in (it would stick
 * on the initial null and show "missing"). The helpers below only *read* the
 * already-created query, so every template reaction tracks its fields.
 */
const selectedId = $derived(selectionStore.selectedArticleId);

const articleQuery = $derived.by(() => {
    const articleId = selectedId;
    return articleId === null
        ? null
        : ensureQuery(queryKeys.article(articleId), () => commands.getArticle(articleId));
});

/** The selected article's flags, created outside the template (see above). */
const stateQuery = $derived.by(() => {
    const articleId = selectedId;
    return articleId === null
        ? null
        : ensureQuery(queryKeys.articleState(articleId), () => commands.getArticleState(articleId));
});

function article(): ArticleDto | null {
    return articleQuery?.data ?? null;
}

function articleLoaded(): boolean {
    return articleQuery?.loaded ?? false;
}

function failure(): CommandError | null {
    return articleQuery?.error ?? null;
}

function failureMessage(): string {
    const error = failure();
    return error === null ? '' : commandErrorMessage(error);
}

/** The header source: the full article once loaded, else the list row. */
function head(): ReaderHead | null {
    return article() ?? selectionStore.selectedArticle;
}

function flags(): ArticleStateDto | null {
    return stateQuery?.data ?? null;
}

function currentFeed(current: Pick<ReaderHead, 'feed_id'>): FeedDto | null {
    const feedId = current.feed_id;
    if (feedId === null) {
        return null;
    }
    return feedsStore.feeds.find((candidate) => candidate.id === feedId) ?? null;
}

function feedTitle(current: Pick<ReaderHead, 'feed_id'>): string | null {
    const feed = currentFeed(current);
    if (feed === null) {
        return null;
    }
    return feed.title ?? feed.url;
}

// Displaying an article marks it read — once per selection, not per render,
// and only once it has actually loaded. A failed fetch must not silently mark
// an article the reader never managed to show.
let lastOpenedId: number | null = null;
$effect(() => {
    const articleId = selectionStore.selectedArticleId;
    if (articleId !== null && articleId !== lastOpenedId && articleQuery?.data != null) {
        lastOpenedId = articleId;
        void markReadOnOpen(articleId);
    }
});

// Prefetch the neighbours' bodies once the selected one has settled (so a
// fast j-burst doesn't queue fetches for rows it is only passing through).
// Centred on the selection, or on its last position when an auto-mark-read
// dropped it from the window. No selection evicts every prefetched body.
$effect(() => {
    const articleId = selectedId;
    if (articleId === null) {
        articlePrefetcher.prefetch([], null);
        return;
    }
    if (articleQuery?.loaded !== true) {
        return;
    }
    const items = articlesStore.activeItems;
    const index = selectionStore.selectedIndex;
    const center = index >= 0 ? index : Math.min(selectionStore.lastPresentIndex, items.length - 1);
    articlePrefetcher.prefetch(neighbourIds(items, center, articleId), articleId);
});

function openSource(event: MouseEvent, current: ReaderHead): void {
    event.preventDefault();
    void openInBrowser(current.id, current.source_url);
}
</script>

<svelte:window onfocusin={onWindowFocusIn} />

<div class="reader">
    {#if selectedId === null}
        <EmptyState icon="sparkle" title={t('reader.empty')}>
            {#snippet hint()}
                {#each HINT as part, index (part.label)}
                    {#if index > 0}
                        <span class="hint-sep" aria-hidden="true">·</span>
                    {/if}
                    <span class="hint-part">
                        <!-- Keys of different actions sit side by side (↑↓);
                             alternatives for one action take the help
                             overlay's `/` (Enter/→), so they never read as a
                             sequence. -->
                        {#each part.ids as id (id)}{#each advertisedKeys(id) as key, position (key)}{#if position > 0}<span
                                        class="hint-alt">/</span
                                    >{/if}<kbd>{keyLabel(key)}</kbd>{/each}{/each}
                        {t(part.label)}
                    </span>
                {/each}
            {/snippet}
        </EmptyState>
    {:else}
        {@const current = article()}
        {@const header = head()}
        {#if current === null && failure() !== null}
            <!-- No article to preserve: the load itself failed. -->
            <div class="reader-status">
                <p class="error" role="alert">{failureMessage()}</p>
            </div>
        {:else if current === null && articleLoaded()}
            <div class="reader-status">
                <p>{t('reader.missing')}</p>
            </div>
        {:else}
            {#if current !== null && failure() !== null}
                <!-- Stale-while-revalidate: a refetch failed but the article is
                     still in hand — keep showing it under a non-destructive
                     banner instead of blanking a populated reader (parity with
                     the article list's refetch banner). -->
                <p class="reader-banner" role="alert">{failureMessage()}</p>
            {/if}
            <!-- Mounted for the whole selection — including while the body is
                 still loading — so j/k never unmounts it or resets its state. -->
            <div class="reader-toolbar" role="toolbar" aria-label={t('reader.toolbar')}>
                <button
                    class="tool"
                    class:on={flags()?.read === true}
                    aria-pressed={flags()?.read === true}
                    aria-label={flags()?.read === true ? t('reader.action.markUnread') : t('reader.action.markRead')}
                    use:tooltip={{
                        text: flags()?.read === true ? t('reader.action.markUnread') : t('reader.action.markRead'),
                        kbd: shortcutKbd('article.toggleRead'),
                    }}
                    onclick={() => void toggleRead(selectedId)}
                >
                    <Icon name="check" />
                </button>
                <button
                    class="tool star"
                    class:on={flags()?.starred === true}
                    aria-pressed={flags()?.starred === true}
                    aria-label={flags()?.starred === true ? t('reader.action.unstar') : t('reader.action.star')}
                    use:tooltip={{
                        text: flags()?.starred === true ? t('reader.action.unstar') : t('reader.action.star'),
                        kbd: shortcutKbd('article.toggleStar'),
                    }}
                    onclick={() => void toggleStar(selectedId)}
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
                        kbd: shortcutKbd('article.toggleReadLater'),
                    }}
                    onclick={() => void toggleReadLater(selectedId)}
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
                        kbd: shortcutKbd('article.toggleArchived'),
                    }}
                    onclick={() => void toggleArchived(selectedId)}
                >
                    <Icon name="archive" />
                </button>
                <span class="toolbar-spring"></span>
                <button
                    class="tool"
                    aria-label={t('reader.action.promote')}
                    use:tooltip={{ text: t('reader.action.promote'), kbd: shortcutKbd('article.promote') }}
                    onclick={() => void promoteSelected()}
                >
                    <Icon name="save" />
                </button>
                <div class="typography-anchor">
                    <button
                        bind:this={typographyToggle}
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
                        <!-- Focus enters on mount; Escape or an outside press
                             closes it (window-level, so it works even when focus
                             never reached inside). The toggle is excluded so its
                             own click toggles closed cleanly. -->
                        <div
                            class="typography-popover"
                            role="dialog"
                            aria-label={t('reader.action.typography')}
                            tabindex="-1"
                            use:dismissable={{
                                onclose: () => {
                                    showTypography = false;
                                },
                                ignore: () => typographyToggle,
                                returnFocus: () => typographyToggle,
                            }}
                        >
                            <TypographyControls />
                        </div>
                    {/if}
                </div>
                <button
                    class="tool"
                    disabled={hydrating || current === null}
                    aria-busy={hydrating}
                    aria-label={t('reader.action.loadFull')}
                    use:tooltip={t('reader.action.loadFull')}
                    onclick={() => void hydrate(selectedId)}
                >
                    <Icon name="article" />
                </button>
                <button
                    class="tool"
                    disabled={header === null}
                    aria-label={t('reader.action.openInBrowser')}
                    use:tooltip={t('reader.action.openInBrowser')}
                    onclick={() => {
                        if (header !== null) {
                            void openInBrowser(selectedId, header.source_url);
                        }
                    }}
                >
                    <Icon name="external" />
                </button>
            </div>
            <!-- Key on the selection so switching articles remounts the
                 scroll body at the top instead of inheriting the previous scroll
                 offset; the body landing within one selection does not. -->
            {#key selectedId}
            <!-- A focusable region named by the article title: Tab, a click,
                 or Enter / → from the list land here, so the platform's native
                 key scrolling works; ← / Esc hand the keyboard back. A focusable
                 scrollable region (tabindex 0, named) is the WAI-ARIA / axe
                 `scrollable-region-focusable` pattern, but svelte's a11y table
                 treats region as static — hence the two targeted ignores. -->
            <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <div
                class="reader-scroll"
                role="region"
                aria-labelledby={header !== null ? `reader-title-${selectedId}` : undefined}
                aria-label={header === null ? t('app.loading') : undefined}
                tabindex="0"
                data-reading-theme={typographyStore.readingTheme}
                bind:this={scrollBody}
                onkeydown={onBodyKeydown}
            >
                <article
                    class="reader-article"
                    class:justify={typographyStore.textAlign === 'justify'}
                    class:hyphenate={typographyStore.hyphenate}
                    style:font-size="{typographyStore.fontSize}px"
                    style:line-height={typographyStore.lineHeight}
                    style:max-width="{typographyStore.measure}px"
                    style:font-family={typographyStore.readerFontStack}
                    style:font-weight={typographyStore.fontWeight}
                    style:letter-spacing="{typographyStore.letterSpacing}em"
                    style:--para-spacing={typographyStore.paragraphSpacing}
                >
                    {#if header !== null}
                        <header class="reader-header">
                            {#if feedTitle(header) !== null}
                                {@const label = feedTitle(header) ?? ''}
                                <div class="reader-kicker">
                                    <span
                                        class="feed-mono"
                                        style:--mono-hue={feedHue(label)}
                                        aria-hidden="true">{label.slice(0, 1).toUpperCase()}</span
                                    >
                                    <span class="feed-name">{label}</span>
                                </div>
                            {/if}
                            <h1 class="reader-title" id="reader-title-{selectedId}">
                                <!-- external URL: the click is intercepted and routed to the URL-scoped opener -->
                                <!-- eslint-disable-next-line svelte/no-navigation-without-resolve -->
                                <a href={header.source_url} onclick={(event) => openSource(event, header)}>
                                    {header.title}
                                </a>
                            </h1>
                            <p class="reader-meta">
                                {#if header.author !== null}
                                    <span>{header.author}</span>
                                {/if}
                                {#if header.published_at !== null}
                                    {@const published = formatIntlDateTime(new Date(header.published_at))}
                                    {#if published !== ''}
                                        <span>{published}</span>
                                    {/if}
                                {/if}
                                {#if header.word_count !== null}
                                    <span>{t('reader.meta.words', { count: header.word_count })}</span>
                                {/if}
                            </p>
                            <ArticleTags articleId={header.id} />
                        </header>
                    {:else}
                        <!-- Selected outside the loaded window: no row to
                             borrow the header from yet. -->
                        <div class="reader-header header-skeleton" aria-hidden="true">
                            <Skeleton width="30%" height="0.85rem" />
                            <Skeleton width="85%" height="1.6rem" />
                            <Skeleton width="45%" height="0.85rem" />
                        </div>
                    {/if}
                    {#if current !== null}
                        <div class="reader-body">
                            <ViewModeDispatch article={current} feed={currentFeed(current)} />
                        </div>
                    {:else}
                        <div class="body-skeleton" aria-busy="true">
                            {#each BODY_SKELETON as width, line (line)}
                                {#if width === ''}
                                    <span class="paragraph-gap"></span>
                                {:else}
                                    <Skeleton {width} height="0.8em" />
                                {/if}
                            {/each}
                            <span class="sr-only">{t('app.loading')}</span>
                        </div>
                    {/if}
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

    /* Non-destructive refetch-failure strip above the retained article. */
    .reader-banner {
        flex: 0 0 auto;
        padding: var(--space-2) var(--space-4);
        margin: var(--space-2) var(--space-3) 0;
        border-radius: var(--radius-md);
        background: var(--error-bg);
        color: var(--error-text);
        font-size: var(--text-xs);
    }

    /* Empty-state hint: one unbreakable group per action. */
    .hint-part {
        white-space: nowrap;
    }

    .hint-sep {
        margin: 0 var(--space-1);
    }

    .hint-alt {
        font-size: var(--text-xs);
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

    /* The starred state is gold everywhere — matches the list row's star, not
       the accent blue the other toggles use. */
    .tool.star.on {
        color: var(--warning-text);
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
        /* A short fade on every article switch (the scroll body is keyed on
           the selection) and again as a body lands over its skeleton, so j/k
           reads as a cross-fade rather than a hard cut. */
        animation: reader-fade-in var(--dur-fast) var(--ease);
    }

    .reader-body {
        animation: reader-fade-in var(--dur-base) var(--ease);
    }

    @keyframes reader-fade-in {
        from {
            opacity: 0;
        }
        to {
            opacity: 1;
        }
    }

    /* The global reduced-motion rule already collapses durations; stop the
       fades outright too so nothing even flickers. */
    @media (prefers-reduced-motion: reduce) {
        .reader-scroll,
        .reader-body {
            animation: none;
        }
    }

    /* Loading placeholders shaped like the content they stand in for. */
    .header-skeleton {
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .body-skeleton {
        display: flex;
        flex-direction: column;
        gap: 0.6em;
    }

    .paragraph-gap {
        height: 0.6em;
    }

    /* Space / Shift+Space page with a glide, unless motion is reduced. */
    @media (prefers-reduced-motion: no-preference) {
        .reader-scroll {
            scroll-behavior: smooth;
        }
    }

    /* Focused for ↑/↓ scrolling (Enter / → from the list): a soft inset frame
       rather than a hard ring around the whole column. */
    .reader-scroll:focus-visible {
        outline: none;
        box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent), transparent 66%);
    }

    /* Justified reading: only the prose body (not the header), with hyphenation
       so justification doesn't open rivers of whitespace. */
    .reader-article.justify :global(.sanitized-content) {
        text-align: justify;
        hyphens: auto;
    }

    /* Independent hyphenation toggle (applies to left-aligned prose too). */
    .reader-article.hyphenate :global(.sanitized-content) {
        hyphens: auto;
    }

    /* Reading-surface tints, independent of the app theme: a warm paper ground
       with dark ink so long reads stay comfortable on any theme. The tint sits
       on the scroll surface; the ink tokens are forced directly (`--ink` is a
       :root color-mix that inherits its computed value, so overriding `--fg`
       alone would not repaint the prose) while links keep their accent. */
    .reader-scroll[data-reading-theme='sepia'] {
        background: #f1e5cf;
        --ink: #43392b;
        --ink-soft: #6b5b45;
        --fg: #43392b;
        --fg-muted: #5c4f3b;
        --fg-subtle: #7a6a52;
    }

    .reader-scroll[data-reading-theme='paper'] {
        background: #faf6ee;
        --ink: #2b2b2b;
        --ink-soft: #555555;
        --fg: #2b2b2b;
        --fg-muted: #4a4a4a;
        --fg-subtle: #6b6b6b;
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
        /* Floor: even in a very narrow reader, a long title wraps between
           graphemes rather than breaking mid-word ("Identif/ied"). */
        overflow-wrap: anywhere;
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
