<script lang="ts">
/**
 * The reader body dispatcher: picks the article / Reddit / YouTube layout
 * from feed-URL + source-URL heuristics (`detectViewMode`) and renders it.
 * Every branch ultimately renders through SanitizedHtml — the layouts add
 * chrome around RSS-native data, never a second HTML-injection site. The
 * `data-view-mode` attribute is the seam the component tests assert on.
 */
import type { ArticleDto, FeedDto } from '$lib/bindings';
import { detectViewMode } from '$lib/reader/view-mode';
import RedditLayout from './RedditLayout.svelte';
import SanitizedHtml from './SanitizedHtml.svelte';
import YouTubeLayout from './YouTubeLayout.svelte';

let { article, feed }: { article: ArticleDto; feed: FeedDto | null } = $props();

const mode = $derived(detectViewMode(feed?.url ?? null, article.source_url));
</script>

<div class="view-mode" data-view-mode={mode}>
    {#if mode === 'youtube'}
        <YouTubeLayout {article} />
    {:else if mode === 'reddit'}
        <RedditLayout {article} />
    {:else}
        <SanitizedHtml html={article.content_html} />
    {/if}
</div>
