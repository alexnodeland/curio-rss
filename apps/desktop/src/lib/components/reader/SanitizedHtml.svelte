<script lang="ts" module>
/**
 * The single sanitized-render component — the ONLY file in the tree allowed
 * to contain `{@html}` (CI grep + lefthook enforce it). DOMPurify here is
 * belt-and-braces over core's ammonia ingest sanitization (raw HTML never
 * reaches the DB); this layer is defense-in-depth, not the primary defense.
 *
 * Posture on top of DOMPurify's defaults:
 * - HTML profile only (no SVG/MathML — kills the svg/onload class outright);
 * - no forms/inputs (phishing surface), no style tags/attributes (the
 *   reader's typography is ours), no frames/objects/base/meta;
 * - `data:` URIs stripped everywhere except `data:image/*` (which the CSP
 *   `img-src` deliberately allows);
 * - link clicks never navigate the webview: delegated to the URL-scoped
 *   opener (http/https only), matching the no-window.open rule.
 */
import DOMPurify, { type Config } from 'dompurify';

const SANITIZE_CONFIG: Config = {
    USE_PROFILES: { html: true },
    FORBID_TAGS: [
        'base',
        'button',
        'dialog',
        'embed',
        'form',
        'iframe',
        'input',
        'meta',
        'object',
        'select',
        'style',
        'textarea',
    ],
    FORBID_ATTR: ['style'],
    ALLOW_UNKNOWN_PROTOCOLS: false,
};

// Non-image data: URIs have no business in feed content; DOMPurify's
// defaults allow data: on media tags, so tighten to data:image/* only.
DOMPurify.addHook('afterSanitizeAttributes', (node) => {
    if (node instanceof Element && node.hasAttribute('src')) {
        const src = node.getAttribute('src') ?? '';
        if (/^\s*data:(?!image\/)/i.test(src)) {
            node.removeAttribute('src');
        }
    }
});
</script>

<script lang="ts">
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import { openExternal } from '$lib/utils/external';
import { uiStore } from '$lib/state/ui.svelte';
import { loadCachedImage } from '$lib/utils/images';

let { html }: { html: string } = $props();

const clean = $derived(DOMPurify.sanitize(html, SANITIZE_CONFIG));

let contentEl: HTMLElement | undefined = $state();

/** How many remote body images this article is currently suppressing. */
let offImageCount = $state(0);

/** Show the "images are off" nudge only when this article actually hides some. */
const showMediaHint = $derived(
    !uiStore.mediaPrefetch && offImageCount > 0 && !uiStore.mediaHintDismissed,
);

/**
 * Remote body images (`http(s)`) never load unmediated — the CSP `img-src`
 * forbids `https:`. When the media-prefetch setting is on we resolve each
 * through the policed cache (→ `asset:`) and swap the src; when off, the src
 * is cleared so nothing hits the network. `data:image/*` (inline) is left
 * alone — it needs no fetch and the CSP already allows it.
 */
$effect(() => {
    void clean; // re-run when the sanitized body changes
    const on = uiStore.mediaPrefetch;
    const root = contentEl;
    if (root === undefined) {
        return;
    }
    let cancelled = false;
    let offCount = 0;
    for (const img of root.querySelectorAll('img')) {
        const original = img.dataset.origSrc ?? img.getAttribute('src') ?? '';
        img.dataset.origSrc = original;
        if (!/^https?:\/\//i.test(original)) {
            continue; // data: / already-local — leave as-is
        }
        img.removeAttribute('src');
        img.toggleAttribute('data-media-off', !on);
        if (on) {
            void loadCachedImage(original).then((result) => {
                if (!cancelled && result.status === 'ok') {
                    img.setAttribute('src', result.data);
                    img.removeAttribute('data-media-off');
                }
            });
        } else {
            offCount += 1;
        }
    }
    offImageCount = offCount;
    return () => {
        cancelled = true;
    };
});

/** Delegates anchor clicks to the OS browser; the webview never navigates. */
function interceptLinks(node: HTMLElement): { destroy(): void } {
    function onClick(event: MouseEvent): void {
        const target = event.target;
        if (!(target instanceof Element)) {
            return;
        }
        const anchor = target.closest('a[href]');
        if (anchor === null) {
            return;
        }
        event.preventDefault();
        const href = anchor.getAttribute('href');
        if (href !== null) {
            void openExternal(href);
        }
    }
    node.addEventListener('click', onClick);
    return {
        destroy(): void {
            node.removeEventListener('click', onClick);
        },
    };
}
</script>

