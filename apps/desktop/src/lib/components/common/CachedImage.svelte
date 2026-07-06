<script lang="ts">
/**
 * A remote image rendered through the Rust image cache: the URL is fetched
 * (policed, content-hashed) and served via the `asset:` protocol, so the
 * webview never makes an unmediated `https:` request. Renders nothing until
 * (and unless) the image resolves — a missing icon is silent, never a
 * broken-image glyph. `onfail` lets a parent try a fallback source.
 */
import { loadCachedImage } from '$lib/utils/images';

let {
    src,
    alt,
    size = 16,
    onfail,
}: {
    src: string;
    alt: string;
    size?: number;
    onfail?: () => void;
} = $props();

let resolved: string | null = $state(null);
let failed = $state(false);

$effect(() => {
    const url = src;
    resolved = null;
    failed = false;
    let cancelled = false;
    void loadCachedImage(url).then((result) => {
        if (cancelled) {
            return;
        }
        if (result.status === 'ok') {
            resolved = result.data;
        } else {
            failed = true;
            onfail?.();
        }
    });
    return () => {
        cancelled = true;
    };
});

function onImageError(): void {
    failed = true;
    onfail?.();
}
</script>

{#if resolved !== null && !failed}
    <img
        class="cached-image"
        src={resolved}
        {alt}
        width={size}
        height={size}
        onerror={onImageError}
    />
{/if}

<style>
    .cached-image {
        object-fit: contain;
        border-radius: 3px;
        flex: 0 0 auto;
    }
</style>
