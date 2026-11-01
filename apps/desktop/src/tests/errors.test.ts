/**
 * `commandErrorMessage`: a user-tier failure in the active locale (its code's
 * copy, filled from its params; the broad codes keep the engine's specific
 * English message in English), internal failures generic, and a busy profile
 * as its own localized line — never the engine's English with a lock path.
 */
import { localeStore, t } from '$lib/i18n';
import { de } from '$lib/i18n/de';
import { en } from '$lib/i18n/en';
import { es } from '$lib/i18n/es';
import { fr } from '$lib/i18n/fr';
import { it as itCatalog } from '$lib/i18n/it';
import { pl } from '$lib/i18n/pl';
import { yue } from '$lib/i18n/yue';
import { zhHans } from '$lib/i18n/zh-Hans';
import { ERROR_COPY, commandErrorMessage } from '$lib/utils/errors';
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

    it('localizes a fine code through i18n, filling its params', () => {
        const error = commandErrorFixture({
            kind: 'user',
            code: 'NETWORK_TIMEOUT',
            params: { host: 'news.example.org' },
            message: 'read timeout fetching "https://news.example.org/feed.xml"',
            recoverable: true,
        });
        expect(commandErrorMessage(error)).toBe(
            'news.example.org took too long to answer. Try again in a moment.',
        );
        localeStore.active = 'de';
        expect(commandErrorMessage(error)).toBe(
            'news.example.org hat zu lange nicht geantwortet. Bitte gleich erneut versuchen.',
        );
    });

    it('fills every param a code carries', () => {
        const error = commandErrorFixture({
            code: 'HTTP_STATUS',
            params: { host: 'example.org', status: '503' },
            message: 'https://example.org/ answered with HTTP 503',
        });
        expect(commandErrorMessage(error)).toBe('example.org answered with an error (HTTP 503).');
        localeStore.active = 'zh-Hans';
        expect(commandErrorMessage(error)).toBe('example.org 返回了错误（HTTP 503）。');
    });

    it('keeps the specific English message for a broad code, and localizes it elsewhere', () => {
        const error = commandErrorFixture({
            code: 'INVALID_INPUT',
            message: 'invalid destination name: contains a path separator',
        });
        expect(commandErrorMessage(error)).toBe(error.message);
        localeStore.active = 'fr';
        expect(commandErrorMessage(error)).toBe(fr['error.invalidInput']);
    });

    it('never shows a raw placeholder when a param is missing', () => {
        const error = commandErrorFixture({
            code: 'HOST_NOT_FOUND',
            params: {},
            message: 'dns resolution failed for "nope.test": no such host',
        });
        expect(commandErrorMessage(error)).toBe(error.message);
        localeStore.active = 'de';
        expect(commandErrorMessage(error)).toBe(de['app.error.internal']);
    });

    it('keeps a param that happens to contain braces', () => {
        const error = commandErrorFixture({
            code: 'DESTINATION_MISSING',
            params: { root: '/vaults/{inbox}' },
            message: 'destination folder is missing: /vaults/{inbox}',
            recoverable: true,
        });
        localeStore.active = 'de';
        const text = commandErrorMessage(error);
        expect(text).toContain('/vaults/{inbox}');
        expect(text).not.toBe(de['app.error.internal']);
    });

    it('gives a Reddit auth refusal the guidance its reason names, in every locale', () => {
        const refusal = (reason: string) =>
            commandErrorFixture({
                code: 'REDDIT_AUTH',
                params: { reason, status: '403' },
                message: 'reddit blocks anonymous full-post requests (HTTP 403)',
            });
        const missing = refusal('credentials_missing');
        expect(commandErrorMessage(missing)).toBe(en['error.redditAuth.credentialsMissing']);
        localeStore.active = 'pl';
        expect(commandErrorMessage(missing)).toBe(pl['error.redditAuth.credentialsMissing']);
        expect(commandErrorMessage(missing)).not.toBe(pl['error.network']);
        expect(commandErrorMessage(refusal('token_revoked'))).toBe(
            pl['error.redditAuth.tokenRevoked'],
        );
        // A reason the frontend has no line for: the code's generic one.
        expect(commandErrorMessage(refusal('something_new'))).toBe(
            'Reddit odmówił wczytania pełnego posta (HTTP 403).',
        );
        const reasons = [
            'error.redditAuth.credentialsMissing',
            'error.redditAuth.credentialsRejected',
            'error.redditAuth.tokenRevoked',
            'error.redditAuth.tokenUnavailable',
        ] as const;
        for (const catalog of [en, es, fr, de, itCatalog, pl, zhHans, yue]) {
            for (const key of reasons) expect(catalog[key], key).toBeTruthy();
        }
    });

    it('has copy, in every locale, for every code', () => {
        const codes = Object.keys(ERROR_COPY) as (keyof typeof ERROR_COPY)[];
        expect(codes.length).toBeGreaterThanOrEqual(27);
        for (const code of codes) {
            const key = ERROR_COPY[code];
            for (const catalog of [en, es, fr, de, itCatalog, pl, zhHans, yue]) {
                expect(catalog[key], `${code} → ${key}`).toBeTruthy();
            }
        }
    });
});
