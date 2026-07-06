/**
 * The one open-in-OS-browser path: the URL-scoped opener plugin (the
 * capability allows `http`/`https` only — mirrored here so a hostile href
 * fails closed before it ever reaches IPC). Never `window.open`.
 */
import { openUrl } from '@tauri-apps/plugin-opener';

/** Whether the opener capability would accept this URL at all. */
export function isOpenableUrl(url: string): boolean {
    return /^https?:\/\//i.test(url.trim());
}

/** Opens `url` in the system browser; silently drops non-http(s) schemes. */
export async function openExternal(url: string): Promise<void> {
    if (!isOpenableUrl(url)) {
        return;
    }
    await openUrl(url.trim());
}
