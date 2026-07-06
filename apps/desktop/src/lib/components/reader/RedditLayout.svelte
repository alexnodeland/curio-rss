<script lang="ts">
/**
 * Reddit layout: the subreddit chip (parsed from the source URL) and an
 * open-the-thread affordance above the feed-provided body — RSS-native data
 * only, no unauthenticated Reddit JSON (ToS-gray, rate-limited; D8). The
 * "open discussion" click routes through the URL-scoped opener, never the
 * webview, like every other outbound link.
 */
import type { ArticleDto } from '$lib/bindings';
import { t } from '$lib/i18n';
import { subredditOf } from '$lib/reader/view-mode';
import { openExternal } from '$lib/utils/external';
import SanitizedHtml from './SanitizedHtml.svelte';

let { article }: { article: ArticleDto } = $props();

const subreddit = $derived(subredditOf(article.source_url));
</script>

<div class="reddit-layout">
    <div class="reddit-bar">
        {#if subreddit !== null}
            <span class="subreddit">r/{subreddit}</span>
        {/if}
        <button
            class="discussion"
            type="button"
            onclick={() => void openExternal(article.source_url)}
        >
            {t('reader.reddit.openThread')}
        </button>
    </div>
    <SanitizedHtml html={article.content_html} />
</div>

<style>
    .reddit-layout {
        display: flex;
        flex-direction: column;
    }

    .reddit-bar {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        margin-bottom: var(--space-5);
        padding-bottom: var(--space-4);
        border-bottom: 1px solid var(--hairline);
    }

    .subreddit {
        padding: 3px var(--space-3);
        border-radius: var(--radius-pill);
        background: color-mix(in srgb, #ff4500, transparent 88%);
        color: #ff6a33;
        font-family: var(--font-family);
        font-size: var(--text-sm);
        font-weight: 650;
    }

    .discussion {
        margin-left: auto;
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--selected);
        color: var(--accent);
        font-family: var(--font-family);
        font-size: var(--text-sm);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .discussion:hover {
        background: var(--selected-strong);
    }
</style>
