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
 * - no author `data-*` attributes: DOMPurify skips its URI checks on them,
 *   and the image pipeline keeps its own state in `data-*` (`origSrc`,
 *   `data-media-off`), set only after sanitizing — an author copy must not
 *   be able to seed it;
 * - link clicks never navigate the webview: same-document `#fragment` links
 *   (footnotes — ingest keeps their `user-content-` ids and hrefs) scroll
 *   within the reader, everything else is delegated to the URL-scoped
 *   opener (http/https only), matching the no-window.open rule.
 *
 * On top of the sanitized body: wide tables scroll inside the column, body
 * images load lazily through the policed cache and collapse when they
 * fail, and a loaded image opens a lightbox (see `utils/reader-content`).
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
    ALLOW_DATA_ATTR: false,
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
import { modalLayer } from '$lib/actions/modal-layer';
import { t } from '$lib/i18n';
import { prefsStore } from '$lib/state/prefs.svelte';
import { openExternal } from '$lib/utils/external';
import {
    isZoomable,
    mountBodyImages,
    scrollToFragment,
    watchTableOverflow,
    wrapTables,
} from '$lib/utils/reader-content';

let { html }: { html: string } = $props();

/**
 * Sanitize, then shape the detached tree before `{@html}` owns it: wide
 * tables get their scroll wrapper here, as markup, because wrapping nodes
 * after render would move them out from under Svelte's node tracking.
 */
function prepare(source: string, tableLabel: string): string {
    // A <template> parses into an inert document: nothing in it loads or
    // runs, and it re-serializes exactly what `{@html}` would parse anyway.
    const template = document.createElement('template');
    template.innerHTML = DOMPurify.sanitize(source, SANITIZE_CONFIG);
    wrapTables(template.content, tableLabel);
    return template.innerHTML;
}

const clean = $derived(prepare(html, t('reader.tableScroll')));

let contentEl: HTMLElement | undefined = $state();

/** How many remote body images this article is currently suppressing. */
let offImageCount = $state(0);

/** Show the "images are off" nudge only when this article actually hides some. */
const showMediaHint = $derived(
    !prefsStore.mediaPrefetch && offImageCount > 0 && !prefsStore.mediaHintDismissed,
);

/** The image the lightbox is showing, or null when it is closed. */
let zoomed: { src: string; alt: string } | null = $state(null);

/**
 * Body images resolve lazily through the policed cache (or stay hidden with
 * media prefetch off) — see `mountBodyImages`. Re-runs when the sanitized
 * body or the prefetch setting changes.
 */
$effect(() => {
    void clean; // re-run when the sanitized body changes
    const on = prefsStore.mediaPrefetch;
    const root = contentEl;
    if (root === undefined) {
        return;
    }
    const mounted = mountBodyImages(root, on, t('reader.openImage'));
    offImageCount = mounted.offCount;
    return () => {
        mounted.dispose();
    };
});

// A table wrapper is a focusable, named scroll region only while its table
// overflows the column — see `watchTableOverflow`.
$effect(() => {
    void clean;
    const root = contentEl;
    if (root === undefined) {
        return;
    }
    return watchTableOverflow(root);
});

// A new article closes any lightbox left open on the previous one.
$effect(() => {
    void clean;
    zoomed = null;
});

function openLightbox(img: HTMLImageElement): void {
    zoomed = { src: img.getAttribute('src') ?? '', alt: img.getAttribute('alt') ?? '' };
}

function closeLightbox(): void {
    zoomed = null;
}

/** `auxclick` fires for middle/right; only a middle-click (button 1) counts. */
function isPrimaryOrMiddle(event: MouseEvent): boolean {
    // Middle-click must be intercepted too — otherwise it bypasses the
    // opener and the webview navigates. Right-click is left for the menu.
    return event.type !== 'auxclick' || event.button === 1;
}

/**
 * Body clicks never navigate the webview: a same-document `#fragment`
 * (footnotes, in-page contents) scrolls within the reader, every other
 * link goes to the URL-scoped OS opener, and a loaded image outside a link
 * opens the lightbox.
 */
function onContentClick(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof Element) || !isPrimaryOrMiddle(event)) {
        return;
    }
    const anchor = target.closest('a[href]');
    if (anchor === null) {
        if (event.type === 'click' && isZoomable(target)) {
            openLightbox(target);
        }
        return;
    }
    event.preventDefault();
    const href = anchor.getAttribute('href') ?? '';
    if (href.startsWith('#')) {
        if (event.type === 'click' && contentEl !== undefined) {
            scrollToFragment(contentEl, href);
        }
        return;
    }
    void openExternal(href);
}

