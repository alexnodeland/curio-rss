<script lang="ts">
/**
 * The YouTube facade: a lightweight click-to-load poster that only mounts
 * the `youtube-nocookie.com` iframe on an explicit click. Nothing touches
 * the network until the user asks — no cookies, no player JS, no thumbnail
 * fetch (the CSP `img-src` forbids `https:`, and privacy forbids an
 * unmediated request; the poster is a token, not the real still). The
 * nocookie origin is the ONLY host the CSP `frame-src` allows.
 */
import { t } from '$lib/i18n';

let { videoId, title }: { videoId: string; title: string } = $props();

let loaded = $state(false);

// nocookie + no related-video leakage on end; autoplay so the click that
// loaded the frame also starts playback (one gesture, not two).
const embedSrc = $derived(`https://www.youtube-nocookie.com/embed/${videoId}?autoplay=1&rel=0`);

function load(): void {
    loaded = true;
}
</script>

<div class="facade">
    {#if loaded}
        <iframe
            class="player"
            src={embedSrc}
            {title}
            allow="accelerometer; autoplay; clipboard-write; encrypted-media; gyroscope; picture-in-picture"
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
            <span class="play" aria-hidden="true">
                <svg width="26" height="26" viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
                    <path d="M8 5v14l11-7z" />
                </svg>
            </span>
            <span class="poster-meta">
                <span class="poster-title truncate">{title}</span>
                <span class="poster-hint">{t('reader.youtube.hint')}</span>
            </span>
        </button>
    {/if}
</div>

<style>
    .facade {
        position: relative;
        width: 100%;
        aspect-ratio: 16 / 9;
        margin-bottom: var(--space-5);
        border-radius: var(--radius-lg);
        border: 1px solid var(--hairline);
        overflow: hidden;
        background: var(--surface-inset);
    }

    .player {
        width: 100%;
        height: 100%;
        border: 0;
        display: block;
    }

    .poster {
        position: absolute;
        inset: 0;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-3);
        width: 100%;
        height: 100%;
        background: transparent;
        color: var(--fg);
        cursor: pointer;
        transition: background var(--dur-fast) var(--ease);
    }

    .poster:hover {
        background: var(--hover);
    }

    .poster:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    .play {
        display: grid;
        place-items: center;
        width: 64px;
        height: 64px;
        border-radius: 50%;
        background: var(--accent);
        color: var(--accent-fg);
        box-shadow: var(--shadow-md);
        transition:
            background var(--dur-fast) var(--ease),
            transform var(--dur-fast) var(--ease);
    }

    .poster:hover .play {
        background: var(--accent-hover);
        transform: scale(1.05);
    }

    .poster-meta {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-1);
        max-width: min(90%, 480px);
    }

    .poster-title {
        font-size: var(--text-base);
        font-weight: 600;
        max-width: 100%;
    }

    .poster-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>
