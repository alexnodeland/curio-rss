/**
 * Libraries from an earlier release: the startup notice (one toast per
 * outcome; warnings stay until dismissed) and the doctor panel's sentence for
 * every outcome the engine reports.
 */
import type { LegacyProfileStatusDto } from '$lib/bindings';
import { toastStore } from '$lib/state/toasts.svelte';
import {
    MOVED_NOTICE_MS,
    announceLegacyProfiles,
    legacyProfileDetail,
    legacyProfileNotice,
    loadLegacyProfiles,
} from '$lib/utils/legacy-profiles';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    legacyProfileFixture,
    rejectWith,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('legacy profiles', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        vi.restoreAllMocks();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('notices a moved library briefly and a left one until dismissed', async () => {
        const show = vi.spyOn(toastStore, 'showToast');
        harness = installIpcHarness({
            legacy_profiles: [
                legacyProfileFixture({ status: 'moved', default_dir: '/shared' }),
                legacyProfileFixture({ path: '/old', status: 'ambiguous' }),
                legacyProfileFixture({ path: '/mine', status: 'ambiguous', opened_in_place: true }),
            ],
        });
        await announceLegacyProfiles();
        await flushIpc();

        expect(show).toHaveBeenCalledTimes(3);
        expect(show).toHaveBeenNthCalledWith(
            1,
            'Moved your library from an earlier Curio release to /shared, where the command line opens it too.',
            'info',
            MOVED_NOTICE_MS,
        );
        expect(show).toHaveBeenNthCalledWith(
            2,
            'A library from an earlier Curio release was left at /old. See Settings › Advanced.',
            'warning',
            0,
        );
        expect(show.mock.calls[2]?.[0]).toContain('Curio opened it where it is');
        expect(show.mock.calls[2]?.[1]).toBe('warning');
    });

    it('says nothing when there was nothing to move', async () => {
        harness = installIpcHarness({ legacy_profiles: [] });
        await announceLegacyProfiles();
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('treats a failed or unavailable command as nothing to report', async () => {
        harness = installIpcHarness({ legacy_profiles: rejectWith(commandErrorFixture()) });
        expect(await loadLegacyProfiles()).toEqual([]);
        harness.teardown();
        // Unmocked: the harness throws, as an older backend would.
        harness = installIpcHarness({});
        expect(await loadLegacyProfiles()).toEqual([]);
    });

    it('explains every outcome, naming the paths and the error', () => {
        const expected: Record<LegacyProfileStatusDto, string> = {
            moved: 'Moved from /old to the location the app and the command line share.',
            default_in_use:
                '/old was left where it is: the shared location already holds a library',
            ambiguous: 'Move the one to keep to /shared by hand.',
            in_use: 'another Curio process has it open',
            default_not_empty: '/shared exists and is not empty',
            unreadable: 'its database could not be checked (boom)',
            move_failed: 'moving it failed (boom)',
        };
        for (const [status, text] of Object.entries(expected)) {
            const dto = legacyProfileFixture({
                path: '/old',
                default_dir: '/shared',
                status: status as LegacyProfileStatusDto,
                detail: 'boom',
            });
            expect(legacyProfileDetail(dto)).toContain(text);
            expect(legacyProfileDetail(dto)).not.toContain('{');
        }
    });

    it('adds that the library was opened where it is', () => {
        const dto = legacyProfileFixture({ status: 'in_use', opened_in_place: true });
        expect(legacyProfileDetail(dto)).toMatch(
            /restart Curio to move it\. Curio opened it where/,
        );
        expect(legacyProfileNotice(dto).message).toContain('Curio opened it where it is');
    });
});
