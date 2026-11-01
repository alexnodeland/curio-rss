/**
 * The toast queue: status messages with auto-dismiss timers, optional action
 * buttons (Undo / Retry), de-duplicated (a plain toast by message + tone, an
 * actionable one only by its caller's {@link ToastOptions.key}) and capped at
 * {@link MAX_TOASTS}. The `Toasts` component renders whatever this store
 * holds, plus two permanent live regions that announce each new toast — the
 * toast nodes themselves carry no live role, because a region that mounts
 * already filled is often not announced (VoiceOver).
 *
 * Timing (WCAG 2.2.1): a plain toast stays for a duration that scales with
 * its length; a toast with an action stays at least {@link ACTION_TOAST_MS};
 * an error stays until dismissed (one given an explicit duration still stays
 * at least {@link ACTION_TOAST_MS}). Hovering or focusing the stack holds
 * every timer, and the time left resumes when the pointer/focus leaves — or
 * when the toast holding focus closes, or the stack empties, since a node
 * that is removed need not fire `focusout` / `mouseleave` (WebKit).
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
 * the close button, {@link ToastStore.settleAll}, {@link ToastStore.withdraw},
 * or a newer toast under the same key. `timeout`: the timer ran out.
 * `evicted`: pushed off the capped stack by newer toasts.
 */
export type ToastCloseReason = 'action' | 'dismiss' | 'timeout' | 'evicted';

export interface ToastOptions {
    /**
     * Milliseconds on screen; 0 keeps it until dismissed. Default: see
     * {@link defaultToastDuration}. An error is never timed shorter than
     * {@link ACTION_TOAST_MS}.
     */
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
     * What the toast is about (e.g. `refresh:12`). A newer toast under the
     * same key replaces the showing one — merging into it (timer refreshed,
     * newest action taken) when the text and tone match too — and
     * {@link ToastStore.withdraw} closes it once it no longer applies.
     * Without a key, a plain toast merges into an identical one (message +
     * tone) and an actionable one never merges: its Undo / Retry belongs to
     * its own action. Ignored with `onClose`.
     */
    readonly key?: string;
}

export interface Toast {
    readonly id: number;
    readonly message: string;
    readonly tone: ToastTone;
    readonly action?: ToastAction;
}

export type Politeness = 'polite' | 'assertive';

/** What holds the timers: the pointer over the stack, or focus inside it. */
export type ToastHold = 'pointer' | 'focus';

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
/** The undo / retry window: a toast with an action, or a timed error, stays at least this long. */
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

/**
 * The time a toast actually gets: the caller's duration or the default —
 * except that a timed error never leaves before {@link ACTION_TOAST_MS},
 * whatever the caller asked for (there must be time to read it and act).
 */
