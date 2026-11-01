/**
 * The doctor panel runs the health/maintenance commands (integrity checks,
 * vacuum, retention sweep, diagnostics bundle) and shows the db/app facts.
 * Each button is a thin command call whose outcome becomes a toast.
 */
import DoctorPanel from '$components/modals/DoctorPanel.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    appInfoFixture,
    dbStatsFixture,
    flushIpc,
    installIpcHarness,
    legacyProfileFixture,
    pathTokenFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

function harnessResponders(overrides: Record<string, Responder> = {}): Record<string, Responder> {
    return {
        db_stats: dbStatsFixture({ article_count: 42 }),
        get_app_info: appInfoFixture(),
        integrity_check: null,
        fts_integrity_check: null,
        vacuum_db: null,
        sweep_event_retention: ['2026-03-01.jsonl', '2026-03-02.jsonl'],
        copy_diagnostics_bundle: pathTokenFixture({ path: '/cache/curio/diagnostics/bundle' }),
        legacy_profiles: [],
        ...overrides,
    };
}

describe('DoctorPanel', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('shows the database facts on mount', async () => {
        harness = installIpcHarness(harnessResponders());
        const { getByText } = render(DoctorPanel);
        await flushIpc();
        expect(getByText('42 articles')).toBeTruthy();
        expect(getByText('Version 0.1.0')).toBeTruthy();
    });

    it('shows no legacy section when startup met no old library', async () => {
        harness = installIpcHarness(harnessResponders());
        const { queryByText } = render(DoctorPanel);
        await flushIpc();
        expect(queryByText('Libraries from earlier releases')).toBeNull();
    });

    it('explains a library from an earlier release that was left behind', async () => {
        harness = installIpcHarness(
            harnessResponders({
                legacy_profiles: [
                    legacyProfileFixture({ path: '/old/curio', status: 'in_use' }),
                    legacyProfileFixture({ path: '/older/curio', status: 'moved' }),
                ],
            }),
        );
        const { getByText } = render(DoctorPanel);
        await flushIpc();

        expect(getByText('Libraries from earlier releases')).toBeTruthy();
        const left = getByText(/^\/old\/curio was left where it is: another Curio process/);
        expect(left.classList.contains('left')).toBe(true);
        const moved = getByText(
            'Moved from /older/curio to the location the app and the command line share.',
        );
        expect(moved.classList.contains('left')).toBe(false);
    });

    it('runs the integrity check and toasts success', async () => {
        harness = installIpcHarness(harnessResponders());
        const { getByText } = render(DoctorPanel);
        await flushIpc();

        await fireEvent.click(getByText('Check integrity'));
        await flushIpc();

        expect(harness.callsFor('integrity_check')).toHaveLength(1);
        expect(toastStore.toasts.at(-1)?.message).toBe('Integrity check passed');
    });

    it('sweeps event retention and reports the removed count', async () => {
        harness = installIpcHarness(harnessResponders());
        const { getByText } = render(DoctorPanel);
        await flushIpc();

        await fireEvent.click(getByText('Sweep old event logs'));
        await flushIpc();

        expect(toastStore.toasts.at(-1)?.message).toBe('Removed 2 old log files');
    });

    it('copies a diagnostics bundle and names the path', async () => {
        harness = installIpcHarness(harnessResponders());
        const { getByText } = render(DoctorPanel);
        await flushIpc();

        await fireEvent.click(getByText('Copy diagnostics…'));
        await flushIpc();

        expect(harness.callsFor('copy_diagnostics_bundle')).toHaveLength(1);
        expect(toastStore.toasts.at(-1)?.message).toContain('/cache/curio/diagnostics/bundle');
    });
});
