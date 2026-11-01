/**
 * The single sanitized-render component against the hostile ingest corpus
 * (`fixtures/html/xss-corpus.html` — the same fixture core's ammonia gate
 * eats). Everything DOMPurify renders here must be inert: no scripts, no
 * handlers, no frames/forms, no javascript:/non-image data: URIs — while
 * the legitimate content around the attacks survives. Plus the link rule:
 * clicks route to the URL-scoped opener, never a webview navigation.
 */
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import SanitizedHtml from '$components/reader/SanitizedHtml.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { resetImageCache } from '$lib/utils/images';
import { LAZY_IMAGE_MARGIN } from '$lib/utils/reader-content';
import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';

const corpusPath = resolve(
    dirname(fileURLToPath(import.meta.url)),
    '../../../../fixtures/html/xss-corpus.html',
);
const corpus = readFileSync(corpusPath, 'utf8');

// The inertness contract, split into focused assertions so the corpus test
// reads as a checklist and each rule stays under the complexity budget.
function expectNoExecutableElements(root: Element): void {
    expect(
        root.querySelector(
            'script, iframe, object, embed, base, meta, form, style, svg, input, button',
        ),
    ).toBeNull();
}

function expectNoHandlersOrJsUris(root: Element): void {
    for (const element of root.querySelectorAll('*')) {
        const tag = element.tagName.toLowerCase();
        for (const attribute of element.attributes) {
            expect(
                attribute.name.toLowerCase().startsWith('on'),
                `<${tag}> kept handler ${attribute.name}`,
            ).toBe(false);
            expect(
                attribute.value.toLowerCase().includes('javascript:'),
                `<${tag}> kept javascript: in ${attribute.name}`,
            ).toBe(false);
        }
    }
}

function expectNoDataUriVectors(root: Element): void {
    for (const linked of root.querySelectorAll('[href]')) {
        const href = (linked.getAttribute('href') ?? '').trim().toLowerCase();
        expect(href.startsWith('data:'), `kept data: href ${href}`).toBe(false);
    }
    for (const sourced of root.querySelectorAll('[src]')) {
        const src = (sourced.getAttribute('src') ?? '').trim().toLowerCase();
        expect(/^data:(?!image\/)/.test(src), `kept non-image data: src ${src}`).toBe(false);
    }
}

