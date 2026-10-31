/**
 * Shell layout state: the three-pane widths and sidebar collapse, the
 * per-feed-type "home" list layout, and the single-open modal enum (plus the
 * edit-feed modal's binding). Pane state persists to the settings table; the
 * modal is session-only.
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

export class LayoutStore {
    sidebarCollapsed: boolean = $state(false);
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

    toggleSidebar(): void {
        this.sidebarCollapsed = !this.sidebarCollapsed;
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