/** Enter / Space on a focused body image opens the lightbox. */
function onContentKeydown(event: KeyboardEvent): void {
    const target = event.target;
    if ((event.key === 'Enter' || event.key === ' ') && target instanceof Element && isZoomable(target)) {
        event.preventDefault();
        event.stopPropagation();
        openLightbox(target);
    }
}

function interceptContent(node: HTMLElement): { destroy(): void } {
    node.addEventListener('click', onContentClick);
    node.addEventListener('auxclick', onContentClick);
    node.addEventListener('keydown', onContentKeydown);
    return {
        destroy(): void {
            node.removeEventListener('click', onContentClick);
            node.removeEventListener('auxclick', onContentClick);
            node.removeEventListener('keydown', onContentKeydown);
        },
    };
}

/**
 * The lightbox owns the keyboard while open: Escape closes it, and no key
 * leaks to the app shortcuts underneath (j/k would change the article
 * behind it). A direct listener — not a delegated `onkeydown` — so the
 * stop happens before the window-level shortcut handler sees the event.
 */
function lightboxKeys(node: HTMLElement): { destroy(): void } {
    function onKeydown(event: KeyboardEvent): void {
        event.stopPropagation();
        if (event.key === 'Escape') {
            event.preventDefault();
            closeLightbox();
        }
    }
    node.addEventListener('keydown', onKeydown);
    return {
        destroy(): void {
            node.removeEventListener('keydown', onKeydown);
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
            onclick={() => void prefsStore.setMediaPrefetch(true)}
        >
            {t('reader.imagesOff.enable')}
        </button>
        <button
            class="media-hint-dismiss"
            type="button"
            aria-label={t('toast.dismiss')}
            onclick={() => prefsStore.dismissMediaHint()}
        >
            <Icon name="close" size={14} />
        </button>
    </div>
{/if}

<!-- eslint-disable-next-line svelte/no-at-html-tags -- the ONE sanctioned {@html} site: DOMPurify-wrapped, grep-gated (scripts/check-frontend-bans.sh) -->
<div class="sanitized-content" data-testid="reader-content" bind:this={contentEl} use:interceptContent>{@html clean}</div>

{#if zoomed !== null}
    <!-- The backdrop click is a pointer convenience; Escape and the close
         button are the keyboard paths. `modalLayer` traps focus in the
         dialog and makes the app frame behind it inert (aria-modal's
         promise). -->
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div class="lightbox" data-testid="reader-lightbox" onclick={closeLightbox}>
        <div
            class="lightbox-dialog"
            role="dialog"
            aria-modal="true"
            aria-label={zoomed.alt.length > 0 ? zoomed.alt : t('reader.imageViewer')}
            tabindex="-1"
            use:modalLayer
            use:lightboxKeys
        >
            <img class="lightbox-image" src={zoomed.src} alt={zoomed.alt} />
            <button class="lightbox-close" type="button" aria-label={t('modal.close')} onclick={closeLightbox}>
                <Icon name="close" size={16} />
            </button>
        </div>
    </div>
{/if}

<style>
    /* Editorial reading rhythm. The article sets the font-family / size /
       line-height on .reader-article (from the typography controls); this
       governs vertical rhythm, emphasis, and the block elements. */
    .sanitized-content {
        /* The body's own tints, mixed from the reading ink over a
           transparent ground. The reading themes (sepia, paper) repaint
           --ink but not the :root surface tokens, which follow the *app*
           theme — so a fill built from --surface-* would be a dark box
           under sepia ink on a dark app theme. These follow whichever
           ground the reader shows. */
        --reader-tint: color-mix(in srgb, transparent, var(--ink) 5%);
        --reader-rule: color-mix(in srgb, transparent, var(--ink) 14%);

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
        /* --para-spacing (a multiple, set on .reader-article by the Reading
           controls) scales the inter-paragraph gap; 1 = the default 1.1em. */
        margin: 0 0 calc(var(--para-spacing, 1) * 1.1em);
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

    /* A loaded image outside a link opens the lightbox. */
    .sanitized-content :global(img[src][tabindex]) {
        cursor: zoom-in;
    }

    /* Waiting for the lazy loader: a quiet placeholder in the reserved box
       (the aspect ratio comes from the image's width/height), with no
       shadow and no alt text bleeding through an empty <img>. Inline-block
       keeps an inline image (an icon in a sentence) in its line, so
       nothing reflows when it loads. */
    .sanitized-content :global(img[data-media-pending]) {
        display: inline-block;
        background: var(--reader-tint);
        box-shadow: none;
        color: transparent;
    }

    /* Remote images with prefetch off never load, and failed ones never
       will — collapse them cleanly (no broken glyph, no empty shadowed
       box) rather than leave a gap. */
    .sanitized-content :global(img[data-media-off]),
    .sanitized-content :global(img[data-media-failed]) {
        display: none;
    }

    /* A figure whose only child is a hidden image would otherwise leave its
       margin behind — fold the wrapper. */
    .sanitized-content :global(figure:has(> img[data-media-off]:only-child)) {
        display: none;
    }

    /* A failed image takes its whole figure with it: a caption describing
       an image that will never appear is an artefact, not content. Only
       when no other image in the figure is still showing, and only for a
       figure of images and a caption — one that also holds code, a quote
       or prose keeps that content. */
    .sanitized-content
        :global(
            figure:has(img[data-media-failed]):not(:has(img:not([data-media-failed]))):not(
                    :has(> :not(img, picture, a, figcaption))
                )
        ) {
        display: none;
    }

    /* Wide tables scroll inside the column instead of blowing it out. The
       box hugs a narrow table rather than stretching it to the column, and
       stays transparent — hairlines and a header tint from the reading ink,
       so it reads on every reading theme. */
    .sanitized-content :global(.table-scroll) {
        width: fit-content;
        max-width: 100%;
        margin: 1.6em 0;
        overflow-x: auto;
        border: 1px solid var(--reader-rule);
        border-radius: var(--radius-lg);
    }

    .sanitized-content :global(table) {
        border-collapse: collapse;
        font-family: var(--font-family);
        font-size: 0.84em;
        line-height: 1.45;
        font-variant-numeric: lining-nums tabular-nums;
    }

    /* A table nested in a cell (newsletter layout) keeps its outer
       table's size instead of shrinking again at every level. */
    .sanitized-content :global(table table) {
        font-size: 1em;
    }

    .sanitized-content :global(th),
    .sanitized-content :global(td) {
        padding: 0.55em 0.85em;
        border-bottom: 1px solid var(--reader-rule);
        text-align: start;
        vertical-align: top;
    }

    .sanitized-content :global(th) {
        background: var(--reader-tint);
        color: var(--fg);
        font-weight: 600;
        white-space: nowrap;
    }

    .sanitized-content :global(tr:last-child > td) {
        border-bottom: none;
    }

    .sanitized-content :global(caption) {
        caption-side: bottom;
        padding: 0.6em 0.85em;
        font-size: 0.92em;
        color: var(--fg-muted);
        text-align: start;
    }

    /* Footnote jump target: scroll lands with breathing room, and the
       target glows briefly so the eye finds it. */
    .sanitized-content :global([id]) {
        scroll-margin-top: var(--space-6);
    }

    .sanitized-content :global([data-fragment-target]) {
        border-radius: var(--radius-sm);
        animation: fragment-flash 1.6s var(--ease);
    }

    .sanitized-content :global([data-fragment-target]:focus) {
        outline: none;
    }

    @keyframes fragment-flash {
        from {
            background: var(--accent-muted);
        }
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
        /* Long paths and identifiers wrap instead of overflowing the
           column; each wrapped line keeps its own chip edges. */
        overflow-wrap: anywhere;
        -webkit-box-decoration-break: clone;
        box-decoration-break: clone;
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

    /* Click-to-zoom: the image at up to the full window, over a dimmed,
       blurred backdrop (the same scrim the modals use). Any click closes. */
    .lightbox {
        position: fixed;
        inset: 0;
        z-index: 100;
        display: flex;
        align-items: center;
        justify-content: center;
        padding: var(--space-8);
        background: color-mix(in srgb, #000, transparent 25%);
        backdrop-filter: blur(3px);
        cursor: zoom-out;
        animation: lightbox-in var(--dur-base) var(--ease);
    }

    .lightbox-dialog {
        position: relative;
        display: flex;
        max-width: 100%;
        max-height: 100%;
    }

    .lightbox-dialog:focus-visible {
        outline: none;
    }

    .lightbox-image {
        max-width: 100%;
        max-height: calc(100vh - 2 * var(--space-8));
        object-fit: contain;
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-lg);
    }

    .lightbox-close {
        position: absolute;
        top: var(--space-2);
        right: var(--space-2);
        display: inline-grid;
        place-items: center;
        width: 32px;
        height: 32px;
        color: #fff;
        background: color-mix(in srgb, #000, transparent 45%);
        border: none;
        border-radius: var(--radius-pill);
        cursor: pointer;
        transition: background var(--dur-fast) var(--ease);
    }

    .lightbox-close:hover {
        background: color-mix(in srgb, #000, transparent 20%);
    }

    @keyframes lightbox-in {
        from {
            opacity: 0;
        }
    }
</style>