function effectiveDuration(message: string, tone: ToastTone, opts: ToastOptions): number {
    const requested =
        opts.durationMs ?? defaultToastDuration(message, tone, opts.action !== undefined);
    return tone === 'error' && requested > 0 ? Math.max(requested, ACTION_TOAST_MS) : requested;
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

    #pointerHold = $state(false);
    #focusHold = $state(false);

    #nextToastId = 1;
    #nextAnnouncementId = 1;
    #timers = new SvelteMap<number, ToastTimer>();
    #closers = new SvelteMap<number, (reason: ToastCloseReason) => void>();
    /** Each keyed toast's key (see {@link ToastOptions.key}). */
    #keys = new SvelteMap<number, string>();

    /** Whether the pointer or focus is on the stack (every timer is held). */
    get paused(): boolean {
        return this.#pointerHold || this.#focusHold;
    }

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
        const durationMs = effectiveDuration(message, tone, opts);
        const merged = this.#mergeTarget(message, tone, opts);
        if (merged !== undefined) {
            // A repeat (the same failure again) refreshes the showing toast
            // instead of stacking a copy.
            this.#armTimer(merged, durationMs);
            return merged;
        }
        const id = this.#nextToastId;
        this.#nextToastId += 1;
        if (opts.onClose !== undefined) {
            this.#closers.set(id, opts.onClose);
        } else if (opts.key !== undefined) {
            this.#keys.set(id, opts.key);
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
     * The showing toast a new one merges into, if any. A toast with a close
     * hook never merges (each carries its own deferred commit); a keyed one
     * goes through {@link #replaceKeyed}; a plain one merges into an identical
     * plain toast; an unkeyed one with an action never merges.
     */
    #mergeTarget(message: string, tone: ToastTone, opts: ToastOptions): number | undefined {
        if (opts.onClose !== undefined) {
            return undefined;
        }
        if (opts.key !== undefined) {
            return this.#replaceKeyed(opts.key, message, tone, opts.action);
        }
        if (opts.action !== undefined) {
            return undefined;
        }
        return this.toasts.find(
            (toast) =>
                toast.message === message &&
                toast.tone === tone &&
                toast.action === undefined &&
                !this.#keys.has(toast.id) &&
                !this.#closers.has(toast.id),
        )?.id;
    }

    /**
     * The showing toast under `key`: when its text and tone match, it takes
     * the newest action and its id is returned (merged); otherwise it is
     * closed, so the newer toast takes its place.
     */
    #replaceKeyed(
        key: string,
        message: string,
        tone: ToastTone,
        action?: ToastAction,
    ): number | undefined {
        const index = this.toasts.findIndex((toast) => this.#keys.get(toast.id) === key);
        const existing = this.toasts[index];
        if (existing === undefined) {
            return undefined;
        }
        if (existing.message !== message || existing.tone !== tone) {
            this.#close(existing.id, 'dismiss');
            return undefined;
        }
        this.toasts[index] =
            action === undefined ? { id: existing.id, message, tone } : { ...existing, action };
        return existing.id;
    }

    /**
     * Closes the toast under `key`, if one is showing — e.g. a failure whose
     * action has since succeeded, so its Retry would replay a stale request
     * over the newer outcome.
     */
    withdraw(key: string): void {
        for (const toast of this.toasts.filter(
            (candidate) => this.#keys.get(candidate.id) === key,
        )) {
            this.#close(toast.id, 'dismiss');
        }
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

    /**
     * Closes a toast by hand (its close button). The button that had focus
     * leaves with the toast, so the focus hold ends here: a removed node need
     * not fire `focusout`, and the other toasts must not stay frozen.
     */
    dismissToast(id: number): void {
        this.#close(id, 'dismiss');
        this.release('focus');
    }

    /**
     * Runs a toast's action and closes it (`onClose` sees `action`); ends the
     * focus hold like {@link dismissToast}.
     */
    runAction(id: number): void {
        const toast = this.toasts.find((candidate) => candidate.id === id);
        if (toast?.action === undefined) {
            return;
        }
        const { action } = toast;
        this.#close(id, 'action');
        this.release('focus');
        void action.run();
    }

    /** Holds every timer: the pointer entered the stack, or focus did. */
    hold(source: ToastHold): void {
        const wasPaused = this.paused;
        if (source === 'pointer') {
            this.#pointerHold = true;
        } else {
            this.#focusHold = true;
        }
        if (wasPaused) {
            return;
        }
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

    /**
     * Ends one hold; once neither the pointer nor focus holds the stack,
     * every held timer restarts with the time it had left.
     */
    release(source: ToastHold): void {
        if (source === 'pointer') {
            this.#pointerHold = false;
        } else {
            this.#focusHold = false;
        }
        if (this.paused) {
            return;
        }
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
        this.#keys.delete(id);
        this.toasts = this.toasts.filter((toast) => toast.id !== id);
        if (this.toasts.length === 0) {
            // No stack left to hover or focus — and its last node going away
            // need not have fired `mouseleave` / `focusout`.
            this.#pointerHold = false;
            this.#focusHold = false;
        }
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
        this.#keys.clear();
        this.toasts = [];
        this.politeAnnouncements = [];
        this.assertiveAnnouncements = [];
        this.#pointerHold = false;
        this.#focusHold = false;
    }
}

/** The app-wide singleton. */
export const toastStore = new ToastStore();
