/**
 * The OS share sheet, from the webview: the Web Share API, which WKWebView
 * answers with the native macOS share picker (Mail, Messages, AirDrop, Notes,
 * Reminders…) and WebView2 with the Windows share dialog — no plugin or
 * native code. WebKitGTK has no share sheet, so the surfaces offer Share only
 * where {@link canShareLinks} finds one; Copy link is offered everywhere.
 */

/** Whether this webview has a share sheet. */
export function canShareLinks(): boolean {
    return typeof navigator !== 'undefined' && typeof navigator.share === 'function';
}

/** How a share ended: sent somewhere, the sheet closed unused, or it failed. */
export type ShareOutcome = 'shared' | 'dismissed' | 'failed';

/**
 * Opens the share sheet for `url`, titled `title`. Must run in a user
 * gesture (a click, a menu pick) — the engine refuses it otherwise.
 */
export async function shareUrl(url: string, title: string): Promise<ShareOutcome> {
    const data: ShareData = { url, title };
    if (!canShareLinks()) {
        return 'failed';
    }
    if (typeof navigator.canShare === 'function' && !navigator.canShare(data)) {
        return 'failed';
    }
    try {
        await navigator.share(data);
        return 'shared';
    } catch (error) {
        // Closing the sheet without picking anything rejects with AbortError.
        const name = error instanceof Error || error instanceof DOMException ? error.name : '';
        return name === 'AbortError' ? 'dismissed' : 'failed';
    }
}
