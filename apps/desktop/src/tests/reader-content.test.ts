/**
 * The reader-body DOM helpers on their own: table wrapping is idempotent,
 * the lazy-load observer roots on the reader's scroll container, a disposed
 * mount drops late results, and fragment jumps honour reduced motion.
 */
import { resetImageCache } from '$lib/utils/images';
import {
    isZoomable,
    mountBodyImages,
    reserveAspectRatio,
    scrollParent,
    scrollToFragment,
    wrapTables,
} from '$lib/utils/reader-content';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

function fragment(html: string): HTMLElement {
    const root = document.createElement('div');
    root.innerHTML = html;
    document.body.append(root);
    return root;
}

describe('reader-content helpers', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        harness?.teardown();
        harness = null;
        resetImageCache();
        vi.unstubAllGlobals();
        document.body.innerHTML = '';
    });

    it('wraps each table exactly once', () => {
        const root = fragment(
            '<table><tr><td>a</td></tr></table><table><tr><td>b</td></tr></table>',
        );
        wrapTables(root, 'Scrollable table');
        wrapTables(root, 'Scrollable table');
        expect(root.querySelectorAll('.table-scroll')).toHaveLength(2);
        expect(root.querySelectorAll('.table-scroll .table-scroll')).toHaveLength(0);
    });

    it('finds the nearest scrolling ancestor, or none', () => {
        const root = fragment('<div class="scroller"><div class="inner"><p>x</p></div></div>');
        const scroller = root.querySelector<HTMLElement>('.scroller');
        const paragraph = root.querySelector('p');
        if (scroller === null || paragraph === null) {
            throw new Error('fixture');
        }
        expect(scrollParent(paragraph)).toBeNull();
        scroller.style.overflowY = 'auto';
        expect(scrollParent(paragraph)).toBe(scroller);
    });

    it('reserves an aspect ratio only for a usable width/height pair', () => {
        const root = fragment(
            '<img width="4" height="3"><img width="4"><img width="0" height="3"><img width="x" height="3">',
        );
        const [good, ...bad] = Array.from(root.querySelectorAll('img'));
        if (good === undefined) {
            throw new Error('fixture');
        }
        reserveAspectRatio(good);
        expect(good.style.aspectRatio).toBe('4 / 3');
        for (const img of bad) {
            reserveAspectRatio(img);
            expect(img.style.aspectRatio).toBe('');
        }
    });

    it('zooms only loaded images outside links', () => {
        const root = fragment(
            '<img id="a" src="data:image/png;base64,AA"><img id="b"><a href="https://x.test"><img id="c" src="data:image/png;base64,AA"></a><p id="d"></p>',
        );
        const byId = (id: string): Element => root.querySelector(`#${id}`) as Element;
        expect(isZoomable(byId('a'))).toBe(true);
        expect(isZoomable(byId('b'))).toBe(false);
        expect(isZoomable(byId('c'))).toBe(false);
        expect(isZoomable(byId('d'))).toBe(false);
    });

    it('drops a resolution that lands after dispose', async () => {
        let release: (value: string) => void = () => undefined;
        harness = installIpcHarness({
            get_cached_image: () =>
                new Promise<string>((done) => {
                    release = done;
                }),
        });
        const root = fragment('<img src="https://x.test/slow.png" alt="slow">');
        const mounted = mountBodyImages(root, true);
        mounted.dispose();
        release('/cache/slow.png');
        await new Promise((done) => setTimeout(done, 0));
        const img = root.querySelector('img');
        expect(img?.hasAttribute('src')).toBe(false);
        expect(img?.hasAttribute('data-media-pending')).toBe(true);
    });

    it('creates no observer when nothing needs loading', () => {
        const created = vi.fn();
        vi.stubGlobal(
            'IntersectionObserver',
            class {
                constructor() {
                    created();
                }
            },
        );
        const root = fragment('<img src="data:image/png;base64,AA" alt="inline">');
        const mounted = mountBodyImages(root, true);
        expect(created).not.toHaveBeenCalled();
        expect(mounted.offCount).toBe(0);
        mounted.dispose();
    });

    it('jumps without smooth scrolling under reduced motion, keeping an existing tabindex', () => {
        vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('reduce') }));
        const root = fragment('<h2 id="user-content-part-2" tabindex="0">Part two</h2>');
        const heading = root.querySelector('h2');
        if (heading === null) {
            throw new Error('fixture');
        }
        const scroll = vi.fn();
        heading.scrollIntoView = scroll;
        expect(scrollToFragment(root, '#user-content-part-2')).toBe(true);
        expect(scroll).toHaveBeenCalledWith({ behavior: 'auto', block: 'start' });
        expect(heading.getAttribute('tabindex')).toBe('0');
        expect(scrollToFragment(root, '#')).toBe(false);
    });
});