{#if showMediaHint}
    <div class="media-hint" role="note">
        <span class="media-hint-text">{t('reader.imagesOff')}</span>
        <button
            class="media-hint-enable"
            type="button"
            onclick={() => void uiStore.setMediaPrefetch(true)}
        >
            {t('reader.imagesOff.enable')}
        </button>
        <button
            class="media-hint-dismiss"
            type="button"
            aria-label={t('toast.dismiss')}
            onclick={() => uiStore.dismissMediaHint()}
        >
            <Icon name="close" size={14} />
        </button>
    </div>
{/if}

<!-- eslint-disable-next-line svelte/no-at-html-tags -- the ONE sanctioned {@html} site: DOMPurify-wrapped, grep-gated (scripts/check-frontend-bans.sh) -->
<div class="sanitized-content" bind:this={contentEl} use:interceptLinks>{@html clean}</div>

<style>
    /* Editorial reading rhythm. The article sets the font-family / size /
       line-height on .reader-article (from the typography controls); this
       governs vertical rhythm, emphasis, and the block elements. */
    .sanitized-content {
        color: var(--ink);
        overflow-wrap: break-word;
        font-kerning: normal;
        font-variant-numeric: oldstyle-nums proportional-nums;
    }

    /* The one-time nudge when an article hides remote images. */
    .media-hint {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        margin-bottom: var(--space-4);
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        border: 1px solid var(--hairline);
        background: var(--surface-raised);
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .media-hint-text {
        flex: 1 1 auto;
        min-width: 0;
    }

    .media-hint-enable {
        flex: 0 0 auto;
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--accent);
        border: none;
        font-size: var(--text-sm);
        font-weight: 500;
        cursor: pointer;
    }

    .media-hint-enable:hover {
        text-decoration: underline;
    }

    .media-hint-dismiss {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 1.25rem;
        height: 1.25rem;
        color: var(--fg-subtle);
        background: transparent;
        border: none;
        border-radius: var(--radius-sm);
        cursor: pointer;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .media-hint-dismiss:hover {
        color: var(--fg);
        background: var(--hover);
    }

    .sanitized-content :global(p) {
        margin: 0 0 1.1em;
        hanging-punctuation: first;
    }

    /* Lede: the first paragraph carries the eye in. */
    .sanitized-content :global(> p:first-of-type) {
        font-size: 1.08em;
        line-height: 1.55;
        color: var(--fg);
    }

    .sanitized-content :global(h1),
    .sanitized-content :global(h2),
    .sanitized-content :global(h3),
    .sanitized-content :global(h4) {
        font-family: var(--font-family);
        color: var(--fg);
        line-height: 1.25;
        letter-spacing: var(--tracking-snug);
        font-weight: 640;
        margin: 1.9em 0 0.6em;
    }

    .sanitized-content :global(h2) {
        font-size: 1.32em;
    }
    .sanitized-content :global(h3) {
        font-size: 1.12em;
    }
    .sanitized-content :global(h4) {
        font-size: 1em;
        color: var(--fg-muted);
        text-transform: uppercase;
        letter-spacing: var(--tracking-caps);
    }

    .sanitized-content :global(figure) {
        margin: 1.8em 0;
    }

    .sanitized-content :global(img) {
        max-width: 100%;
        height: auto;
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-md);
    }

    /* Remote images with prefetch off never load — collapse them cleanly
       (no broken glyph) rather than leave a gap. */
    .sanitized-content :global(img[data-media-off]) {
        display: none;
    }

    /* A figure whose only child is a collapsed remote image would otherwise
       leave its margin (and an orphaned caption) behind — fold the wrapper. */
    .sanitized-content :global(figure:has(> img[data-media-off]:only-child)) {
        display: none;
    }

    .sanitized-content :global(figcaption) {
        margin-top: 0.7em;
        font-family: var(--font-family);
        font-size: 0.8em;
        line-height: 1.45;
        color: var(--fg-muted);
        text-align: center;
    }

    .sanitized-content :global(pre) {
        padding: var(--space-4);
        margin: 1.6em 0;
        overflow-x: auto;
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
        font-family: var(--font-mono);
        font-size: 0.82em;
        line-height: 1.6;
        -webkit-font-smoothing: auto;
    }

    .sanitized-content :global(code) {
        font-family: var(--font-mono);
        font-size: 0.86em;
    }

    /* Inline code gets a subtle chip; code inside <pre> does not. */
    .sanitized-content :global(:not(pre) > code) {
        padding: 0.12em 0.38em;
        border-radius: var(--radius-sm);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        font-size: 0.82em;
        white-space: nowrap;
    }

    .sanitized-content :global(blockquote) {
        margin: 1.6em 0;
        padding: 0.1em 0 0.1em 1.15em;
        border-left: 3px solid var(--accent);
        color: var(--fg);
        font-style: italic;
        font-size: 1.06em;
        line-height: 1.5;
    }

    .sanitized-content :global(blockquote p:last-child) {
        margin-bottom: 0;
    }

    .sanitized-content :global(ul),
    .sanitized-content :global(ol) {
        margin: 0 0 1.1em;
        padding-left: 1.4em;
    }

    .sanitized-content :global(li) {
        margin-bottom: 0.4em;
    }

    .sanitized-content :global(li::marker) {
        color: var(--accent);
    }

    .sanitized-content :global(hr) {
        margin: 2.2em auto;
        width: 40%;
        border: none;
        border-top: 1px solid var(--hairline-strong);
    }

    .sanitized-content :global(a) {
        color: var(--link);
        text-decoration: underline;
        text-decoration-color: color-mix(in srgb, var(--link), transparent 55%);
        text-underline-offset: 0.15em;
        text-decoration-thickness: 1px;
        transition: text-decoration-color var(--dur-fast) var(--ease);
    }

    .sanitized-content :global(a:hover) {
        text-decoration-color: var(--link);
    }

    .sanitized-content :global(strong) {
        font-weight: 680;
        color: var(--fg);
    }
</style>
