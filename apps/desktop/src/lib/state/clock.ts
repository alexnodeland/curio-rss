/**
 * The local calendar day, as a reactive clock. The age-scaled list dates
 * (`formatIntlListDate`: a time for today, "yesterday", "3 days ago", a date
 * past that) only change when the day turns, so a row that stays mounted
 * across midnight must re-render then — or one list ends up mixing today's
 * labels with yesterday's.
 *
 * Reading {@link DayClock.now} inside a reactive context (a `$derived`, a
 * template `{@const}`) subscribes to the day: while anything is subscribed a
 * once-a-minute check runs, and it notifies the readers only when the local
 * date has actually changed (so a list never re-renders on a plain tick).
 * Checking every minute, rather than one timer aimed at midnight, also
 * catches up after the machine sleeps through the turn (a suspended webview
 * delays its timers). Outside a reactive context the getter is just
 * `new Date()`.
 */
import { createSubscriber } from 'svelte/reactivity';

/** How often a subscribed clock checks whether the local day has turned. */
export const DAY_CHECK_INTERVAL_MS = 60_000;

/** The local calendar day of `date`, as a comparable key. */
function localDay(date: Date): string {
    return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export class DayClock {
    #subscribe = createSubscriber((notify) => {
        let day = localDay(new Date());
        const timer = setInterval(() => {
            const today = localDay(new Date());
            if (today !== day) {
                day = today;
                notify();
            }
        }, DAY_CHECK_INTERVAL_MS);
        return () => clearInterval(timer);
    });

    /** The current time; a reactive read re-runs when the local day turns. */
    get now(): Date {
        this.#subscribe();
        return new Date();
    }
}

/** The app's one day clock — every list layout reads its dates against it. */
export const dayClock = new DayClock();
