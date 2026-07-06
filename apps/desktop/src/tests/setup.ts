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

export {};
