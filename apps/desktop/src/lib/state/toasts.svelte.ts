/**
 * The toast queue: transient status messages with auto-dismiss timers,
 * de-duplicated by message + tone and capped at {@link MAX_TOASTS}. The
 * `Toasts` component renders whatever this store holds.
 */
import { SvelteMap } from 'svelte/reactivity';

export type ToastTone = 'info' | 'success' | 'warning' | 'error';

export interface Toast {
    readonly id: number;
    readonly message: string;
    readonly tone: ToastTone;
}

export const DEFAULT_TOAST_DURATION_MS = 3000;

/** The most toasts kept on screen at once; older ones drop so the stack can't
 *  overflow off the viewport (e.g. a failing sweep firing one per feed). */
export const MAX_TOASTS = 4;

export class ToastStore {
    toasts: Toast[] = $state([]);

    #nextToastId = 1;
    #toastTimers = new SvelteMap<number, ReturnType<typeof setTimeout>>();

    /** Queues a toast; duration 0 keeps it until dismissed. */
    showToast(
        message: string,
        tone: ToastTone = 'info',
        durationMs: number = DEFAULT_TOAST_DURATION_MS,
    ): number {
        // Dedup: an identical message+tone already showing just refreshes its
        // timer, so a repeated failure doesn't stack N copies.
        const existing = this.toasts.find(
            (toast) => toast.message === message && toast.tone === tone,
        );
        if (existing !== undefined) {
            this.#armToastTimer(existing.id, durationMs);
            return existing.id;
        }
        const id = this.#nextToastId;
        this.#nextToastId += 1;
        let next = [...this.toasts, { id, message, tone }];
        // Cap the stack so it can't overflow off-screen; drop the oldest.
        if (next.length > MAX_TOASTS) {
            for (const toast of next.slice(0, next.length - MAX_TOASTS)) {
                this.#clearToastTimer(toast.id);
            }
            next = next.slice(next.length - MAX_TOASTS);
        }
        this.toasts = next;
        this.#armToastTimer(id, durationMs);
        return id;
    }

    /** (Re)arms the auto-dismiss timer for a toast; duration 0 means sticky. */
    #armToastTimer(id: number, durationMs: number): void {
        this.#clearToastTimer(id);
        if (durationMs > 0) {
            this.#toastTimers.set(
                id,
                setTimeout(() => this.dismissToast(id), durationMs),
            );
        }
    }

    #clearToastTimer(id: number): void {
        const timer = this.#toastTimers.get(id);
        if (timer !== undefined) {
            clearTimeout(timer);
            this.#toastTimers.delete(id);
        }
    }

    dismissToast(id: number): void {
        this.#clearToastTimer(id);
        this.toasts = this.toasts.filter((toast) => toast.id !== id);
    }

    /** Test isolation. */
    reset(): void {
        for (const timer of this.#toastTimers.values()) {
            clearTimeout(timer);
        }
        this.#toastTimers.clear();
        this.toasts = [];
    }
}

/** The app-wide singleton. */
export const toastStore = new ToastStore();
