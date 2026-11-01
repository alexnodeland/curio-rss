/**
 * What the edit-feed Health section says about a feed's refresh schedule,
 * derived from the backend-owned schedule fields on the DTO (the engine
 * computes the backoff; the head only renders it).
 */
import type { FeedDto } from '$lib/bindings';

export type FeedScheduleNote =
    /** Curio paused the feed by itself after a long run of failures. */
    | { kind: 'autoPaused'; failures: number }
    /** The feed is failing and waits before its next attempt. */
    | { kind: 'backoff'; failures: number; nextAttempt: Date }
    /** Healthy, but its own refresh hint or interval spaces its sweeps. */
    | { kind: 'scheduled'; nextAttempt: Date };

/**
 * The schedule note for `feed` at `now`, or `null` when there is nothing to
 * say: an active feed due on the next sweep, a user-paused one, a dead one.
 */
export function feedScheduleNote(feed: FeedDto, now: Date = new Date()): FeedScheduleNote | null {
    if (feed.status === 'paused') {
        // No failure run left (the engine ends an automatic pause on the
        // next success, but a stale row may linger): nothing to explain.
        return feed.auto_paused_at === null || feed.consecutive_failures === 0
            ? null
            : { kind: 'autoPaused', failures: feed.consecutive_failures };
    }
    if (feed.status !== 'active' || feed.next_attempt_at === null) {
        return null;
    }
    const nextAttempt = new Date(feed.next_attempt_at);
    if (Number.isNaN(nextAttempt.getTime()) || nextAttempt.getTime() <= now.getTime()) {
        return null;
    }
    return feed.consecutive_failures > 0
        ? { kind: 'backoff', failures: feed.consecutive_failures, nextAttempt }
        : { kind: 'scheduled', nextAttempt };
}