describe('SanitizedHtml', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        prefsStore.reset();
        resetImageCache();
        harness?.teardown();
        harness = null;
    });

    it('renders the hostile xss-corpus fixture inert', () => {
        const { container } = render(SanitizedHtml, { props: { html: corpus } });
        const root = container.querySelector('.sanitized-content');
        expect(root).not.toBeNull();
        if (root === null) {
            return;
        }

        expectNoExecutableElements(root);
        expectNoHandlersOrJsUris(root);
        expectNoDataUriVectors(root);

        // The legitimate content around the attacks survives sanitization.
        expect(root.textContent).toContain('Legitimate');
        expect(root.querySelector('strong')).not.toBeNull();
        expect(root.querySelector('a[href="https://example.com/fine"]')).not.toBeNull();
    });

    it('drops author data-* attributes, whatever they hold', () => {
        const { container } = render(SanitizedHtml, {
            props: {
                html:
                    '<p data-media-off data-foo="bar">text</p>' +
                    '<a href="https://example.com/a" data-href="javascript:alert(1)">a</a>',
            },
        });
        const root = container.querySelector('.sanitized-content');
        for (const element of root?.querySelectorAll('*') ?? []) {
            const names = [...element.attributes].map((attribute) => attribute.name);
            expect(
                names.filter((name) => name.startsWith('data-')),
                `<${element.tagName.toLowerCase()}> kept author data-*`,
            ).toEqual([]);
        }
        expect(root?.querySelector('a[href="https://example.com/a"]')).not.toBeNull();
    });

    it('routes images by their real src, never an author data-orig-src', async () => {
        harness = installIpcHarness({ get_cached_image: '/cache/real.png' });
        prefsStore.mediaPrefetch = true;
        const { container } = render(SanitizedHtml, {
            props: {
                html:
                    '<img src="https://example.com/real.png" ' +
                    'data-orig-src="https://evil.example/spoof.png" alt="spoofed">',
            },
        });
        await flushIpc();
        expect(harness.callsFor('get_cached_image')).toEqual([
            { url: 'https://example.com/real.png' },
        ]);
        const image = container.querySelector('img');
        expect(image?.dataset.origSrc).toBe('https://example.com/real.png');
        expect(image?.getAttribute('src')).toBe('/cache/real.png');
    });

    // A non-remote (data:) image never enters the media pipeline, so nothing would
    // reset an author-supplied data-media-off; the sanitizer has to drop it.
    it('does not let an author data-media-off hide a non-remote image', () => {
        const { container } = render(SanitizedHtml, {
            props: { html: '<img src="data:image/gif;base64,R0lGOD" data-media-off alt="inline">' },
        });
        expect(container.querySelector('img')?.hasAttribute('data-media-off')).toBe(false);
    });

    it('re-sanitizes when the html prop changes', async () => {
        const { container, rerender } = render(SanitizedHtml, {
            props: { html: '<p>first</p>' },
        });
        await rerender({ html: '<em>second</em><script>alert(1)</script>' });
        const root = container.querySelector('.sanitized-content');
        expect(root?.querySelector('em')?.textContent).toBe('second');
        expect(root?.querySelector('script')).toBeNull();
        expect(root?.textContent).not.toContain('first');
    });

    it('routes content link clicks to the URL-scoped opener, never navigation', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><a href="https://example.com/post"><em>go</em></a></p>' },
        });
        const inner = container.querySelector('em');
        expect(inner).not.toBeNull();
        if (inner === null) {
            return;
        }
        await fireEvent.click(inner); // bubbles through the anchor to the root
        const opens = harness.callsFor('plugin:opener|open_url');
        expect(opens).toHaveLength(1);
        expect(opens[0]?.url).toBe('https://example.com/post');
    });

    it('wraps a wide table in a focusable horizontal-scroll container', () => {
        const cells = (tag: string): string =>
            Array.from({ length: 12 }, (_, i) => `<${tag}>col ${i + 1}</${tag}>`).join('');
        const { container } = render(SanitizedHtml, {
            props: {
                html: `<p>before</p><table><thead><tr>${cells('th')}</tr></thead><tbody><tr>${cells('td')}</tr></tbody></table><p>after</p>`,
            },
        });
        const root = container.querySelector('.sanitized-content');
        const table = root?.querySelector('table');
        expect(table?.querySelectorAll('th')).toHaveLength(12);
        const wrapper = table?.parentElement;
        // Contained: the table sits in the scroll wrapper, the wrapper in the
        // reading column — never a sibling the column has to stretch around.
        expect(wrapper?.classList.contains('table-scroll')).toBe(true);
        expect(wrapper?.parentElement).toBe(root);
        expect(wrapper?.getAttribute('tabindex')).toBe('0');
        expect(wrapper?.getAttribute('role')).toBe('group');
        expect(wrapper?.getAttribute('aria-label')).toBe('Scrollable table');
        expect(root?.textContent).toContain('after');
    });

    it('names a captioned table scroll region by its caption', () => {
        const { container } = render(SanitizedHtml, {
            props: {
                html: '<table><caption>Results by year</caption><tr><td>1</td></tr></table>',
            },
        });
        const wrapper = container.querySelector('.table-scroll');
        expect(wrapper?.getAttribute('aria-label')).toBe('Results by year');
    });

    it('scrolls footnote links within the reader, with no opener call', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        const scrolled: Element[] = [];
        const original = Element.prototype.scrollIntoView;
        Element.prototype.scrollIntoView = function (this: Element): void {
            scrolled.push(this);
        };
        try {
            const { container } = render(SanitizedHtml, {
                props: {
                    html:
                        '<p>Claim<sup id="user-content-fnref1"><a href="#user-content-fn1">1</a></sup>.</p>' +
                        '<ol><li id="user-content-fn1">First note. <a href="#user-content-fnref1">↩</a></li></ol>',
                },
            });
            const ref = container.querySelector('a[href="#user-content-fn1"]');
            const note = container.querySelector('li');
            expect(ref).not.toBeNull();
            if (ref === null || note === null) {
                return;
            }
            await fireEvent.click(ref);
            expect(scrolled).toEqual([note]);
            expect(document.activeElement).toBe(note);
            expect(note.hasAttribute('data-fragment-target')).toBe(true);

            // The back-link jumps home the same way, and the highlight moves.
            const back = container.querySelector('a[href="#user-content-fnref1"]');
            if (back === null) {
                return;
            }
            await fireEvent.click(back);
            expect(scrolled[1]?.id).toBe('user-content-fnref1');
            expect(note.hasAttribute('data-fragment-target')).toBe(false);

            // A fragment with no target is a no-op — never a navigation.
            expect(harness.calls).toHaveLength(0);
        } finally {
            Element.prototype.scrollIntoView = original;
        }
    });

    it('ignores fragment links whose target is missing or malformed', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><a href="#user-content-gone">x</a><a href="#%E0%A4%A">y</a></p>' },
        });
        for (const anchor of container.querySelectorAll('a')) {
            const click = new MouseEvent('click', { bubbles: true, cancelable: true });
            anchor.dispatchEvent(click);
            expect(click.defaultPrevented).toBe(true);
        }
        expect(harness.calls).toHaveLength(0);
    });

    it('drops non-http(s) link schemes before they reach IPC', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><a href="ftp://example.com/file">grab</a></p>' },
        });
        const anchor = container.querySelector('a');
        expect(anchor).not.toBeNull();
        if (anchor === null) {
            return;
        }
        await fireEvent.click(anchor);
        expect(harness.calls).toHaveLength(0);
    });
});

