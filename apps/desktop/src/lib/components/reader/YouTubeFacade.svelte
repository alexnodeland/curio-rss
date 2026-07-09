<script lang="ts">
/**
 * The YouTube facade: a lightweight click-to-load video card that only mounts
 * the `youtube-nocookie.com` iframe on an explicit click. Nothing touches the
 * network until the user asks — no cookies, no player JS, no thumbnail fetch
 * (the CSP `img-src` forbids `https:`, and privacy forbids an unmediated
 * request). The poster is a deterministic, self-contained card derived from
 * the video id — a handsome token, not the real still. The nocookie origin is
 * the ONLY host the CSP `frame-src` allows.
 */
import Icon from '$components/common/Icon.svelte';
import VideoPoster from '$components/common/VideoPoster.svelte';
import { tooltip } from '$lib/actions/tooltip';
import { t } from '$lib/i18n';
import { openExternal } from '$lib/utils/external';

let {
    videoId,
    title,
    channel = null,
}: { videoId: string; title: string; channel?: string | null } = $props();

let loaded = $state(false);

// nocookie + no related-video leakage on end; autoplay so the click that
// loaded the frame also starts playback (one gesture, not two). If the
// webview blocks autoplay or rejects the embedding origin, the persistent
// "Open on YouTube" affordance below guarantees the click is never dead.
const embedSrc = $derived(`https://www.youtube-nocookie.com/embed/${videoId}?autoplay=1&rel=0`);
const watchUrl = $derived(`https://www.youtube.com/watch?v=${videoId}`);

function load(): void {
    loaded = true;
}

function openOnYouTube(event: MouseEvent): void {
    event.stopPropagation();
    void openExternal(watchUrl);
}
</script>

<div class="facade">
    {#if loaded}
        <iframe
            class="player"
            src={embedSrc}
            {title}
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; fullscreen; gyroscope; picture-in-picture"
            referrerpolicy="strict-origin-when-cross-origin"
            allowfullscreen
        ></iframe>
    {:else}
        <button
            class="poster"
            type="button"
            onclick={load}
            aria-label={t('reader.youtube.play', { title })}
        >
            <VideoPoster {videoId} seed={videoId} />
            <span class="scrim" aria-hidden="true"></span>
            <span class="poster-badge" aria-hidden="true">
                <Icon name="play" size={12} />
                <span>{t('reader.youtube.badge')}</span>
            </span>
            <span class="play" aria-hidden="true">
                <svg width="30" height="30" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <path d="M8 5v14l11-7z" />
                </svg>
            </span>
            <span class="poster-scrim" aria-hidden="true">
                <span class="poster-title truncate-2">{title}</span>
                <span class="poster-sub">
                    {#if channel !== null}<span class="poster-channel">{channel}</span>{/if}
                    <span class="poster-hint">{t('reader.youtube.hint')}</span>
                </span>
            </span>
        </button>
    {/if}
    <button
        class="open-external"
        type="button"
        aria-label={t('reader.youtube.watch')}
        onclick={openOnYouTube}
        use:tooltip={t('reader.youtube.watch')}
    >
        <Icon name="external" size={15} />
    </button>
</div>

<style>
    .facade {
        position: relative;
        width: 100%;
        aspect-ratio: 16 / 9;
        margin-bottom: var(--space-6);
        border-radius: var(--radius-lg);
        border: 1px solid var(--hairline);
        overflow: hidden;
        background: var(--surface-inset);
        box-shadow: var(--shadow-md);
    }

    .player {
        width: 100%;
        height: 100%;
        border: 0;
        display: block;
    }

    /* Always-present escape hatch: open the video on youtube.com in the OS
       browser. Guarantees the facade is never a dead click even if the webview
       blocks the embed (autoplay policy / embedding-origin rejection). */
    .open-external {
        position: absolute;
        top: var(--space-3);
        right: var(--space-3);
        z-index: 2;
        display: inline-grid;
        place-items: center;
        width: 30px;
        height: 30px;
        border-radius: var(--radius-md);
        color: #fff;
        background: rgb(0 0 0 / 55%);
        backdrop-filter: blur(4px);
        box-shadow: inset 0 0 0 1px rgb(255 255 255 / 16%);
        transition:
            background var(--dur-fast) var(--ease),
            transform var(--dur-fast) var(--ease);
    }

    .open-external:hover {
        background: rgb(0 0 0 / 72%);
        transform: scale(1.06);
    }

    .open-external:focus-visible {
        outline: none;
        box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent), transparent 35%);
    }

    .poster {
        position: absolute;
        inset: 0;
        display: block;
        width: 100%;
        height: 100%;
        padding: 0;
        background: transparent;
        color: #fff;
        cursor: pointer;
    }

    /* A scrim over the poster (gradient or real still) for text legibility. */
    .scrim {
        position: absolute;
        inset: 0;
        background: linear-gradient(to top, rgb(0 0 0 / 62%) 0%, transparent 42%, rgb(0 0 0 / 18%) 100%);
    }

    .poster-badge {
        position: absolute;
        top: var(--space-3);
        left: var(--space-3);
        display: inline-flex;
        align-items: center;
        gap: 5px;
        padding: 3px var(--space-2) 3px 6px;
        border-radius: var(--radius-sm);
        background: #ff0000;
        color: #fff;
        font-family: var(--font-family);
        font-size: 0.6875rem;
        font-weight: 700;
        letter-spacing: 0.01em;
    }

    .play {
        position: absolute;
        top: 50%;
        left: 50%;
        transform: translate(-50%, -50%);
        display: grid;
        place-items: center;
        width: 66px;
        height: 66px;
        border-radius: 50%;
        background: #ff0000;
        color: #fff;
        box-shadow:
            0 6px 20px rgb(0 0 0 / 35%),
            inset 0 0 0 1px rgb(255 255 255 / 14%);
        transition:
            background var(--dur-fast) var(--ease),
            transform var(--dur-fast) var(--ease);
    }

    .poster:hover .play {
        background: #ff2323;
        transform: translate(-50%, -50%) scale(1.06);
    }

    .poster:focus-visible {
        outline: none;
    }

    .poster:focus-visible .play {
        box-shadow:
            0 6px 20px rgb(0 0 0 / 35%),
            0 0 0 4px color-mix(in srgb, var(--accent), transparent 35%);
    }

    .poster-scrim {
        position: absolute;
        left: 0;
        right: 0;
        bottom: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        padding: var(--space-5) var(--space-4) var(--space-4);
        text-align: left;
    }

    .poster-title {
        font-family: var(--font-family);
        font-size: var(--text-lg);
        font-weight: 640;
        line-height: 1.25;
        letter-spacing: var(--tracking-snug);
        text-shadow: 0 1px 3px rgb(0 0 0 / 45%);
    }

    .poster-sub {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        font-family: var(--font-family);
        font-size: var(--text-xs);
        color: rgb(255 255 255 / 82%);
    }

    .poster-channel {
        font-weight: 600;
    }

    .poster-channel::after {
        content: '·';
        margin-left: var(--space-2);
        opacity: 0.7;
    }

    .poster-hint {
        color: rgb(255 255 255 / 66%);
    }

    .truncate-2 {
        display: -webkit-box;
        -webkit-line-clamp: 2;
        line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }
</style>
