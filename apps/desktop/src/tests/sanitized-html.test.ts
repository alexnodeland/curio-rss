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
import { fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

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
