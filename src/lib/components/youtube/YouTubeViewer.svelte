<script lang="ts">
    import type { Article, YouTubeMetadata, YouTubeComment as YTComment } from '$lib/types';
    import { fetchYouTubeMetadata, fetchYouTubeComments } from '$lib/api';
    import { formatDuration, formatNumber, formatRelativeTime } from '$lib/utils/format';
    import YouTubeComment from './YouTubeComment.svelte';

    export let article: Article;

    let metadata: YouTubeMetadata | null = null;
    let comments: YTComment[] = [];
    let isLoadingMetadata = true;
    let isLoadingComments = false;
    let error = '';
    let showComments = false;

    // Extract video ID from article URL
    function extractVideoId(url: string): string | null {
        const patterns = [
            /youtube\.com\/watch\?v=([^&]+)/,
            /youtu\.be\/([^?]+)/,
            /youtube\.com\/embed\/([^?]+)/,
        ];

        for (const pattern of patterns) {
            const match = url.match(pattern);
            if (match) return match[1];
        }
        return null;
    }

    async function loadMetadata() {
        if (!article.url) {
            error = 'No video URL available';
            isLoadingMetadata = false;
            return;
        }

        isLoadingMetadata = true;
        error = '';

        try {
            metadata = await fetchYouTubeMetadata(article.url);
        } catch (e) {
            error = e instanceof Error ? e.message : 'Failed to load video metadata';
        } finally {
            isLoadingMetadata = false;
        }
    }

    async function loadComments() {
        if (!article.url || isLoadingComments) return;

        isLoadingComments = true;

        try {
            comments = await fetchYouTubeComments(article.url);
            showComments = true;
        } catch (e) {
            // Comments might not be available - not a critical error
            console.error('Failed to load comments:', e);
        } finally {
            isLoadingComments = false;
        }
    }

    function toggleComments() {
        if (!showComments && comments.length === 0) {
            loadComments();
        } else {
            showComments = !showComments;
        }
    }

    function formatUploadDate(dateStr: string): string {
        // YouTube returns date as YYYYMMDD
        if (dateStr.length === 8) {
            const year = dateStr.slice(0, 4);
            const month = dateStr.slice(4, 6);
            const day = dateStr.slice(6, 8);
            const date = new Date(`${year}-${month}-${day}`);
            return formatRelativeTime(date.toISOString());
        }
        return dateStr;
    }

    function getBestThumbnail(): string | null {
        if (!metadata?.thumbnails.length) return article.thumbnail_url;

        // Prefer maxres, then hq, then any available
        const priorities = ['maxresdefault', 'sddefault', 'hqdefault', 'mqdefault'];
        for (const id of priorities) {
            const thumb = metadata.thumbnails.find(t => t.id === id);
            if (thumb) return thumb.url;
        }
        return metadata.thumbnails[0]?.url || article.thumbnail_url;
    }

    function openInYouTube() {
        if (article.url) {
            window.open(article.url, '_blank');
        }
    }

    // Load metadata when article changes
    $: if (article) loadMetadata();

    $: videoId = article.url ? extractVideoId(article.url) : null;
</script>

