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
import { openExternal } from '$lib/utils/external';

let { html }: { html: string } = $props();

const clean = $derived(DOMPurify.sanitize(html, SANITIZE_CONFIG));

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

<!-- eslint-disable-next-line svelte/no-at-html-tags -- the ONE sanctioned {@html} site: DOMPurify-wrapped, grep-gated (scripts/check-frontend-bans.sh) -->
<div class="sanitized-content" use:interceptLinks>{@html clean}</div>

<style>
    .sanitized-content {
        color: var(--fg);
        overflow-wrap: break-word;
    }

    .sanitized-content :global(p) {
        margin-bottom: var(--space-4);
    }

    .sanitized-content :global(h1),
    .sanitized-content :global(h2),
    .sanitized-content :global(h3),
    .sanitized-content :global(h4) {
        margin: var(--space-5) 0 var(--space-3);
        line-height: 1.3;
    }

    .sanitized-content :global(img) {
        max-width: 100%;
        height: auto;
        border-radius: var(--radius-sm);
    }

    .sanitized-content :global(pre) {
        padding: var(--space-3);
        margin-bottom: var(--space-4);
        overflow-x: auto;
        background: var(--bg-tertiary);
        border-radius: var(--radius-md);
        font-family: var(--font-mono);
        font-size: 0.875em;
    }

    .sanitized-content :global(code) {
        font-family: var(--font-mono);
        font-size: 0.875em;
    }

    .sanitized-content :global(blockquote) {
        margin: 0 0 var(--space-4);
        padding-left: var(--space-4);
        border-left: 3px solid var(--border);
        color: var(--fg-muted);
    }

    .sanitized-content :global(ul),
    .sanitized-content :global(ol) {
        margin-bottom: var(--space-4);
        padding-left: var(--space-5);
    }

    .sanitized-content :global(a) {
        color: var(--link);
        text-decoration: underline;
    }
</style>
