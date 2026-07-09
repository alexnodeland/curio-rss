/**
 * Copy text to the clipboard from the webview. `clipboard-write` is already
 * permitted by the CSP; there's no Tauri clipboard plugin, so this uses the
 * standard async Clipboard API and reports success so callers can toast.
 */
export async function copyText(text: string): Promise<boolean> {
    try {
        await navigator.clipboard.writeText(text);
        return true;
    } catch {
        return false;
    }
}
