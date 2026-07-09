/**
 * Vitest setup. Node >= 22 defines an experimental `localStorage` global
 * that is `undefined` unless node runs with `--localstorage-file`; because
 * the key already exists on globalThis, vitest's populateGlobal skips
 * copying jsdom's real Storage over it. Bridge jsdom's implementation back
 * in so `window.localStorage` behaves like a browser.
 */
const jsdomInstance = (globalThis as { jsdom?: { window: Window & typeof globalThis } }).jsdom;

if (typeof globalThis.localStorage === 'undefined' && jsdomInstance !== undefined) {
    for (const key of ['localStorage', 'sessionStorage'] as const) {
        Object.defineProperty(globalThis, key, {
            value: jsdomInstance.window[key],
            configurable: true,
            writable: true,
        });
    }
}

/**
 * jsdom has no ResizeObserver, which Svelte's `bind:clientHeight` needs.
 * A no-op stub is enough: components that measure fall back to explicit
 * fallback dimensions in tests (see VirtualList's fallbackViewportHeight).
 */
if (typeof globalThis.ResizeObserver === 'undefined') {
    class ResizeObserverStub {
        observe(): void {
            // no layout in jsdom — nothing to observe
        }
        unobserve(): void {
            // no-op
        }
        disconnect(): void {
            // no-op
        }
    }
    globalThis.ResizeObserver = ResizeObserverStub as unknown as typeof ResizeObserver;
}

/**
 * jsdom has no `window.CSS`, which the theme import validator uses
 * (`CSS.supports('color', v)`). A faithful-enough minimal validator: accept
 * the color forms the built-in themes use (hex, rgb/rgba, hsl/hsla, named,
 * `var()`), and reject anything carrying a CSS declaration/selector breakout.
 * Production calls the real `CSS.supports`.
 */
if (typeof globalThis.CSS === 'undefined') {
    const supports = (property: string, value: string): boolean => {
        if (property !== 'color' || typeof value !== 'string' || value.length === 0) return false;
        if (/[;{}<>]/.test(value)) return false;
        return (
            /^#([0-9a-f]{3,4}|[0-9a-f]{6}|[0-9a-f]{8})$/i.test(value) ||
            /^(rgb|hsl)a?\([^)]*\)$/i.test(value) ||
            /^var\(--[a-z0-9-]+\)$/i.test(value) ||
            /^[a-z]+$/i.test(value)
        );
    };
    globalThis.CSS = { supports } as unknown as typeof CSS;
}

export {};
