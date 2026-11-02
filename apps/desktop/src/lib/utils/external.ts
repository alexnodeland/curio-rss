/**
 * The one open-in-OS-handler path: the URL-scoped opener plugin. The capability
 * allows exactly `http`/`https` (system browser) and `mailto`/`tel` (system
 * mail / dialer) — mirrored in {@link isOpenableUrl} so a hostile href fails
 * closed before it ever reaches IPC. Every other scheme is dropped. Never
 * `window.open`.
 */
import { openUrl } from '@tauri-apps/plugin-opener';

/**
 * Whether the opener capability would accept this URL at all — the exact scheme
 * set the Tauri opener ACL allows. `http(s)` carry a `//` authority; `mailto`/
 * `tel` do not. Keep this in lockstep with `capabilities/main.json`.
 */
export function isOpenableUrl(url: string): boolean {
    return /^(?:https?:\/\/|mailto:|tel:)/i.test(url.trim());
}

/**
 * Whether `url` is a web page address — the only kind of article source the UI
 * renders as a link or offers to open. The same screen core's policed client
 * applies (`fetch::screen_url`): an absolute `http(s)` URL with a host and no
 * userinfo. Core stores nothing else as a feed entry's source (#262); this
 * keeps a value from an older build, or any other surprise, from ever becoming
 * a live `href`.
 */
export function isWebUrl(url: string): boolean {
    const trimmed = url.trim();
    if (!/^https?:\/\//i.test(trimmed)) {
        return false;
    }
    let parsed: URL;
    try {
        parsed = new URL(trimmed);
    } catch {
        return false;
    }
    return parsed.hostname !== '' && parsed.username === '' && parsed.password === '';
}

/** Opens `url` in the system handler; silently drops any other scheme. */
export async function openExternal(url: string): Promise<void> {
    if (!isOpenableUrl(url)) {
        return;
    }
    await openUrl(url.trim());
}
