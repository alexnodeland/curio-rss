/**
 * The toast queue: status messages with auto-dismiss timers, optional action
 * buttons (Undo / Retry), de-duplicated by message + tone and capped at
 * {@link MAX_TOASTS}. The `Toasts` component renders whatever this store
 * holds, plus two permanent live regions that announce each new toast — the
 * toast nodes themselves carry no live role, because a region that mounts
 * already filled is often not announced (VoiceOver).
 *
 * Timing (WCAG 2.2.1): a plain toast stays for a duration that scales with
 * its length; a toast with an action stays at least {@link ACTION_TOAST_MS};
 * an error stays until dismissed. Hovering or focusing the stack pauses every
 * timer, and the time left resumes when the pointer/focus leaves.
 */
import { SvelteMap } from 'svelte/reactivity';

export type ToastTone = 'info' | 'success' | 'warning' | 'error';

/** A button on a toast — clicking it runs `run` and closes the toast. */
export interface ToastAction {
    readonly label: string;
    readonly run: () => void | Promise<void>;
}

/**
 * Why a toast left the screen. `action`: its button was clicked. `dismiss`:
 * the close button, or {@link ToastStore.settleAll}. `timeout`: the timer ran
 * out. `evicted`: pushed off the capped stack by newer toasts.
 */
export type ToastCloseReason = 'action' | 'dismiss' | 'timeout' | 'evicted';

export interface ToastOptions {
    /** Milliseconds on screen; 0 keeps it until dismissed. Default: see {@link defaultToastDuration}. */
    readonly durationMs?: number;
    /** A button (Undo, Retry…). */
    readonly action?: ToastAction;
    /**
     * Called once when the toast closes, with the reason — how a deferred
     * commit (an undo window) learns it should go ahead. Not called by
     * {@link ToastStore.reset}.
     */
    readonly onClose?: (reason: ToastCloseReason) => void;
    /**
     * Merge into an identical toast already showing (same message, tone and
     * action label) instead of stacking a copy. Default: true for a plain
     * toast, false for one with an action. Ignored with `onClose`.
     */
    readonly dedupe?: boolean;
}

export interface Toast {
    readonly id: number;
    readonly message: string;
    readonly tone: ToastTone;
    readonly action?: ToastAction;
}

export type Politeness = 'polite' | 'assertive';

/** One line written into a live region (a fresh node per announcement). */
export interface Announcement {
    readonly id: number;
    readonly text: string;
    /** The toast this line announced, if any (it leaves with the toast). */
    readonly toastId?: number;
}

/** The shortest a plain toast stays on screen. */
export const MIN_TOAST_DURATION_MS = 4000;
/** The longest a plain toast stays on screen, however long its text. */
export const MAX_TOAST_DURATION_MS = 10_000;
/** Extra reading time per character, on top of the minimum. */
const PER_CHAR_MS = 50;
/** The undo / retry window: a toast with an action stays at least this long. */
export const ACTION_TOAST_MS = 8000;

/** The most toasts kept on screen at once; older ones drop so the stack can't
 *  overflow off the viewport (e.g. a failing sweep firing one per feed). */
export const MAX_TOASTS = 4;

/** Announcement nodes kept per live region (older ones are pruned). */
const MAX_ANNOUNCEMENTS = 3;

/**
 * How long a toast stays when the caller names no duration: errors are
 * sticky (they stay until dismissed), others scale with the message length,
 * and a toast with an action never closes before its window is up.
 */
export function defaultToastDuration(message: string, tone: ToastTone, hasAction: boolean): number {
    if (tone === 'error') {
        return 0;
    }
    const scaled = Math.min(
        MAX_TOAST_DURATION_MS,
        MIN_TOAST_DURATION_MS + message.length * PER_CHAR_MS,
    );
    return hasAction ? Math.max(ACTION_TOAST_MS, scaled) : scaled;
}

interface ToastTimer {
    /** Time left on the clock (updated whenever the timer pauses). */
    remaining: number;
    /** When the running stretch started (`Date.now()`), if running. */
    startedAt: number | null;
    handle: ReturnType<typeof setTimeout> | null;
}

export class ToastStore {
    toasts: Toast[] = $state([]);

    /** The polite live region's lines (non-error toasts). */
    politeAnnouncements: Announcement[] = $state([]);
    /** The assertive live region's lines (error toasts). */
    assertiveAnnouncements: Announcement[] = $state([]);

    /** Whether the pointer or focus is on the stack (every timer is held). */
    paused: boolean = $state(false);

    #nextToastId = 1;
    #nextAnnouncementId = 1;
    #timers = new SvelteMap<number, ToastTimer>();
    #closers = new SvelteMap<number, (reason: ToastCloseReason) => void>();

