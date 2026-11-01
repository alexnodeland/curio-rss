/**
 * DOM helpers for the sanitized article body (`SanitizedHtml.svelte`): the
 * pieces of reader behaviour that act on already-sanitized markup rather
 * than on the string — wide-table containment, the lazy body-image
 * pipeline, and in-page fragment (footnote) scrolling. Kept out of the
 * component so each rule is unit-testable on its own.
 *
 * Nothing here parses or trusts feed markup: every function runs on the
 * DOMPurify output (which drops author `data-*`), and the only attributes
 * written are the reader's own (`data-media-*`, `data-scroll-label`, and the
 * `tabindex`/`role`/`aria-*` that make a scrolling table or a zoomable image
 * reachable).
 */
import { loadCachedImage } from '$lib/utils/images';

/** Class of the horizontal-scroll wrapper every body `<table>` sits in. */
export const TABLE_SCROLL_CLASS = 'table-scroll';

/**
 * How far ahead of the visible reading area (in CSS pixels, vertically) a
 * body image starts resolving — roughly a screen of reading, so images are
 * usually ready by the time they scroll in without fetching the whole
 * article up front.
 */
export const LAZY_IMAGE_MARGIN = '800px 0px';

/** Where a table wrapper keeps the name it takes on while it overflows. */
const SCROLL_LABEL_ATTR = 'data-scroll-label';

/**
 * Wraps every top-level `<table>` under `root` in a horizontal-scroll
 * container, so a wide table scrolls inside the reading column instead of
 * blowing it out. Runs on the sanitized *detached* tree before it is
 * rendered (the wrapper must exist before `{@html}` takes ownership of the
 * nodes). A table nested in another (newsletter layout tables) rides its
 * outer table's wrapper instead of boxing itself again. `label` is the
 * scroll region's name for assistive tech (a caption names it instead);
 * it is applied, with a Tab stop, only while the table actually overflows
 * — see {@link watchTableOverflow}. Idempotent: a table already in a
 * wrapper is left alone.
 */
export function wrapTables(root: ParentNode, label: string): void {
    for (const table of root.querySelectorAll('table')) {
        // (A table at the top of a fragment has no parent element.)
        const parent = table.parentElement;
        if (
            parent !== null &&
            (parent.classList.contains(TABLE_SCROLL_CLASS) || parent.closest('table') !== null)
        ) {
            continue;
        }
        const wrapper = table.ownerDocument.createElement('div');
        wrapper.className = TABLE_SCROLL_CLASS;
        const caption = table.caption?.textContent?.trim() ?? '';
        wrapper.setAttribute(SCROLL_LABEL_ATTR, caption.length > 0 ? caption : label);
        table.replaceWith(wrapper);
        wrapper.append(table);
    }
}

/**
 * Makes a table wrapper a named, focusable scroll region exactly while its
 * table is wider than the column (keyboard users scroll a region by
 * focusing it); a table that fits is plain content with no extra Tab stop.
 */
export function syncTableOverflow(wrapper: Element): void {
    if (wrapper.scrollWidth > wrapper.clientWidth) {
        wrapper.setAttribute('role', 'region');
        wrapper.setAttribute('tabindex', '0');
        wrapper.setAttribute('aria-label', wrapper.getAttribute(SCROLL_LABEL_ATTR) ?? '');
    } else {
        wrapper.removeAttribute('role');
        wrapper.removeAttribute('tabindex');
        wrapper.removeAttribute('aria-label');
    }
}

/**
 * Keeps every table wrapper under `root` in sync with whether it overflows
 * (see {@link syncTableOverflow}) as the column resizes — window width,
 * reading width, font size. Checks once when the webview has no
 * `ResizeObserver`. Returns the disconnect.
 */
export function watchTableOverflow(root: ParentNode): () => void {
    const wrappers = Array.from(root.querySelectorAll(`.${TABLE_SCROLL_CLASS}`));
    for (const wrapper of wrappers) {
        syncTableOverflow(wrapper);
    }
    if (typeof ResizeObserver === 'undefined' || wrappers.length === 0) {
        return () => undefined;
    }
    const observer = new ResizeObserver((entries) => {
        for (const entry of entries) {
            const wrapper = entry.target.closest(`.${TABLE_SCROLL_CLASS}`);
            if (wrapper !== null) {
                syncTableOverflow(wrapper);
            }
        }
    });
    for (const wrapper of wrappers) {
        observer.observe(wrapper);
        // The wrapper keeps its width when only the table grows (web fonts
        // settling, images inside cells), so watch the table too.
        const table = wrapper.firstElementChild;
        if (table !== null) {
            observer.observe(table);
        }
    }
    return () => observer.disconnect();
}

/** The nearest scrolling ancestor of `node` — the lazy-load observer root. */
export function scrollParent(node: Element): Element | null {
    for (let current = node.parentElement; current !== null; current = current.parentElement) {
        const { overflowY } = getComputedStyle(current);
        if (overflowY === 'auto' || overflowY === 'scroll' || overflowY === 'overlay') {
            return current;
        }
    }
    return null;
}

