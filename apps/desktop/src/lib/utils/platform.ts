/**
 * The one platform question the UI asks: is this an Apple machine? There ⌘
 * is the command modifier (⌘K, ⌘-click) and ⌃-click is a right-click; on
 * Linux and Windows Ctrl plays ⌘'s part. Read from the webview's navigator
 * (WKWebView reports `MacIntel` / a `Macintosh` user agent), so it answers
 * the same in the packaged app, the dev browser and jsdom (never Apple).
 */

/** The navigator fields the answer is read from (injectable for tests). */
export interface PlatformHints {
    readonly platform?: string;
    readonly userAgent?: string;
}

/** Whether `hints` (default: the webview's navigator) describe macOS or iOS. */
export function isApplePlatform(hints: PlatformHints | undefined = globalThis.navigator): boolean {
    if (hints === undefined) {
        return false;
    }
    return /Mac|iPhone|iPad|iPod/.test(`${hints.platform ?? ''} ${hints.userAgent ?? ''}`);
}

/** Whether `event` holds the platform's command modifier (⌘ on Apple, else Ctrl). */
export function hasCommandModifier(
    event: Pick<KeyboardEvent, 'metaKey' | 'ctrlKey'>,
    apple: boolean = isApplePlatform(),
): boolean {
    return apple ? event.metaKey : event.ctrlKey;
}
