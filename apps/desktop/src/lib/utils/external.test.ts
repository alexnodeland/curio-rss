/**
 * The opener scheme gate: `isOpenableUrl` must mirror the Tauri opener ACL
 * exactly (http/https for the browser, mailto/tel for the OS mail/dialer) and
 * fail closed on everything else, so a hostile feed href can't reach IPC.
 */
import { isOpenableUrl, isWebUrl } from '$lib/utils/external';
import { describe, expect, it } from 'vitest';

describe('isOpenableUrl', () => {
    it('accepts the allowed schemes (case- and whitespace-insensitive)', () => {
        expect(isOpenableUrl('https://example.com')).toBe(true);
        expect(isOpenableUrl('http://example.com')).toBe(true);
        expect(isOpenableUrl('HTTPS://EXAMPLE.COM')).toBe(true);
        expect(isOpenableUrl('  https://example.com  ')).toBe(true);
        expect(isOpenableUrl('mailto:hi@example.com')).toBe(true);
        expect(isOpenableUrl('MAILTO:hi@example.com?subject=Hi')).toBe(true);
        expect(isOpenableUrl('tel:+1-555-0100')).toBe(true);
    });

    it('rejects every other scheme (fail closed)', () => {
        expect(isOpenableUrl('javascript:alert(1)')).toBe(false);
        expect(isOpenableUrl('file:///etc/passwd')).toBe(false);
        expect(isOpenableUrl('data:text/html,<script>')).toBe(false);
        expect(isOpenableUrl('ftp://example.com')).toBe(false);
        expect(isOpenableUrl('//example.com')).toBe(false);
        expect(isOpenableUrl('/relative/path')).toBe(false);
        expect(isOpenableUrl('mailtoX:nope')).toBe(false);
        expect(isOpenableUrl('')).toBe(false);
    });
});

describe('isWebUrl', () => {
    it('accepts http(s) only (case- and whitespace-insensitive)', () => {
        expect(isWebUrl('https://example.com/a')).toBe(true);
        expect(isWebUrl('http://example.com')).toBe(true);
        expect(isWebUrl('  HTTPS://EXAMPLE.COM  ')).toBe(true);
    });

    it('rejects every other source, the opener-only schemes included', () => {
        for (const url of [
            'javascript:alert(1)',
            '  JaVaScRiPt:alert(2)',
            'httpsupdated://xss.example.com/one',
            'foo://h/dir/three',
            'ftp://files.example/pub/two',
            'data:text/html,x',
            'mailto:hi@example.com',
            'tel:+1-555-0100',
            '//example.com',
            '/relative/path',
            '',
        ]) {
            expect(isWebUrl(url), url).toBe(false);
        }
    });
});