/**
 * Reserves an image's box from its `width`/`height` attributes (kept at
 * ingest as plain pixel integers) so text doesn't jump when it loads. Set
 * through the CSSOM — not a `style` attribute — so it is the reader's own
 * declaration, never feed markup.
 */
export function reserveAspectRatio(img: HTMLImageElement): void {
    const width = Number.parseInt(img.getAttribute('width') ?? '', 10);
    const height = Number.parseInt(img.getAttribute('height') ?? '', 10);
    if (width > 0 && height > 0) {
        // `auto` holds the box while nothing has loaded, then yields to the
        // file's natural ratio, so declared dimensions that disagree with
        // the real image never stretch it (what the UA's own width/height
        // mapping does).
        img.style.aspectRatio = `auto ${width} / ${height}`;
    }
}

/** The attributes that make a loaded body image a lightbox trigger. */
const ZOOM_ATTRS = ['tabindex', 'role', 'aria-haspopup', 'aria-label', 'aria-description'];

/** Removes what {@link makeZoomable} added. */
function clearZoomable(img: HTMLImageElement): void {
    for (const name of ZOOM_ATTRS) {
        img.removeAttribute(name);
    }
}

/**
 * Exposes a loaded image (outside a link) as what it is to keyboard and
 * screen-reader users: a button that opens a dialog. The alt text stays
 * its name and `hint` says what activating it does; an image without alt
 * text is named by `hint` alone.
 */
function makeZoomable(img: HTMLImageElement, hint: string): void {
    img.setAttribute('tabindex', '0');
    img.setAttribute('role', 'button');
    img.setAttribute('aria-haspopup', 'dialog');
    if ((img.getAttribute('alt') ?? '').trim().length > 0) {
        img.setAttribute('aria-description', hint);
    } else {
        img.setAttribute('aria-label', hint);
    }
}

/** Collapses an image that could not be fetched or decoded (see the CSS). */
export function markImageFailed(img: HTMLImageElement): void {
    img.removeAttribute('data-media-pending');
    img.removeAttribute('src');
    clearZoomable(img);
    img.setAttribute('data-media-failed', '');
}

/** Whether `img` is a loaded body image the lightbox can open. */
export function isZoomable(img: Element): img is HTMLImageElement {
    return (
        img instanceof HTMLImageElement &&
        img.hasAttribute('src') &&
        !img.hasAttribute('data-media-pending') &&
        !img.hasAttribute('data-media-failed') &&
        !img.hasAttribute('data-media-off') &&
        img.closest('a[href]') === null
    );
}

/** What {@link mountBodyImages} reports back to the component. */
export interface MountedImages {
    /** Remote images suppressed because media prefetch is off. */
    offCount: number;
    /** Stops observing and drops every in-flight result. */
    dispose(): void;
}

function showLoaded(img: HTMLImageElement, src: string, zoomHint: string): void {
    img.setAttribute('src', src);
    img.removeAttribute('data-media-pending');
    // A loaded image (outside a link) opens the lightbox — reachable by
    // keyboard too.
    if (img.closest('a[href]') === null) {
        makeZoomable(img, zoomHint);
    }
}

/** Resets one image to its routed-but-not-loaded state; returns its remote URL, if any. */
function resetImage(img: HTMLImageElement): string | null {
    img.removeAttribute('srcset');
    img.removeAttribute('sizes');
    img.removeAttribute('data-media-failed');
    img.removeAttribute('data-media-pending');
    clearZoomable(img);
    reserveAspectRatio(img);
    const original = img.dataset.origSrc ?? img.getAttribute('src') ?? '';
    img.dataset.origSrc = original;
    if (!/^https?:\/\//i.test(original)) {
        return null; // data: / already-local — no fetch needed
    }
    img.removeAttribute('src');
    return original;
}

/**
 * Resets every body image under `root` for routing; returns the remote
 * images waiting to load (empty with prefetch off) and how many were hidden.
 */
function routeImages(
    root: HTMLElement,
    prefetch: boolean,
    zoomHint: string,
): { pending: Map<Element, string>; offCount: number } {
    // Strip srcset on any <picture><source>: responsive candidates bypass
    // the src-only media pipeline entirely — with media on the CSP blocks
    // them (broken image), with media off they aren't hidden.
    for (const source of root.querySelectorAll('source[srcset]')) {
        source.removeAttribute('srcset');
    }
    const pending = new Map<Element, string>();
    let offCount = 0;
    for (const img of root.querySelectorAll('img')) {
        const remote = resetImage(img);
        if (remote === null) {
            const local = img.getAttribute('src');
            if (local !== null) {
                showLoaded(img, local, zoomHint);
            }
            continue;
        }
        img.toggleAttribute('data-media-off', !prefetch);
        if (prefetch) {
            img.setAttribute('data-media-pending', '');
            pending.set(img, remote);
        } else {
            offCount += 1;
        }
    }
    return { pending, offCount };
}

