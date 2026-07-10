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

/** Opens `url` in the system handler; silently drops any other scheme. */
export async function openExternal(url: string): Promise<void> {
    if (!isOpenableUrl(url)) {
        return;
    }
    await openUrl(url.trim());
}
