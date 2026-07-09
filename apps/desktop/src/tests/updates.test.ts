/**
 * The updater's one piece of pure logic — the download-progress fraction. The
 * plugin-calling paths (check/install/relaunch) run only in the packaged app,
 * so they aren't unit-tested here.
 */
import { updateFraction } from '$lib/utils/updates';
import { describe, expect, it } from 'vitest';

describe('updateFraction', () => {
    it('is the ratio of downloaded to total', () => {
        expect(updateFraction(0, 100)).toBe(0);
        expect(updateFraction(50, 100)).toBe(0.5);
        expect(updateFraction(100, 100)).toBe(1);
    });

    it('clamps to 1 if more bytes arrive than the reported total', () => {
        expect(updateFraction(150, 100)).toBe(1);
    });

    it('is 0 when the total length is unknown', () => {
        expect(updateFraction(50, 0)).toBe(0);
    });
});
