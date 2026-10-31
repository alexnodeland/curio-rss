/**
 * `commandErrorMessage`: user-tier messages verbatim, internal failures
 * generic, and a busy profile (another Curio process holds it) as its own
 * localized line — never the engine's English message with a lock path.
 */
import { localeStore, t } from '$lib/i18n';
import { de } from '$lib/i18n/de';
import { commandErrorMessage } from '$lib/utils/errors';
import { afterEach, describe, expect, it } from 'vitest';
import { commandErrorFixture } from './ipc-harness';

describe('commandErrorMessage', () => {
    afterEach(() => {
        localeStore.reset();
    });

    it('surfaces a user-tier message verbatim', () => {
        const error = commandErrorFixture({ kind: 'user', message: 'feed already exists' });
        expect(commandErrorMessage(error)).toBe('feed already exists');
    });

    it('hides an internal failure behind the generic line', () => {
        const error = commandErrorFixture({
            kind: 'internal',
            code: 'STORAGE',
            message: 'sqlite error: disk I/O error',
        });
        expect(commandErrorMessage(error)).toBe(t('app.error.internal'));
    });

    it('shows a busy profile as localized copy, without the raw lock path', () => {
        const error = commandErrorFixture({
            kind: 'user',
            code: 'PROFILE_BUSY',
            recoverable: true,
            message:
                "another Curio process kept this profile's event log locked for 15000 ms (/Users/me/.curio/events.lock)",
        });
        const english = commandErrorMessage(error);
        expect(english).toBe(t('app.error.profileBusy'));
        expect(english).not.toContain('events.lock');

        localeStore.active = 'de';
        expect(commandErrorMessage(error)).toBe(de['app.error.profileBusy']);
        expect(commandErrorMessage(error)).not.toBe(english);
    });
});
