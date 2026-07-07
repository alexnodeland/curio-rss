<script lang="ts">
/**
 * An article's lead image (`ArticleSummaryDto.image`) rendered as a cover
 * thumbnail through the policed image cache. First-class RSS imagery: the
 * feed's own `media:thumbnail`/`enclosure` or the body's first `<img>`,
 * surfaced in list rows and the home-view cards.
 *
 * It renders **nothing** unless the media-prefetch setting is on, an image
 * URL is present, and the cache resolves it (→ `asset:`) — so a text-only
 * item reserves no space and a failed fetch collapses cleanly rather than
 * leaving a broken glyph or an empty box. The `variant` picks the frame:
 * a fixed rounded square for a list row, a full-width 16:9 for a card.
 */
import { loadCachedImage } from '$lib/utils/images';
import { uiStore } from '$lib/state/ui.svelte';

let {
    image,
    alt = '',
    variant = 'row',
}: {
    image: string | null;
    alt?: string;
    variant?: 'row' | 'card';
} = $props();

let resolved: string | null = $state(null);

$effect(() => {
    resolved = null;
    const url = image;
    if (!uiStore.mediaPrefetch || url === null) {
        return;
    }
    let cancelled = false;
    void loadCachedImage(url).then((result) => {
        if (!cancelled && result.status === 'ok') {
            resolved = result.data;
        }
    });
    return () => {
        cancelled = true;
    };
});
</script>

{#if resolved !== null}
    <span class="thumb thumb--{variant}">
        <img src={resolved} {alt} loading="lazy" />
    </span>
{/if}

<style>
    .thumb {
        display: block;
        overflow: hidden;
        flex: 0 0 auto;
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
    }

    .thumb img {
        display: block;
        width: 100%;
        height: 100%;
        object-fit: cover;
    }

    .thumb--row {
        width: 52px;
        height: 52px;
        border-radius: var(--radius-md);
    }

    .thumb--card {
        width: 100%;
        aspect-ratio: 16 / 9;
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-sm);
    }
</style>
