/**
 * The window's chrome (#148). On macOS the title bar is overlaid on the
 * webview: the traffic lights sit inside the leading pane header, and the
 * three pane headers are the window's drag regions (double-click zooms).
 * Linux and Windows keep their native decorations, so nothing changes there.
 *
 * `window_chrome` seeds the state at startup (and after a webview reload);
 * `FullscreenChanged` follows the window in and out of fullscreen, where
 * macOS hides the traffic lights and the inset drops to zero. The store
 * mirrors the answer onto `<html data-titlebar="overlay">`, which switches
 * the `--titlebar-inset` token on (app.css).
 */
import { events, commands } from '$lib/bindings';

/**
 * The attribute Tauri's drag script looks for. `deep`: a press anywhere in
 * the header drags the window, except on a control (button, input, link,
 * tab stop…) or under an explicit `false` — so the header's buttons stay
 * clickable.
 */
export const DRAG_REGION_ATTR = 'data-tauri-drag-region';

/** `<html>` attribute value while the traffic lights take header room. */
const OVERLAY = 'overlay';

/**
 * The `--titlebar-inset` the overlay rule in app.css sets (a test pins the
 * two together): the leading header's extra room for the traffic lights.
 * Layout code that sizes panes in script (the sidebar's resize floor) reads
 * it from here.
 */
export const TITLEBAR_INSET_PX = 66;

/**
 * Whether this webview runs inside the macOS app, judged from the platform
 * alone. Only the fallback reads it: `tauri.conf.json` gives the macOS window
 * its overlay title bar whatever the store believes, so an unanswered query
 * there must still assume the overlay — the traffic lights are on top of the
 * header either way, and without drag regions nothing would move the window.
 */
function macosTauri(): boolean {
    const internals = (globalThis as { __TAURI_INTERNALS__?: { invoke?: unknown } })
        .__TAURI_INTERNALS__;
    return (
        typeof internals?.invoke === 'function' &&
        typeof navigator !== 'undefined' &&
        /\bMac OS X\b|\bMacintosh\b/.test(navigator.userAgent)
    );
}

export class WindowChromeStore {
    /** The OS title bar is overlaid on the webview (macOS). */
    overlay: boolean = $state(false);
    /** The window is fullscreen (the traffic lights are hidden). */
    fullscreen: boolean = $state(false);

    /** Bumped by every event, so a slower seed answer never overwrites one. */
    #generation = 0;

    /** The traffic lights are showing inside the leading header. */
    get inset(): boolean {
        return this.overlay && !this.fullscreen;
    }

    /** The leading header's extra room in px: {@link TITLEBAR_INSET_PX} or 0. */
    get insetPx(): number {
        return this.inset ? TITLEBAR_INSET_PX : 0;
    }

    /**
     * The value for a header's drag-region attribute: `deep` while the
     * headers are the title bar, else none (native decorations drag the
     * window; fullscreen has nothing to drag).
     */
    get dragRegion(): 'deep' | undefined {
        return this.inset ? 'deep' : undefined;
    }

    /**
     * Value for a surface without a header (the recovery screen, the empty
     * reader): a press directly on it drags, never one on its content.
     */
    get dragSurface(): 'true' | undefined {
        return this.inset ? 'true' : undefined;
    }

    /**
     * Subscribes to `FullscreenChanged`, then seeds from `window_chrome`.
     * Never throws. Without an answer the chrome follows the platform: the
     * macOS app was built with the overlay title bar (tauri.conf.json), so
     * it keeps the inset and drag regions; anywhere else (Linux, Windows, a
     * plain browser) the window has native chrome. Returns an unsubscribe
     * for teardown.
     */
    async wire(): Promise<() => void> {
        let unlisten: () => void = () => {};
        try {
            unlisten = await events.fullscreenChanged.listen((event) => {
                this.#generation += 1;
                this.fullscreen = event.payload.fullscreen;
                this.#apply();
            });
            const seededAt = this.#generation;
            const result = await commands.windowChrome();
            if (result.status === 'ok') {
                this.overlay = result.data.overlay_title_bar;
                if (seededAt === this.#generation) {
                    this.fullscreen = result.data.fullscreen;
                }
            } else {
                this.overlay = macosTauri();
            }
        } catch {
            // Nothing answered: no Tauri runtime (a plain browser) or a
            // failed subscription — the platform decides, as above.
            this.overlay = macosTauri();
        }
        this.#apply();
        return unlisten;
    }

    /** Mirrors the inset onto `<html>`, where the CSS token reads it. */
    #apply(): void {
        if (typeof document === 'undefined') {
            return;
        }
        if (this.inset) {
            document.documentElement.dataset.titlebar = OVERLAY;
        } else {
            delete document.documentElement.dataset.titlebar;
        }
    }

    /** Test isolation. */
    reset(): void {
        this.overlay = false;
        this.fullscreen = false;
        this.#generation = 0;
        this.#apply();
    }
}

/** The app-wide singleton. */
export const windowChromeStore = new WindowChromeStore();
