/**
 * Shell layout state: the three-pane widths, which panes show (sidebar
 * collapse, focus mode, the narrow-window auto-collapse), the per-feed-type
 * "home" list layout, and the single-open modal enum (plus the edit-feed
 * modal's binding). Pane state persists to the settings table; the window
 * width, a sidebar peeked open in a narrow window and the modal are
 * session-only.
 */
import { SETTING_KEYS, settingsStore } from './settings.svelte';

export type ModalKind = 'add-feed' | 'settings' | 'help' | 'destinations' | 'edit-feed';

/** Which section the edit-feed modal opens focused on. */
export type EditFeedSection = 'details' | 'health';

/** Resize bounds per pane — ThreePane and `initLayout` share them. */
export const PANE_LIMITS = {
    sidebar: { min: 200, max: 420 },
    list: { min: 260, max: 600 },
} as const;

/**
 * The window widths at which the sidebar collapses on its own, so the list and
 * the reader keep a usable width. Matches ThreePane's narrow-width caps.
 */
export const NARROW_QUERY = '(max-width: 1000px)';

export class LayoutStore {
    /** The user's sidebar choice for a wide window (persisted). */
    sidebarCollapsed: boolean = $state(false);
    /** Focus mode: the reader alone, sidebar and list hidden (persisted). */
    focusMode: boolean = $state(false);
    /** The window is at or below {@link NARROW_QUERY} (session). */
    narrow: boolean = $state(false);
    /**
     * The sidebar opened by hand while the window is narrow. Session-only and
     * dropped whenever the window crosses the breakpoint, so the stored wide
     * choice is never rewritten by a narrow window.
     */
    sidebarPeek: boolean = $state(false);
    /**
     * Whether pane show/hide animates. Off until the layout first changes
     * after launch, so adopting the persisted layout never slides a pane.
     */
    paneMotion: boolean = $state(false);
    sidebarWidth: number = $state(280);
    listWidth: number = $state(360);

    /**
     * Per-feed-type list layout: when a YouTube- or Reddit-typed feed is
     * selected, the middle pane can render its "home page" (a video grid /
     * a post feed) instead of the compact rows. Remembered per type.
     */
    homeYoutube: boolean = $state(false);
    homeReddit: boolean = $state(false);

    activeModal: ModalKind | null = $state(null);

    /** The feed the edit-feed modal is bound to while it is open. */
    editFeedId: number | null = $state(null);
    /** Which section the edit-feed modal scrolls to on open. */
    editFeedSection: EditFeedSection = $state('details');

    /**
     * Adopts persisted pane layout at startup (after `settingsStore.load()`).
     * Unparseable or out-of-range values fall back to the defaults.
     */
    initLayout(): void {
        const collapsed = settingsStore.get(SETTING_KEYS.sidebarCollapsed);
        if (collapsed !== undefined) {
            this.sidebarCollapsed = collapsed === 'true';
        }
        this.focusMode = settingsStore.get(SETTING_KEYS.focusMode) === 'true';
        this.sidebarWidth = readWidth(SETTING_KEYS.sidebarWidth, PANE_LIMITS.sidebar, 280);
        this.listWidth = readWidth(SETTING_KEYS.listWidth, PANE_LIMITS.list, 360);
        this.homeYoutube = settingsStore.get(SETTING_KEYS.homeYoutube) === 'true';
        this.homeReddit = settingsStore.get(SETTING_KEYS.homeReddit) === 'true';
    }

    /** Whether the "home" layout is on for a feed type. */
    isHomeLayout(type: 'youtube' | 'reddit'): boolean {
        return type === 'youtube' ? this.homeYoutube : this.homeReddit;
    }

    /** Toggle/set the "home" layout for a feed type, persisted. */
    async setHomeLayout(type: 'youtube' | 'reddit', on: boolean): Promise<void> {
        if (type === 'youtube') {
            this.homeYoutube = on;
            await settingsStore.set(SETTING_KEYS.homeYoutube, String(on));
        } else {
            this.homeReddit = on;
            await settingsStore.set(SETTING_KEYS.homeReddit, String(on));
        }
    }

    /** Whether the sidebar is on screen. */
    get sidebarVisible(): boolean {
        return !this.focusMode && (this.narrow ? this.sidebarPeek : !this.sidebarCollapsed);
    }

