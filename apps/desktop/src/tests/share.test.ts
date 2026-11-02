/**
 * The share sheet (#153): offered only where the webview has one, opened
 * with the article's URL and title, a dismissed sheet is no error, and only a
 * sheet that could not open is toasted.
 */
import { shareLink } from '$lib/state/actions';
import { toastStore } from '$lib/state/toasts.svelte';
import { canShareLinks, shareUrl } from '$lib/utils/share';
import { afterEach, describe, expect, it, vi } from 'vitest';

function stubShare(
    share: (data: ShareData) => Promise<void>,
    canShare?: (data: ShareData) => boolean,
): void {
    Object.defineProperty(navigator, 'share', { value: share, configurable: true });
    Object.defineProperty(navigator, 'canShare', { value: canShare, configurable: true });
}

afterEach(() => {
    Reflect.deleteProperty(navigator, 'share');
    Reflect.deleteProperty(navigator, 'canShare');
    toastStore.reset();
});

describe('share', () => {
    it('is not offered where the webview has no share sheet', async () => {
        expect(canShareLinks()).toBe(false);
        expect(await shareUrl('https://example.org/a', 'A')).toBe('failed');
    });

    it('opens the sheet with the URL and title', async () => {
        const share = vi.fn().mockResolvedValue(undefined);
        stubShare(share);
        expect(canShareLinks()).toBe(true);
        expect(await shareUrl('https://example.org/a', 'A title')).toBe('shared');
        expect(share).toHaveBeenCalledWith({ url: 'https://example.org/a', title: 'A title' });
    });

    it('a sheet closed unused is dismissed, anything else failed', async () => {
        stubShare(vi.fn().mockRejectedValue(new DOMException('cancelled', 'AbortError')));
        expect(await shareUrl('https://example.org/a', 'A')).toBe('dismissed');
        stubShare(vi.fn().mockRejectedValue(new DOMException('no gesture', 'NotAllowedError')));
        expect(await shareUrl('https://example.org/a', 'A')).toBe('failed');
        stubShare(vi.fn().mockRejectedValue('odd'));
        expect(await shareUrl('https://example.org/a', 'A')).toBe('failed');
    });

    it('does not open the sheet for data it cannot share', async () => {
        const share = vi.fn().mockResolvedValue(undefined);
        stubShare(share, () => false);
        expect(await shareUrl('https://example.org/a', 'A')).toBe('failed');
        expect(share).not.toHaveBeenCalled();
    });

    it('shareLink toasts only a sheet that could not open', async () => {
        stubShare(vi.fn().mockResolvedValue(undefined));
        await shareLink('https://example.org/a', 'A');
        stubShare(vi.fn().mockRejectedValue(new DOMException('cancelled', 'AbortError')));
        await shareLink('https://example.org/a', 'A');
        expect(toastStore.toasts).toHaveLength(0);

        stubShare(vi.fn().mockRejectedValue(new DOMException('no gesture', 'NotAllowedError')));
        await shareLink('https://example.org/a', 'A');
        expect(toastStore.toasts.map((toast) => toast.message)).toEqual([
            'Couldn’t open the share sheet',
        ]);
    });
});
