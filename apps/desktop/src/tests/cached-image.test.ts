/**
 * CachedImage renders a remote image only once it resolves through the Rust
 * image cache, and calls `onfail` (never a broken-image glyph) when it does
 * not — the hook the add-feed favicon uses to try a fallback source.
 */
import CachedImage from '$components/common/CachedImage.svelte';
import { resetImageCache } from '$lib/utils/images';
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, commandErrorFixture, flushIpc, installIpcHarness } from './ipc-harness';

describe('CachedImage', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetImageCache();
        harness?.teardown();
        harness = null;
    });

    it('renders the resolved image once the cache answers', async () => {
        harness = installIpcHarness({ get_cached_image: '/cache/logo.png' });
        const { getByAltText } = render(CachedImage, {
            src: 'https://x.test/logo.png',
            alt: 'Logo',
        });
        await flushIpc();
        const image = getByAltText('Logo') as HTMLImageElement;
        expect(image.getAttribute('src')).toBe('/cache/logo.png');
    });

    it('renders nothing and calls onfail when the cache errors', async () => {
        const onfail = vi.fn();
        harness = installIpcHarness({
            get_cached_image: () => Promise.reject(commandErrorFixture({ code: 'NETWORK' })),
        });
        const { queryByAltText } = render(CachedImage, {
            src: 'https://x.test/missing.png',
            alt: 'Missing',
            onfail,
        });
        await flushIpc();
        expect(queryByAltText('Missing')).toBeNull();
        expect(onfail).toHaveBeenCalledOnce();
    });
});
