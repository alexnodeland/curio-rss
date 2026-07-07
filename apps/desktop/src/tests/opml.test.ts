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
    importOutcomeFixture,
    installIpcHarness,
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
            import_file: importOutcomeFixture({
                feeds_added: 2,
                feeds_skipped: 1,
                articles_added: 0,
                articles_skipped: 0,
            }),
        });
        const { getByText } = render(OpmlPanel);

        // The default source is OPML; import routes through the generic
        // import_file command carrying that source.
        await fireEvent.click(getByText('Import…'));
        await flushIpc();

        expect(harness.callsFor('import_file')).toEqual([
            { pathToken: 'TOKEN-IN', source: 'opml' },
        ]);
        expect(uiStore.toasts.at(-1)?.message).toBe('Imported 2 feeds and 0 articles, skipped 1');
        for (const call of harness.calls) {
            expect(JSON.stringify(call.args)).not.toContain(PICKED_PATH);
        }
    });

    it('sends the selected source to the importer', async () => {
        harness = installIpcHarness({
            pick_import_file: pathTokenFixture({ token: 'TOKEN-CSV', path: PICKED_PATH }),
            import_file: importOutcomeFixture({
                feeds_added: 0,
                feeds_skipped: 0,
                articles_added: 7,
                articles_skipped: 0,
            }),
        });
        const { getByText, getByRole } = render(OpmlPanel);

        await fireEvent.change(getByRole('combobox'), { target: { value: 'pocket_csv' } });
        await fireEvent.click(getByText('Import…'));
        await flushIpc();

        expect(harness.callsFor('import_file')).toEqual([
            { pathToken: 'TOKEN-CSV', source: 'pocket_csv' },
        ]);
        expect(uiStore.toasts.at(-1)?.message).toBe('Imported 0 feeds and 7 articles, skipped 0');
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

        await fireEvent.click(getByText('Import…'));
        await flushIpc();

        expect(harness.callsFor('import_file')).toHaveLength(0);
        expect(uiStore.toasts.at(-1)?.message).toBe('Cancelled');
    });
});