    /** Whether the article list is on screen (hidden only in focus mode). */
    get listVisible(): boolean {
        return !this.focusMode;
    }

    /**
     * Shows or hides the sidebar. A wide window flips (and persists) the
     * stored choice; a narrow one only peeks it open or shut for the session.
     * From focus mode it brings the sidebar back (see {@link revealSidebar}).
     */
    async toggleSidebar(): Promise<void> {
        if (this.focusMode) {
            await this.revealSidebar();
            return;
        }
        this.paneMotion = true;
        if (this.narrow) {
            this.sidebarPeek = !this.sidebarPeek;
            return;
        }
        this.sidebarCollapsed = !this.sidebarCollapsed;
        await settingsStore.set(SETTING_KEYS.sidebarCollapsed, String(this.sidebarCollapsed));
    }

    /**
     * Puts the sidebar on screen whatever hid it — focus mode (left), a
     * stored collapse (cleared) or a narrow window (peeked open) — so a
     * caller about to focus it never aims at a hidden pane.
     */
    async revealSidebar(): Promise<void> {
        if (this.sidebarVisible) {
            return;
        }
        this.paneMotion = true;
        if (this.focusMode) {
            this.focusMode = false;
            await settingsStore.set(SETTING_KEYS.focusMode, 'false');
        }
        if (this.narrow) {
            this.sidebarPeek = true;
        } else if (this.sidebarCollapsed) {
            this.sidebarCollapsed = false;
            await settingsStore.set(SETTING_KEYS.sidebarCollapsed, 'false');
        }
    }

    /** Enters or leaves focus mode (the reader alone), persisted. */
    async toggleFocusMode(): Promise<void> {
        this.paneMotion = true;
        this.focusMode = !this.focusMode;
        await settingsStore.set(SETTING_KEYS.focusMode, String(this.focusMode));
    }

    /**
     * Records the window crossing the narrow breakpoint: the sidebar folds
     * away below it and returns above it, and a peek never outlives the
     * crossing.
     */
    setNarrow(narrow: boolean): void {
        if (narrow === this.narrow) {
            return;
        }
        this.paneMotion = true;
        this.narrow = narrow;
        this.sidebarPeek = false;
    }

    /**
     * Follows the window width through {@link NARROW_QUERY}. The first
     * reading lands without motion; later crossings animate. Returns the
     * unsubscribe; a host without `matchMedia` (jsdom) is a no-op.
     */
    watchViewport(win: Pick<Window, 'matchMedia'> = window): () => void {
        if (typeof win.matchMedia !== 'function') {
            return () => {};
        }
        const query = win.matchMedia(NARROW_QUERY);
        this.narrow = query.matches;
        const onChange = (event: MediaQueryListEvent): void => this.setNarrow(event.matches);
        query.addEventListener('change', onChange);
        return () => query.removeEventListener('change', onChange);
    }

    openModal(modal: ModalKind): void {
        this.activeModal = modal;
    }

    /** Opens the edit-feed modal bound to one feed, optionally scrolled to a section. */
    openEditFeed(feedId: number, section: EditFeedSection = 'details'): void {
        this.editFeedId = feedId;
        this.editFeedSection = section;
        this.activeModal = 'edit-feed';
    }

    closeModal(): void {
        this.activeModal = null;
        this.editFeedId = null;
        this.editFeedSection = 'details';
    }

    /** Test isolation. */
    reset(): void {
        this.activeModal = null;
        this.editFeedId = null;
        this.editFeedSection = 'details';
        this.sidebarCollapsed = false;
        this.focusMode = false;
        this.narrow = false;
        this.sidebarPeek = false;
        this.paneMotion = false;
    }
}

function readWidth(
    key: (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS],
    limits: { min: number; max: number },
    fallback: number,
): number {
    const raw = settingsStore.get(key);
    if (raw === undefined) {
        return fallback;
    }
    const parsed = Number.parseInt(raw, 10);
    if (Number.isNaN(parsed)) {
        return fallback;
    }
    return Math.min(limits.max, Math.max(limits.min, parsed));
}

/** The app-wide singleton. */
export const layoutStore = new LayoutStore();
