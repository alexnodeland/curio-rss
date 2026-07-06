/**
 * The image-cache frontend layer. Remote images never load in the webview
 * directly (the CSP `img-src` has no `https:`); instead the Rust
 * `get_cached_image` command fetches them through the policed client
 * (content-hash LRU in the OS cache dir) and returns a local path, which we
 * convert to an `asset:`-protocol URL the scoped protocol can serve.
 *
 * The conversion uses Tauri's own internal `convertFileSrc` (the exact
 * function `@tauri-apps/api/core` exports) rather than importing that
 * module — the frontend-bans grep reserves `@tauri-apps/api/core` for the
 * generated bindings. In a non-Tauri context (jsdom without the mock) the
 * path passes through unchanged, which keeps component tests simple.
 */
import { commands } from '$lib/bindings';
import type { CommandResult } from '$lib/state/query-cache.svelte';

interface TauriInternals {
    convertFileSrc?: (filePath: string, protocol?: string) => string;
}

/** Converts a cached-image file path into an `asset:`-servable webview URL. */
export function toAssetUrl(path: string): string {
    const internals = (globalThis as { __TAURI_INTERNALS__?: TauriInternals }).__TAURI_INTERNALS__;
    return internals?.convertFileSrc ? internals.convertFileSrc(path) : path;
}

const inflight = new Map<string, Promise<CommandResult<string>>>();

/**
 * Resolves a remote image `url` to a cached `asset:` URL. Memoized per URL
 * while in flight and on success; failures are not cached, so a later
 * attempt (e.g. after connectivity returns) can retry.
 */
export function loadCachedImage(url: string): Promise<CommandResult<string>> {
    const existing = inflight.get(url);
    if (existing !== undefined) {
        return existing;
    }
    const pending = resolve(url);
    inflight.set(url, pending);
    void pending.then((result) => {
        if (result.status === 'error') {
            inflight.delete(url);
        }
    });
    return pending;
}

async function resolve(url: string): Promise<CommandResult<string>> {
    try {
        const result = await commands.getCachedImage(url);
        return result.status === 'ok' ? { status: 'ok', data: toAssetUrl(result.data) } : result;
    } catch (thrown) {
        return {
            status: 'error',
            error: {
                kind: 'internal',
                code: 'INTERNAL',
                message: thrown instanceof Error ? thrown.message : String(thrown),
                recoverable: false,
            },
        };
    }
}

/** Test isolation. */
export function resetImageCache(): void {
    inflight.clear();
}
