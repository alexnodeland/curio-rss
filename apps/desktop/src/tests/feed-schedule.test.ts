import { feedScheduleNote } from '$lib/utils/feed-schedule';
import { describe, expect, it } from 'vitest';
import { feedFixture } from './ipc-harness';

const NOW = new Date('2026-11-01T12:00:00.000Z');
const LATER = '2026-11-01T12:30:00.000Z';
const EARLIER = '2026-11-01T11:30:00.000Z';

describe('feedScheduleNote', () => {
    it('says nothing about a healthy feed due on the next sweep', () => {
        expect(feedScheduleNote(feedFixture(), NOW)).toBeNull();
    });

    it('shows a failing feed backing off until its next attempt', () => {
        const note = feedScheduleNote(
            feedFixture({ consecutive_failures: 3, next_attempt_at: LATER }),
            NOW,
        );
        expect(note).toEqual({ kind: 'backoff', failures: 3, nextAttempt: new Date(LATER) });
    });

    it('shows when a healthy feed with its own refresh hint is next fetched', () => {
        const note = feedScheduleNote(feedFixture({ next_attempt_at: LATER }), NOW);
        expect(note).toEqual({ kind: 'scheduled', nextAttempt: new Date(LATER) });
    });

    it('drops a next attempt already in the past (or unreadable)', () => {
        expect(
            feedScheduleNote(
                feedFixture({ consecutive_failures: 1, next_attempt_at: EARLIER }),
                NOW,
            ),
        ).toBeNull();
        expect(feedScheduleNote(feedFixture({ next_attempt_at: 'soon' }), NOW)).toBeNull();
    });

    it('explains an automatic pause but not a user pause', () => {
        expect(
            feedScheduleNote(
                feedFixture({
                    status: 'paused',
                    consecutive_failures: 10,
                    auto_paused_at: EARLIER,
                    next_attempt_at: LATER,
                }),
                NOW,
            ),
        ).toEqual({ kind: 'autoPaused', failures: 10 });
        expect(
            feedScheduleNote(feedFixture({ status: 'paused', next_attempt_at: LATER }), NOW),
        ).toBeNull();
    });

    it('says nothing about a dead feed', () => {
        expect(
            feedScheduleNote(
                feedFixture({ status: 'dead', consecutive_failures: 2, next_attempt_at: LATER }),
                NOW,
            ),
        ).toBeNull();
    });
});