<div class="youtube-viewer">
    {#if isLoadingMetadata}
        <div class="loading">
            <div class="loading-spinner"></div>
            <span>Loading video info...</span>
        </div>
    {:else if error}
        <div class="error">
            <p>{error}</p>
            <button on:click={loadMetadata}>Try Again</button>
        </div>
    {:else if metadata}
        <div class="video-container">
            {#if videoId}
                <div class="video-embed">
                    <iframe
                        src="https://www.youtube.com/embed/{videoId}"
                        title={metadata.title}
                        frameborder="0"
                        allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
                        allowfullscreen
                    ></iframe>
                </div>
            {:else}
                <div class="video-thumbnail">
                    <img src={getBestThumbnail()} alt={metadata.title} />
                    <button class="play-button" on:click={openInYouTube}>
                        <svg xmlns="http://www.w3.org/2000/svg" width="48" height="48" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M8 5v14l11-7z"/>
                        </svg>
                    </button>
                </div>
            {/if}
        </div>

        <div class="video-info">
            <h1 class="video-title">{metadata.title}</h1>

            <div class="video-stats">
                {#if metadata.view_count !== null}
                    <span class="stat">
                        {formatNumber(metadata.view_count)} views
                    </span>
                {/if}
                <span class="stat">
                    {formatUploadDate(metadata.upload_date)}
                </span>
                {#if metadata.like_count !== null}
                    <span class="stat likes">
                        <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                        </svg>
                        {formatNumber(metadata.like_count)}
                    </span>
                {/if}
                <span class="stat duration">
                    {formatDuration(metadata.duration)}
                </span>
            </div>

            <div class="channel-info">
                <div class="channel-details">
                    <a href={metadata.channel_url} target="_blank" rel="noopener noreferrer" class="channel-name">
                        {metadata.channel}
                    </a>
                    {#if metadata.channel_follower_count}
                        <span class="subscriber-count">
                            {formatNumber(metadata.channel_follower_count)} subscribers
                        </span>
                    {/if}
                </div>
                <button class="open-btn" on:click={openInYouTube}>
                    Open in YouTube
                </button>
            </div>

            {#if metadata.chapters && metadata.chapters.length > 0}
                <div class="chapters-section">
                    <h3>Chapters</h3>
                    <div class="chapters-list">
                        {#each metadata.chapters as chapter}
                            <div class="chapter">
                                <span class="chapter-time">{formatDuration(chapter.start_time)}</span>
                                <span class="chapter-title">{chapter.title}</span>
                            </div>
                        {/each}
                    </div>
                </div>
            {/if}

            <div class="description-section">
                <h3>Description</h3>
                <div class="description">
                    {metadata.description}
                </div>
            </div>

            {#if metadata.tags.length > 0}
                <div class="tags-section">
                    <div class="tags">
                        {#each metadata.tags.slice(0, 10) as tag}
                            <span class="tag">#{tag}</span>
                        {/each}
                    </div>
                </div>
            {/if}

            <div class="comments-section">
                <button class="comments-toggle" on:click={toggleComments}>
                    <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                    </svg>
                    {#if metadata.comment_count !== null}
                        {formatNumber(metadata.comment_count)} Comments
                    {:else}
                        Comments
                    {/if}
                    {#if isLoadingComments}
                        <span class="loading-inline"></span>
                    {/if}
                </button>

                {#if showComments}
                    <div class="comments-list">
                        {#if comments.length === 0}
                            <p class="no-comments">
                                {isLoadingComments ? 'Loading comments...' : 'No comments available'}
                            </p>
                        {:else}
                            {#each comments as comment (comment.id)}
                                <YouTubeComment {comment} />
                            {/each}
                        {/if}
                    </div>
                {/if}
            </div>
        </div>
    {/if}
</div>

<style>
    .youtube-viewer {
        flex: 1;
        height: 100%;
        overflow-y: auto;
        background: var(--bg);
    }

    .loading,
    .error {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-3);
        padding: var(--space-8);
        color: var(--fg-muted);
    }

    .loading-spinner {
        width: 32px;
        height: 32px;
        border: 3px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 1s linear infinite;
    }

    @keyframes spin {
        to { transform: rotate(360deg); }
    }

    .error {
        color: var(--error);
    }

    .error button {
        padding: var(--space-2) var(--space-4);
        background: var(--accent);
        color: var(--accent-fg);
        border-radius: var(--radius-md);
    }

    .video-container {
        background: black;
    }

    .video-embed {
        position: relative;
        padding-bottom: 56.25%;
        height: 0;
    }

    .video-embed iframe {
        position: absolute;
        top: 0;
        left: 0;
        width: 100%;
        height: 100%;
    }

    .video-thumbnail {
        position: relative;
        display: flex;
        align-items: center;
        justify-content: center;
    }

    .video-thumbnail img {
        width: 100%;
        max-height: 60vh;
        object-fit: contain;
    }

    .play-button {
        position: absolute;
        padding: var(--space-4);
        background: rgba(0, 0, 0, 0.7);
        color: white;
        border-radius: 50%;
    }

    .play-button:hover {
        background: rgba(255, 0, 0, 0.9);
    }

    .video-info {
        padding: var(--space-4);
    }

    .video-title {
        font-size: 20px;
        font-weight: 600;
        line-height: 1.3;
        color: var(--fg);
        margin-bottom: var(--space-3);
    }

    .video-stats {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-3);
        margin-bottom: var(--space-4);
        font-size: 14px;
        color: var(--fg-muted);
    }

    .stat {
        display: flex;
        align-items: center;
        gap: var(--space-1);
    }

    .stat.likes svg {
        color: var(--success);
    }

    .channel-info {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-3);
        background: var(--bg-secondary);
        border-radius: var(--radius-md);
        margin-bottom: var(--space-4);
    }

    .channel-details {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .channel-name {
        font-weight: 600;
        color: var(--fg);
    }

    .channel-name:hover {
        color: var(--accent);
    }

    .subscriber-count {
        font-size: 12px;
        color: var(--fg-muted);
    }

    .open-btn {
        padding: var(--space-2) var(--space-3);
        font-size: 13px;
        background: var(--accent);
        color: var(--accent-fg);
        border-radius: var(--radius-md);
    }

    .open-btn:hover {
        background: var(--accent-hover);
    }

    .chapters-section,
    .description-section,
    .tags-section {
        margin-bottom: var(--space-4);
    }

    .chapters-section h3,
    .description-section h3 {
        font-size: 14px;
        font-weight: 600;
        color: var(--fg);
        margin-bottom: var(--space-2);
    }

    .chapters-list {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        padding: var(--space-2);
        background: var(--bg-secondary);
        border-radius: var(--radius-md);
        max-height: 200px;
        overflow-y: auto;
    }

    .chapter {
        display: flex;
        gap: var(--space-3);
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-sm);
        font-size: 13px;
    }

    .chapter:hover {
        background: var(--bg-tertiary);
    }

    .chapter-time {
        color: var(--accent);
        font-family: var(--font-mono);
        font-size: 12px;
        min-width: 48px;
    }

    .chapter-title {
        color: var(--fg);
    }

    .description {
        white-space: pre-wrap;
        font-size: 14px;
        line-height: 1.6;
        color: var(--fg);
        max-height: 200px;
        overflow-y: auto;
        padding: var(--space-3);
        background: var(--bg-secondary);
        border-radius: var(--radius-md);
    }

    .tags {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
    }

    .tag {
        padding: var(--space-1) var(--space-2);
        font-size: 12px;
        color: var(--accent);
        background: var(--accent-muted);
        border-radius: var(--radius-sm);
    }

    .comments-section {
        margin-top: var(--space-4);
        border-top: 1px solid var(--border);
        padding-top: var(--space-4);
    }

    .comments-toggle {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-3);
        font-size: 14px;
        font-weight: 500;
        color: var(--fg);
        background: var(--bg-secondary);
        border-radius: var(--radius-md);
    }

    .comments-toggle:hover {
        background: var(--bg-tertiary);
    }

    .loading-inline {
        width: 16px;
        height: 16px;
        border: 2px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 1s linear infinite;
    }

    .comments-list {
        margin-top: var(--space-4);
        display: flex;
        flex-direction: column;
        gap: var(--space-3);
    }

    .no-comments {
        text-align: center;
        color: var(--fg-muted);
        padding: var(--space-4);
    }
</style>