/**
 * A controllable IntersectionObserver: records what each observer watches
 * and lets a test bring chosen elements "near the viewport".
 */
class FakeIntersectionObserver {
    static instances: FakeIntersectionObserver[] = [];
    readonly observed = new Set<Element>();
    readonly options: IntersectionObserverInit | undefined;
    private readonly callback: IntersectionObserverCallback;

    constructor(callback: IntersectionObserverCallback, options?: IntersectionObserverInit) {
        this.callback = callback;
        this.options = options;
        FakeIntersectionObserver.instances.push(this);
    }

    observe(target: Element): void {
        this.observed.add(target);
    }

    unobserve(target: Element): void {
        this.observed.delete(target);
    }

    disconnect(): void {
        this.observed.clear();
    }

    takeRecords(): IntersectionObserverEntry[] {
        return [];
    }

    /** Fires the callback as if `targets` scrolled within the root margin. */
    enter(targets: Element[]): void {
        const entries = targets.map(
            (target) => ({ target, isIntersecting: true }) as unknown as IntersectionObserverEntry,
        );
        this.callback(entries, this as unknown as IntersectionObserver);
    }
}

const flush = (): Promise<void> => new Promise((done) => setTimeout(done, 0));

describe('SanitizedHtml body media', () => {
    let harness: IpcHarness | null = null;

    beforeEach(() => {
        prefsStore.mediaPrefetch = true;
    });

    afterEach(() => {
        harness?.teardown();
        harness = null;
        prefsStore.reset();
        resetImageCache();
        FakeIntersectionObserver.instances = [];
        vi.unstubAllGlobals();
    });

    it('only resolves images that come near the viewport', async () => {
        vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
        harness = installIpcHarness({ get_cached_image: '/cache/near.png' });
        const { container } = render(SanitizedHtml, {
            props: {
                html:
                    '<p><img src="https://x.test/1.png" alt="one"></p>' +
                    '<p><img src="https://x.test/2.png" alt="two"></p>' +
                    '<p><img src="https://x.test/3.png" alt="three"></p>',
            },
        });
        const images = Array.from(container.querySelectorAll('img'));
        expect(images.every((img) => img.hasAttribute('data-media-pending'))).toBe(true);
        expect(harness.callsFor('get_cached_image')).toHaveLength(0);

        const observer = FakeIntersectionObserver.instances.at(-1);
        expect(observer?.observed.size).toBe(3);
        expect(observer?.options?.rootMargin).toBe(LAZY_IMAGE_MARGIN);
        const first = images[0];
        if (observer === undefined || first === undefined) {
            return;
        }
        observer.enter([first]);
        await flush();

        expect(harness.callsFor('get_cached_image')).toEqual([{ url: 'https://x.test/1.png' }]);
        expect(first.getAttribute('src')).toBe('/cache/near.png');
        expect(first.hasAttribute('data-media-pending')).toBe(false);
        expect(images[1]?.hasAttribute('src')).toBe(false);
        expect(observer.observed.has(first)).toBe(false);
    });

    it('collapses an image whose fetch fails instead of leaving an empty box', async () => {
        harness = installIpcHarness({
            get_cached_image: () => Promise.reject(commandErrorFixture({ code: 'NETWORK' })),
        });
        const { container } = render(SanitizedHtml, {
            props: { html: '<figure><img src="https://x.test/gone.png" alt="gone"></figure>' },
        });
        await flush();
        const img = container.querySelector('img');
        expect(harness.callsFor('get_cached_image')).toHaveLength(1);
        expect(img?.hasAttribute('data-media-failed')).toBe(true);
        expect(img?.hasAttribute('data-media-pending')).toBe(false);
        expect(img?.hasAttribute('src')).toBe(false);
    });

    it('collapses an image that fails to decode after loading', async () => {
        harness = installIpcHarness({ get_cached_image: '/cache/corrupt.png' });
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><img src="https://x.test/corrupt.png" alt="c"></p>' },
        });
        await flush();
        const img = container.querySelector('img');
        expect(img?.getAttribute('src')).toBe('/cache/corrupt.png');
        img?.dispatchEvent(new Event('error'));
        expect(img?.hasAttribute('data-media-failed')).toBe(true);
    });

    it('reserves the aspect ratio from width/height before the image loads', () => {
        vi.stubGlobal('IntersectionObserver', FakeIntersectionObserver);
        const { container } = render(SanitizedHtml, {
            props: {
                html: '<p><img src="https://x.test/w.png" width="640" height="360" alt="w"></p>',
            },
        });
        const img = container.querySelector('img');
        expect(img?.style.aspectRatio).toBe('640 / 360');
    });

    it('opens a loaded image in a lightbox that Escape closes, restoring focus', async () => {
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><img src="data:image/png;base64,AAAA" alt="A chart"></p>' },
        });
        const img = container.querySelector('img');
        expect(img?.getAttribute('tabindex')).toBe('0');
        if (img === null) {
            return;
        }
        img.focus();
        await fireEvent.keyDown(img, { key: 'Enter' });
        const dialog = document.querySelector('[data-testid="reader-lightbox"] [role="dialog"]');
        expect(dialog).not.toBeNull();
        expect(dialog?.getAttribute('aria-label')).toBe('A chart');
        expect(document.activeElement).toBe(dialog);

        // App shortcuts underneath never see the lightbox's keys.
        const shortcut = vi.fn();
        window.addEventListener('keydown', shortcut);
        try {
            if (dialog === null) {
                return;
            }
            await fireEvent.keyDown(dialog, { key: 'j' });
            await fireEvent.keyDown(dialog, { key: 'Escape' });
        } finally {
            window.removeEventListener('keydown', shortcut);
        }
        expect(shortcut).not.toHaveBeenCalled();
        expect(document.querySelector('[data-testid="reader-lightbox"]')).toBeNull();
        expect(document.activeElement).toBe(img);
    });

    it('opens on click and closes from the close button', async () => {
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><img src="data:image/png;base64,AAAA" alt=""></p>' },
        });
        const img = container.querySelector('img');
        if (img === null) {
            return;
        }
        await fireEvent.click(img);
        const dialog = document.querySelector('[role="dialog"]');
        expect(dialog?.getAttribute('aria-label')).toBe('Image viewer');
        const close = document.querySelector<HTMLButtonElement>('.lightbox-close');
        if (close === null) {
            return;
        }
        await fireEvent.click(close);
        expect(document.querySelector('[role="dialog"]')).toBeNull();
    });

    it('never zooms an image inside a link — the link wins', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        const { container } = render(SanitizedHtml, {
            props: {
                html: '<a href="https://x.test/post"><img src="data:image/png;base64,AAAA" alt="x"></a>',
            },
        });
        const img = container.querySelector('img');
        if (img === null) {
            return;
        }
        expect(img.hasAttribute('tabindex')).toBe(false);
        await fireEvent.click(img);
        expect(document.querySelector('[role="dialog"]')).toBeNull();
        expect(harness.callsFor('plugin:opener|open_url')).toHaveLength(1);
    });

    it('never fetches with media prefetch off', () => {
        prefsStore.mediaPrefetch = false;
        harness = installIpcHarness({});
        const { container } = render(SanitizedHtml, {
            props: { html: '<p><img src="https://x.test/off.png" alt="off"></p>' },
        });
        expect(container.querySelector('img')?.hasAttribute('data-media-off')).toBe(true);
        expect(harness.calls).toHaveLength(0);
    });
});
