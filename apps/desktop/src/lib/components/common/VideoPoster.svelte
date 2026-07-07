<script lang="ts">
/**
 * A video poster background: the real YouTube still (fetched through the
 * policed image cache → `asset:`) when the remote-media prefetch setting is
 * on and it resolves, otherwise a deterministic id-derived gradient. Fills
 * its positioned parent; overlays (play button, scrim) are the parent's.
 * Never makes an unmediated `https:` request — the cache is the only path.
 */
import { posterHue, youTubeThumbnailUrl } from '$lib/reader/view-mode';
import { uiStore } from '$lib/state/ui.svelte';
import { loadCachedImage } from '$lib/utils/images';

let { videoId, seed }: { videoId: string | null; seed: string } = $props();

const hue = $derived(posterHue(videoId ?? seed));

let thumb: string | null = $state(null);

$effect(() => {
    const id = videoId;
    const on = uiStore.mediaPrefetch;
    thumb = null;
    if (!on || id === null) {
        return;
    }
    let cancelled = false;
    void loadCachedImage(youTubeThumbnailUrl(id)).then((result) => {
        if (!cancelled && result.status === 'ok') {
            thumb = result.data;
        }
    });
    return () => {
        cancelled = true;
    };
});
</script>

{#if thumb !== null}
    <img class="poster-img" src={thumb} alt="" aria-hidden="true" />
{:else}
    <span class="poster-art" style:--poster-hue={hue} aria-hidden="true"></span>
{/if}

<style>
    .poster-img {
        position: absolute;
        inset: 0;
        width: 100%;
        height: 100%;
        object-fit: cover;
        display: block;
    }

    .poster-art {
        position: absolute;
        inset: 0;
        background:
            radial-gradient(120% 120% at 30% 20%, hsl(var(--poster-hue) 55% 42%), transparent 60%),
            linear-gradient(150deg, hsl(var(--poster-hue) 45% 24%), hsl(calc(var(--poster-hue) + 30) 40% 15%));
    }
</style>
