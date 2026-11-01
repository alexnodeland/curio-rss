/**
 * The reader-body DOM helpers on their own: table wrapping is idempotent,
 * the lazy-load observer roots on the reader's scroll container, a disposed
 * mount drops late results, and fragment jumps honour reduced motion.
 */
import { resetImageCache } from '$lib/utils/images';
import {
    isZoomable,
    markImageFailed,
    mountBodyImages,
    reserveAspectRatio,
    scrollParent,
    scrollToFragment,
    watchTableOverflow,
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

    it('wraps a table at the top of a detached fragment', () => {
        const template = document.createElement('template');
        template.innerHTML = '<table><tr><td>a</td></tr></table>';
        wrapTables(template.content, 'Scrollable table');
        expect(template.content.firstElementChild?.classList.contains('table-scroll')).toBe(true);
    });

    it('re-checks overflow as the column resizes, and disconnects', () => {
        const observers: FakeResizeObserver[] = [];
        class FakeResizeObserver {
            readonly observed: Element[] = [];
            disconnected = false;
            constructor(readonly callback: ResizeObserverCallback) {
                observers.push(this);
            }
            observe(target: Element): void {
                this.observed.push(target);
            }
            unobserve(): void {}
            disconnect(): void {
                this.disconnected = true;
            }
        }
        vi.stubGlobal('ResizeObserver', FakeResizeObserver);
        const root = fragment('<table><tr><td>a</td></tr></table>');
        wrapTables(root, 'Scrollable table');
        const wrapper = root.querySelector<HTMLElement>('.table-scroll');
        const table = root.querySelector('table');
        if (wrapper === null || table === null) {
            throw new Error('fixture');
        }
        let scrollWidth = 300;
        Object.defineProperty(wrapper, 'scrollWidth', { get: () => scrollWidth });
        Object.defineProperty(wrapper, 'clientWidth', { get: () => 500 });

        const disconnect = watchTableOverflow(root);
        expect(wrapper.hasAttribute('tabindex')).toBe(false);
        const [observer] = observers;
        if (observer === undefined) {
            throw new Error('no ResizeObserver created');
        }
        expect(observer.observed).toEqual([wrapper, table]);

        // The table outgrows the column: a resize on the table (not the
        // wrapper) promotes the wrapper to a named scroll region.
        scrollWidth = 900;
        observer.callback(
            [{ target: table } as unknown as ResizeObserverEntry],
            observer as unknown as ResizeObserver,
        );
        expect(wrapper.getAttribute('tabindex')).toBe('0');
        expect(wrapper.getAttribute('role')).toBe('region');
        expect(wrapper.getAttribute('aria-label')).toBe('Scrollable table');

        disconnect();
        expect(observer.disconnected).toBe(true);
    });

    it('creates no ResizeObserver for a body without tables', () => {
        const created = vi.fn();
        vi.stubGlobal(
            'ResizeObserver',
            class {
                constructor() {
                    created();
                }
            },
        );
        watchTableOverflow(fragment('<p>no tables</p>'))();
        expect(created).not.toHaveBeenCalled();
    });

    it('drops the zoom affordances from an image that fails', () => {
        const root = fragment('<img src="data:image/png;base64,AA" alt="x">');
        const mounted = mountBodyImages(root, true, 'Open image larger');
        const img = root.querySelector('img');
        if (img === null) {
            throw new Error('fixture');
        }
        expect(img.getAttribute('role')).toBe('button');
        markImageFailed(img);
        for (const name of ['tabindex', 'role', 'aria-haspopup', 'aria-description']) {
            expect(img.hasAttribute(name), name).toBe(false);
        }
        mounted.dispose();
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
        expect(good.style.aspectRatio).toBe('auto 4 / 3');
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
        const mounted = mountBodyImages(root, true, 'Open image larger');
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
        const mounted = mountBodyImages(root, true, 'Open image larger');
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
