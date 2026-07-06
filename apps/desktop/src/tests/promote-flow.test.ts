/**
 * The promote flow — the named-destination contract, end to end. This is the
 * gate the WP4 spec calls out: a raw filesystem path must NEVER cross IPC as
 * an argument. A folder is chosen through the Rust dialog (returns an opaque
 * token), only the token is sent to `add_destination`, and promote crosses by
 * destination NAME. The test scans every recorded invoke payload and asserts
 * the picked root never appears in any of them.
 */
import DestinationsPanel from '$components/modals/DestinationsPanel.svelte';
import type { DestinationDto } from '$lib/bindings';
import { promoteSelected } from '$lib/state/actions';
import { destinationsStore } from '$lib/state/destinations.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, saveOutcomeFixture } from './ipc-harness';

const ROOT = '/home/user/Documents/vault';
const TOKEN = '01JZABCDEFGHJKMNPQRSTVWXYZ';

describe('promote flow', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        selectionStore.reset();
        uiStore.reset();
        settingsStore.reset();
        destinationsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('adds by token, promotes by name — no raw path in any invoke payload', async () => {
        let registry: DestinationDto[] = [];
        harness = installIpcHarness({
            list_destinations: () => registry,
            pick_destination_root: { token: TOKEN, path: ROOT },
            // The dialog token is what add must receive — a path here would be a bug.
            add_destination: (args) => {
                registry = [{ name: args.name as string, root: ROOT }];
                return null;
            },
            get_setting: null,
            set_setting: null,
            promote_article: saveOutcomeFixture({ destination: 'vault', disposition: 'created' }),
        });

        // 1. Add a destination through the panel's token flow.
        const { getByText, getByLabelText } = render(DestinationsPanel, { onclose: () => {} });
        await flushIpc();
        await fireEvent.click(getByText('Choose folder…'));
        await flushIpc();
        await fireEvent.input(getByLabelText('Name'), { target: { value: 'vault' } });
        await fireEvent.click(getByText('Add destination'));
        await flushIpc();

        // The add carried the opaque TOKEN, never the ROOT path.
        expect(harness.callsFor('add_destination')).toEqual([{ name: 'vault', pathToken: TOKEN }]);

        // 2. Promote the selected article — the sole destination is the target.
        selectionStore.selectedArticleId = 100;
        await promoteSelected();
        await flushIpc();

        const promotes = harness.callsFor('promote_article');
        expect(promotes).toEqual([{ articleId: 100, destination: 'vault' }]);
        // The promote currency is a NAME, not a path.
        const destination = promotes[0]?.destination as string;
        expect(destination).toBe('vault');
        expect(destination.includes('/')).toBe(false);

        // 3. The invariant: the picked root never crossed IPC as an argument.
        for (const call of harness.calls) {
            expect(JSON.stringify(call.args)).not.toContain(ROOT);
        }

        // A success toast names the destination.
        expect(uiStore.toasts.at(-1)?.message).toBe('Saved to vault');
        expect(uiStore.toasts.at(-1)?.tone).toBe('success');
    });

    it('with no destination set, p opens the panel instead of promoting', async () => {
        harness = installIpcHarness({ list_destinations: [], get_setting: null });
        destinationsStore.prime();
        await flushIpc();

        selectionStore.selectedArticleId = 100;
        await promoteSelected();

        expect(uiStore.activeModal).toBe('destinations');
        expect(harness.callsFor('promote_article')).toHaveLength(0);
    });

    it('does nothing with no article selected', async () => {
        harness = installIpcHarness({ list_destinations: [{ name: 'vault', root: ROOT }] });
        destinationsStore.prime();
        await flushIpc();

        await promoteSelected();
        expect(harness.callsFor('promote_article')).toHaveLength(0);
        expect(uiStore.activeModal).toBeNull();
    });
});
