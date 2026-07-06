/**
 * The image-cache frontend layer: remote URLs resolve through the Rust
 * `get_cached_image` command (memoized on success, retried on failure), and
 * the asset-URL conversion degrades to a passthrough without Tauri internals
 * (jsdom), keeping the module testable without importing the banned
 * `@tauri-apps/api/core` module.
 */
import { loadCachedImage, resetImageCache, toAssetUrl } from '$lib/utils/images';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, commandErrorFixture, installIpcHarness } from './ipc-harness';

describe('image cache util', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetImageCache();
        harness?.teardown();
        harness = null;
    });

    it('resolves a remote URL through get_cached_image', async () => {
        harness = installIpcHarness({ get_cached_image: '/cache/curio/images/abc.png' });
        const result = await loadCachedImage('https://x.test/logo.png');
        // No Tauri internals in jsdom → toAssetUrl passes the path through.
        expect(result).toEqual({ status: 'ok', data: '/cache/curio/images/abc.png' });
        expect(harness.callsFor('get_cached_image')).toEqual([{ url: 'https://x.test/logo.png' }]);
    });

    it('memoizes successful resolutions — one fetch per URL', async () => {
        harness = installIpcHarness({ get_cached_image: '/cache/x.png' });
        await loadCachedImage('https://x.test/a.png');
        await loadCachedImage('https://x.test/a.png');
        expect(harness.callsFor('get_cached_image')).toHaveLength(1);
    });

    it('does not cache failures — a later attempt retries', async () => {
        let calls = 0;
        harness = installIpcHarness({
            get_cached_image: () => {
                calls += 1;
                return Promise.reject(commandErrorFixture({ code: 'NETWORK', message: 'boom' }));
            },
        });
        const first = await loadCachedImage('https://x.test/b.png');
        expect(first.status).toBe('error');
        await loadCachedImage('https://x.test/b.png');
        expect(calls).toBe(2);
    });

    it('folds a thrown (non-command) error into the internal arm', async () => {
        harness = installIpcHarness({});
        const result = await loadCachedImage('https://x.test/c.png');
        expect(result.status).toBe('error');
    });

    it('toAssetUrl passes the path through when no Tauri converter is present', () => {
        expect(toAssetUrl('/cache/x.png')).toBe('/cache/x.png');
    });
});
