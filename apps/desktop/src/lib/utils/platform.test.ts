/**
 * The Apple-platform question behind ⌘ vs Ctrl: answered from the webview's
 * navigator, so the packaged WKWebView, the dev browser and jsdom agree.
 */
import { hasCommandModifier, isApplePlatform } from '$lib/utils/platform';
import { describe, expect, it } from 'vitest';

describe('isApplePlatform', () => {
    it('recognises macOS and iOS webviews', () => {
        expect(isApplePlatform({ platform: 'MacIntel' })).toBe(true);
        expect(
            isApplePlatform({
                userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X 14_5) AppleWebKit/605.1.15',
            }),
        ).toBe(true);
        expect(isApplePlatform({ platform: 'iPad' })).toBe(true);
    });

    it('is false elsewhere, and without a navigator', () => {
        expect(isApplePlatform({ platform: 'Linux x86_64', userAgent: 'X11; Linux' })).toBe(false);
        expect(isApplePlatform({ platform: 'Win32' })).toBe(false);
        expect(isApplePlatform({})).toBe(false);
        // jsdom (the test environment) is not a Mac.
        expect(isApplePlatform()).toBe(false);
    });
});

describe('hasCommandModifier', () => {
    it('is ⌘ on Apple platforms and Ctrl elsewhere', () => {
        expect(hasCommandModifier({ metaKey: true, ctrlKey: false }, true)).toBe(true);
        expect(hasCommandModifier({ metaKey: false, ctrlKey: true }, true)).toBe(false);
        expect(hasCommandModifier({ metaKey: false, ctrlKey: true }, false)).toBe(true);
        expect(hasCommandModifier({ metaKey: true, ctrlKey: false }, false)).toBe(false);
        expect(hasCommandModifier({ metaKey: false, ctrlKey: true })).toBe(true);
    });
});