/**
 * Calls `load` for each pending image once it comes within
 * {@link LAZY_IMAGE_MARGIN} of `root`'s scroll viewport (immediately, when
 * the webview has no `IntersectionObserver`). Returns the disconnect.
 */
function loadNearViewport(
    root: HTMLElement,
    pending: Map<Element, string>,
    load: (img: HTMLImageElement, url: string) => void,
): () => void {
    if (typeof IntersectionObserver === 'undefined') {
        for (const [img, url] of pending) {
            load(img as HTMLImageElement, url);
        }
        return () => undefined;
    }
    if (pending.size === 0) {
        return () => undefined;
    }
    const observer = new IntersectionObserver(
        (entries) => {
            for (const entry of entries) {
                const url = pending.get(entry.target);
                if (entry.isIntersecting && url !== undefined) {
                    pending.delete(entry.target);
                    observer.unobserve(entry.target);
                    load(entry.target as HTMLImageElement, url);
                }
            }
        },
        { root: scrollParent(root), rootMargin: LAZY_IMAGE_MARGIN },
    );
    for (const img of pending.keys()) {
        observer.observe(img);
    }
    return () => observer.disconnect();
}

/**
 * Routes the body images under `root` through the policed image cache.
 * Remote (`http(s)`) images never load unmediated — the CSP `img-src`
 * forbids `https:`. With `prefetch` off every remote image is hidden
 * (`data-media-off`) and nothing is fetched. With it on, each is marked
 * `data-media-pending` and resolved (→ `asset:`) only once it comes within
 * {@link LAZY_IMAGE_MARGIN} of the reader's scroll viewport; a failed
 * resolution collapses it (`data-media-failed`) instead of leaving an empty,
 * shadowed box. Without `IntersectionObserver` (very old webviews) images
 * resolve eagerly. `zoomHint` is the localized description a loaded,
 * zoomable image carries ("open image larger").
 */
export function mountBodyImages(
    root: HTMLElement,
    prefetch: boolean,
    zoomHint: string,
): MountedImages {
    let disposed = false;
    const { pending, offCount } = routeImages(root, prefetch, zoomHint);

    function load(img: HTMLImageElement, url: string): void {
        void loadCachedImage(url).then((result) => {
            if (disposed) {
                return;
            }
            if (result.status === 'ok') {
                showLoaded(img, result.data, zoomHint);
            } else {
                markImageFailed(img);
            }
        });
    }

    // A decode failure of an already-resolved src (corrupt cache entry,
    // unsupported format) collapses the image too. `error` doesn't bubble,
    // so listen in the capture phase.
    function onError(event: Event): void {
        if (event.target instanceof HTMLImageElement && root.contains(event.target)) {
            markImageFailed(event.target);
        }
    }
    root.addEventListener('error', onError, true);
    const disconnect = loadNearViewport(root, pending, load);

    return {
        offCount,
        dispose(): void {
            disposed = true;
            disconnect();
            root.removeEventListener('error', onError, true);
        },
    };
}

/** Whether the user asked the OS for reduced motion (false where unknowable). */
function prefersReducedMotion(): boolean {
    return (
        typeof window.matchMedia === 'function' &&
        window.matchMedia('(prefers-reduced-motion: reduce)').matches
    );
}

/** The element under `root` whose id is `id` (ids are compared, never put in a selector). */
function findById(root: ParentNode, id: string): HTMLElement | null {
    for (const element of root.querySelectorAll<HTMLElement>('[id]')) {
        if (element.id === id) {
            return element;
        }
    }
    return null;
}

/** Decodes a fragment, or `null` when it is malformed percent-encoding. */
function decodeFragment(fragment: string): string | null {
    try {
        return decodeURIComponent(fragment);
    } catch {
        return null;
    }
}

/**
 * Scrolls a same-document `#fragment` link's target into view inside the
 * reader — footnote refs (`#user-content-fn1`), their back-links, in-page
 * tables of contents. Ingest keeps both the ids and the fragment hrefs in
 * the `user-content-` namespace, so they still match here. The target takes
 * focus (so a screen reader follows the jump and Tab continues from it) and
 * is marked `data-fragment-target` for a brief highlight. Returns whether a
 * target was found; a fragment with no target is simply a no-op — it never
 * navigates the webview.
 */
export function scrollToFragment(root: ParentNode, href: string): boolean {
    const id = decodeFragment(href.replace(/^#/, ''));
    if (id === null || id.length === 0) {
        return false;
    }
    const target = findById(root, id);
    if (target === null) {
        return false;
    }
    for (const previous of root.querySelectorAll('[data-fragment-target]')) {
        previous.removeAttribute('data-fragment-target');
    }
    if (typeof target.scrollIntoView === 'function') {
        target.scrollIntoView({
            behavior: prefersReducedMotion() ? 'auto' : 'smooth',
            block: 'start',
        });
    }
    if (!target.hasAttribute('tabindex')) {
        target.setAttribute('tabindex', '-1');
    }
    target.focus({ preventScroll: true });
    target.setAttribute('data-fragment-target', '');
    return true;
}
