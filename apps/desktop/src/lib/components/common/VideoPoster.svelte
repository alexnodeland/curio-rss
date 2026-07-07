<script lang="ts">
/**
 * A video poster background: the real still (fetched through the policed
 * image cache → `asset:`) when the remote-media prefetch setting is on and
 * it resolves, otherwise a deterministic id-derived gradient. The still is
 * the YouTube thumbnail when a video id is known, else the feed's own
 * declared image (`fallbackImage`) — so a video card without a parseable id
 * still shows real RSS imagery. Fills its positioned parent; overlays (play
 * button, scrim) are the parent's. Never makes an unmediated `https:`
 * request — the cache is the only path.
 */
import { posterHue, youTubeThumbnailUrl } from '$lib/reader/view-mode';
import { uiStore } from '$lib/state/ui.svelte';
import { loadCachedImage } from '$lib/utils/images';

let {
    videoId,
    seed,
    fallbackImage = null,
}: { videoId: string | null; seed: string; fallbackImage?: string | null } = $props();

const hue = $derived(posterHue(videoId ?? seed));
const stillUrl = $derived(videoId !== null ? youTubeThumbnailUrl(videoId) : fallbackImage);

let thumb: string | null = $state(null);

$effect(() => {
    const url = stillUrl;
    const on = uiStore.mediaPrefetch;
    thumb = null;
    if (!on || url === null) {
        return;
    }
    let cancelled = false;
    void loadCachedImage(url).then((result) => {
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
