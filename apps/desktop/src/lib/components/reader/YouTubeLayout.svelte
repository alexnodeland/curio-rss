<script lang="ts">
/**
 * YouTube layout: the click-to-load facade above the feed-provided
 * description, both from RSS-native data only. No API call, no yt-dlp — the
 * video id is parsed out of the article's own source URL. When the URL
 * carries no recognisable id the layout degrades to the sanitized body.
 */
import type { ArticleDto } from '$lib/bindings';
import { youTubeVideoId } from '$lib/reader/view-mode';
import SanitizedHtml from './SanitizedHtml.svelte';
import YouTubeFacade from './YouTubeFacade.svelte';

let { article }: { article: ArticleDto } = $props();

const videoId = $derived(youTubeVideoId(article.source_url));
</script>

<div class="youtube-layout">
    {#if videoId !== null}
        <YouTubeFacade {videoId} title={article.title} />
    {/if}
    <SanitizedHtml html={article.content_html} />
</div>

<style>
    .youtube-layout {
        display: flex;
        flex-direction: column;
    }
</style>