    /**
     * Queues a toast and returns its id. The third argument is either a
     * duration in ms (0 = sticky; the legacy form) or {@link ToastOptions}.
     */
    showToast(
        message: string,
        tone: ToastTone = 'info',
        options: number | ToastOptions = {},
    ): number {
        const opts: ToastOptions = typeof options === 'number' ? { durationMs: options } : options;
        const durationMs =
            opts.durationMs ?? defaultToastDuration(message, tone, opts.action !== undefined);
        // Dedup: an identical message+tone (with the same kind of button)
        // already showing just refreshes its timer — and takes the newest
        // action — so a repeated failure doesn't stack N copies. A toast with
        // a close hook is never merged (each carries its own deferred commit),
        // nor one with an action unless the caller opts in (an Undo belongs to
        // its own snapshot; a Retry is interchangeable).
        const dedupe = opts.onClose === undefined && (opts.dedupe ?? opts.action === undefined);
        const merged = dedupe ? this.#mergeDuplicate(message, tone, opts.action) : undefined;
        if (merged !== undefined) {
            this.#armTimer(merged, durationMs);
            return merged;
        }
        const id = this.#nextToastId;
        this.#nextToastId += 1;
        if (opts.onClose !== undefined) {
            this.#closers.set(id, opts.onClose);
        }
        this.#push(
            opts.action === undefined
                ? { id, message, tone }
                : { id, message, tone, action: opts.action },
        );
        this.#armTimer(id, durationMs);
        this.announce(message, tone === 'error' ? 'assertive' : 'polite', id);
        return id;
    }

    /**
     * Finds a showing toast with the same message, tone and action label (and
     * no close hook), hands it the newest action, and returns its id.
     */
    #mergeDuplicate(message: string, tone: ToastTone, action?: ToastAction): number | undefined {
        const index = this.toasts.findIndex(
            (toast) =>
                toast.message === message &&
                toast.tone === tone &&
                toast.action?.label === action?.label &&
                !this.#closers.has(toast.id),
        );
        const existing = this.toasts[index];
        if (existing === undefined) {
            return undefined;
        }
        if (action !== undefined) {
            this.toasts[index] = { ...existing, action };
        }
        return existing.id;
    }

    /** Appends a toast, capping the stack: the oldest drop off as `evicted`. */
    #push(toast: Toast): void {
        const next = [...this.toasts, toast];
        const overflow = Math.max(0, next.length - MAX_TOASTS);
        this.toasts = next.slice(overflow);
        for (const old of next.slice(0, overflow)) {
            this.#close(old.id, 'evicted');
        }
    }

    /**
     * Writes a line into one of the two permanent live regions. Each
     * announcement is a new node, so a repeat of the same text is still read.
     * A toast's line leaves the region with the toast (a removal is not
     * announced), so nothing stale lingers for a screen-reader user to find.
     */
    announce(text: string, politeness: Politeness = 'polite', toastId?: number): void {
        const entry: Announcement = { id: this.#nextAnnouncementId, text, toastId };
        this.#nextAnnouncementId += 1;
        if (politeness === 'assertive') {
            this.assertiveAnnouncements = [...this.assertiveAnnouncements, entry].slice(
                -MAX_ANNOUNCEMENTS,
            );
        } else {
            this.politeAnnouncements = [...this.politeAnnouncements, entry].slice(
                -MAX_ANNOUNCEMENTS,
            );
        }
    }

    /** Closes a toast by hand (the close button). */
    dismissToast(id: number): void {
        this.#close(id, 'dismiss');
    }

    /** Runs a toast's action and closes it (`onClose` sees `action`). */
    runAction(id: number): void {
        const toast = this.toasts.find((candidate) => candidate.id === id);
        if (toast?.action === undefined) {
            return;
        }
        const { action } = toast;
        this.#close(id, 'action');
        void action.run();
    }

    /** Holds every timer (pointer or focus entered the stack). */
    pause(): void {
        if (this.paused) {
            return;
        }
        this.paused = true;
        const now = Date.now();
        for (const timer of this.#timers.values()) {
            if (timer.handle !== null && timer.startedAt !== null) {
                clearTimeout(timer.handle);
                timer.remaining = Math.max(0, timer.remaining - (now - timer.startedAt));
                timer.handle = null;
                timer.startedAt = null;
            }
        }
    }

    /** Restarts every held timer with the time it had left. */
    resume(): void {
        if (!this.paused) {
            return;
        }
        this.paused = false;
        for (const id of [...this.#timers.keys()]) {
            this.#startTimer(id);
        }
    }

    /**
     * Closes every toast as dismissed, so pending deferred commits (an
     * unsubscribe still in its undo window) go ahead — called when the window
     * is about to unload.
     */
    settleAll(): void {
        for (const toast of [...this.toasts]) {
            this.#close(toast.id, 'dismiss');
        }
    }

    /** (Re)arms the auto-dismiss timer for a toast; duration 0 means sticky. */
    #armTimer(id: number, durationMs: number): void {
        this.#clearTimer(id);
        if (durationMs > 0) {
            this.#timers.set(id, { remaining: durationMs, startedAt: null, handle: null });
            this.#startTimer(id);
        }
    }

    #startTimer(id: number): void {
        const timer = this.#timers.get(id);
        if (timer === undefined || this.paused || timer.handle !== null) {
            return;
        }
        timer.startedAt = Date.now();
        timer.handle = setTimeout(() => this.#close(id, 'timeout'), timer.remaining);
    }

    #clearTimer(id: number): void {
        const timer = this.#timers.get(id);
        if (timer !== undefined) {
            if (timer.handle !== null) {
                clearTimeout(timer.handle);
            }
            this.#timers.delete(id);
        }
    }

    #close(id: number, reason: ToastCloseReason): void {
        this.#clearTimer(id);
        this.toasts = this.toasts.filter((toast) => toast.id !== id);
        const keep = (line: Announcement): boolean => line.toastId !== id;
        this.politeAnnouncements = this.politeAnnouncements.filter(keep);
        this.assertiveAnnouncements = this.assertiveAnnouncements.filter(keep);
        const onClose = this.#closers.get(id);
        if (onClose !== undefined) {
            this.#closers.delete(id);
            onClose(reason);
        }
    }

    /** Test isolation (close hooks are dropped, not called). */
    reset(): void {
        for (const timer of this.#timers.values()) {
            if (timer.handle !== null) {
                clearTimeout(timer.handle);
            }
        }
        this.#timers.clear();
        this.#closers.clear();
        this.toasts = [];
        this.politeAnnouncements = [];
        this.assertiveAnnouncements = [];
        this.paused = false;
    }
}

/** The app-wide singleton. */
export const toastStore = new ToastStore();
