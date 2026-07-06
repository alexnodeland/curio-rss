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
        margin-bottom: var(--space-4);
        padding-bottom: var(--space-3);
        border-bottom: 1px solid var(--border-subtle);
    }

    .subreddit {
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-xl);
        background: var(--bg-tertiary);
        color: var(--fg-muted);
        font-size: 0.8125rem;
        font-weight: 600;
    }

    .discussion {
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--accent);
        border: 1px solid var(--accent-muted);
        font-size: 0.8125rem;
    }

    .discussion:hover {
        background: var(--accent-muted);
    }
</style>
