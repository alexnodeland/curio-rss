/**
 * OPML import/export over the PathToken flow: the native dialog is a Rust
 * command that returns an opaque token, and only that token crosses back to
 * the IO command — a raw filesystem path never appears in any invoke
 * payload. Cancellation is a no-op with a toast.
 */
import OpmlPanel from '$components/modals/OpmlPanel.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    flushIpc,
    installIpcHarness,
    opmlImportOutcomeFixture,
    pathTokenFixture,
} from './ipc-harness';

const PICKED_PATH = '/home/user/Downloads/subs.opml';

describe('OpmlPanel', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('imports through the token, never a raw path', async () => {
        harness = installIpcHarness({
            pick_import_file: pathTokenFixture({ token: 'TOKEN-IN', path: PICKED_PATH }),
            import_opml: opmlImportOutcomeFixture({ added: 2, skipped: 1 }),
        });
        const { getByText } = render(OpmlPanel);

        await fireEvent.click(getByText('Import OPML…'));
        await flushIpc();

        expect(harness.callsFor('import_opml')).toEqual([{ pathToken: 'TOKEN-IN' }]);
        expect(uiStore.toasts.at(-1)?.message).toBe('Imported 2 feeds, skipped 1');
        for (const call of harness.calls) {
            expect(JSON.stringify(call.args)).not.toContain(PICKED_PATH);
        }
    });

    it('exports through the token', async () => {
        harness = installIpcHarness({
            pick_export_path: pathTokenFixture({ token: 'TOKEN-OUT', path: PICKED_PATH }),
            export_opml: null,
        });
        const { getByText } = render(OpmlPanel);

        await fireEvent.click(getByText('Export OPML…'));
        await flushIpc();

        expect(harness.callsFor('export_opml')).toEqual([{ pathToken: 'TOKEN-OUT' }]);
        expect(uiStore.toasts.at(-1)?.message).toBe('Exported your subscriptions');
    });

    it('treats a cancelled pick as a no-op', async () => {
        harness = installIpcHarness({ pick_import_file: null });
        const { getByText } = render(OpmlPanel);

        await fireEvent.click(getByText('Import OPML…'));
        await flushIpc();

        expect(harness.callsFor('import_opml')).toHaveLength(0);
        expect(uiStore.toasts.at(-1)?.message).toBe('Cancelled');
    });
});
