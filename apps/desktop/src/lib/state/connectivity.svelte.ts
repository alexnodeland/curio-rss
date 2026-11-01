/**
 * Connectivity state for the sidebar's "Offline" badge (#123). The Rust
 * refresh scheduler probes the network before every background sweep and,
 * when it is gone, skips the sweep (no fetches, no feed-health errors, no
 * "feeds failed" notification) and emits `ConnectivityChanged`; it re-probes
 * every minute until the network is back, then emits again. This store mirrors
 * that verdict: the event drives it, and `connectivity_online` seeds it at
 * startup (and after a webview reload) so a badge raised earlier isn't lost.
 */
import { events, commands } from '$lib/bindings';

export class ConnectivityStore {
    /** True while the scheduler's last probe found the network unreachable. */
    offline: boolean = $state(false);

    /** Bumped by every event, so a slower seed answer never overwrites one. */
    #generation = 0;

    /**
     * Subscribes to `ConnectivityChanged`, then seeds the current verdict.
     * Returns an unsubscribe for teardown.
     */
    async wire(): Promise<() => void> {
        const unlisten = await events.connectivityChanged.listen((event) => {
            this.#generation += 1;
            this.offline = !event.payload.online;
        });
        const seededAt = this.#generation;
        const result = await commands.connectivityOnline();
        if (result.status === 'ok' && seededAt === this.#generation) {
            this.offline = !result.data;
        }
        return unlisten;
    }

    /** Test isolation. */
    reset(): void {
        this.offline = false;
        this.#generation = 0;
    }
}

/** The app-wide singleton. */
export const connectivityStore = new ConnectivityStore();
