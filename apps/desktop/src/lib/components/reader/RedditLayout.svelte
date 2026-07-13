<script lang="ts">
/**
 * Reddit layout: a subreddit-branded post header carrying the RSS-native
 * structure of the item — subreddit, author, and the distinct comments vs.
 * link affordances parsed from the feed's own footer (`parseRedditPost`) —
 * over the feed-provided body with that boilerplate footer pruned. The
 * layout itself does no fetching; "Load full post" delegates to the core's
 * load-full path, which (with the `enrich-reddit` feature, D14) upgrades
 * the body via the post's public JSON — full selftext, galleries, images.
 * Outbound clicks route through the URL-scoped opener.
 */
import Icon from '$components/common/Icon.svelte';
import type { ArticleDto } from '$lib/bindings';
import { t } from '$lib/i18n';
import { parseRedditPost, stripRedditFooter, subredditOf } from '$lib/reader/view-mode';
import { loadFullArticle } from '$lib/state/actions';
import { openExternal } from '$lib/utils/external';
import SanitizedHtml from './SanitizedHtml.svelte';

let { article }: { article: ArticleDto } = $props();

let loadingFull = $state(false);

/** Upgrades the stub body to the full post (core-side enrichment). */
async function loadFull(): Promise<void> {
    if (loadingFull) {
        return;
    }
    loadingFull = true;
    try {
        await loadFullArticle(article.id);
    } finally {
        loadingFull = false;
    }
}

const subreddit = $derived(subredditOf(article.source_url));
const post = $derived(parseRedditPost(article.content_html, article.source_url));
const body = $derived(stripRedditFooter(article.content_html));

/** The registrable host of the external link, for the link-post preview. */
function linkHost(url: string): string {
    try {
        return new URL(url).hostname.replace(/^www\./, '');
    } catch {
        return url;
    }
}
</script>

<div class="reddit-layout">
    <div class="reddit-post">
        <div class="post-head">
            {#if subreddit !== null}
                <span class="sub-avatar" aria-hidden="true">
                    <Icon name="message" size={15} strokeWidth={2} />
                </span>
                <div class="post-ident">
                    <span class="subreddit">r/{subreddit}</span>
                    {#if post.author !== null}
                        <span class="post-author">{t('reddit.postedBy', { author: post.author })}</span>
                    {/if}
                </div>
            {/if}
            <span class="reddit-wordmark" aria-hidden="true">reddit</span>
        </div>

        {#if post.isLinkPost && post.linkUrl !== null}
            {@const url = post.linkUrl}
            <button class="link-card" type="button" onclick={() => void openExternal(url)}>
                <span class="link-card-icon" aria-hidden="true"><Icon name="link" size={16} /></span>
                <span class="link-card-body">
                    <span class="link-card-host truncate">{linkHost(url)}</span>
                    <span class="link-card-url truncate">{url}</span>
                </span>
                <span class="link-card-open" aria-hidden="true"><Icon name="external" size={15} /></span>
            </button>
        {/if}

        <SanitizedHtml html={body} />

        <div class="post-actions">
            <button
                class="post-action primary"
                type="button"
                onclick={() => void openExternal(post.commentsUrl)}
            >
                <Icon name="message" size={16} />
                <span>{t('reader.reddit.openThread')}</span>
            </button>
            <button
                class="post-action"
                type="button"
                disabled={loadingFull}
                onclick={() => void loadFull()}
            >
                <Icon name="refresh" size={16} />
                <span>{loadingFull ? t('reader.loadingFull') : t('reader.reddit.loadFull')}</span>
            </button>
            {#if post.isLinkPost && post.linkUrl !== null}
                {@const url = post.linkUrl}
                <button class="post-action" type="button" onclick={() => void openExternal(url)}>
                    <Icon name="link" size={16} />
                    <span>{t('reader.reddit.openLink')}</span>
                </button>
            {/if}
        </div>
    </div>
</div>

<style>
    .reddit-layout {
        display: flex;
        flex-direction: column;
    }

    .reddit-post {
        display: flex;
        flex-direction: column;
    }

    .post-head {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding-bottom: var(--space-4);
        margin-bottom: var(--space-5);
        border-bottom: 1px solid var(--hairline);
    }

    .sub-avatar {
        display: grid;
        place-items: center;
        width: 34px;
        height: 34px;
        flex: 0 0 auto;
        border-radius: 50%;
        color: #fff;
        background: linear-gradient(140deg, #ff6a33, #ff4500);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, #ff4500, #000 12%);
    }

    .post-ident {
        display: flex;
        flex-direction: column;
        min-width: 0;
        gap: 1px;
    }

    .subreddit {
        font-family: var(--font-family);
        font-size: var(--text-md);
        font-weight: 650;
        color: var(--fg);
    }

    .post-author {
        font-family: var(--font-family);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .reddit-wordmark {
        margin-left: auto;
        font-family: var(--font-family);
        font-size: var(--text-xs);
        font-weight: 700;
        letter-spacing: var(--tracking-wide);
        color: #ff4500;
        opacity: 0.85;
    }

    /* Link-post preview card — the off-site target, click-through to open. */
    .link-card {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        width: 100%;
        margin-bottom: var(--space-5);
        padding: var(--space-3) var(--space-4);
        border-radius: var(--radius-lg);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        color: var(--fg);
        text-align: left;
        transition:
            background var(--dur-fast) var(--ease),
            border-color var(--dur-fast) var(--ease);
    }

    .link-card:hover {
        background: var(--hover);
        border-color: var(--hairline-strong);
    }

    .link-card-icon {
        display: grid;
        place-items: center;
        width: 32px;
        height: 32px;
        flex: 0 0 auto;
        border-radius: var(--radius-md);
        color: var(--accent);
        background: var(--selected);
    }

    .link-card-body {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .link-card-host {
        font-family: var(--font-family);
        font-size: var(--text-md);
        font-weight: 560;
    }

    .link-card-url {
        font-family: var(--font-family);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .link-card-open {
        flex: 0 0 auto;
        color: var(--fg-subtle);
    }

    .post-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        margin-top: var(--space-6);
        padding-top: var(--space-4);
        border-top: 1px solid var(--hairline);
    }

    .post-action {
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-pill);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        color: var(--fg-muted);
        font-family: var(--font-family);
        font-size: var(--text-sm);
        font-weight: 560;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .post-action:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .post-action.primary {
        background: color-mix(in srgb, #ff4500, transparent 88%);
        border-color: color-mix(in srgb, #ff4500, transparent 72%);
        color: #ff6a33;
    }

    .post-action.primary:hover {
        background: color-mix(in srgb, #ff4500, transparent 80%);
        color: #ff7a48;
    }
</style>
