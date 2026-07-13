/**
 * The destinations store: the backend-owned registry list, the promote-target
 * resolution (chosen default / sole / ambiguous), and add/remove refreshing
 * the list. The panel's token flow is covered in promote-flow.test.ts.
 */
import type { DestinationDto } from '$lib/bindings';
import { destinationsStore } from '$lib/state/destinations.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, destinationFixture, flushIpc, installIpcHarness } from './ipc-harness';

function dest(name: string): DestinationDto {
    return destinationFixture({ name, root: `/roots/${name}` });
}

describe('destinations store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        settingsStore.reset();
        destinationsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('lists the backend registry', async () => {
        harness = installIpcHarness({ list_destinations: [dest('notes'), dest('vault')] });
        destinationsStore.prime();
        await flushIpc();
        expect(destinationsStore.destinations.map((entry) => entry.name)).toEqual([
            'notes',
            'vault',
        ]);
    });

    it('promoteTarget prefers an existing chosen default', async () => {
        harness = installIpcHarness({
            list_destinations: [dest('notes'), dest('vault')],
            set_setting: null,
        });
        destinationsStore.prime();
        await flushIpc();
        await destinationsStore.setSelected('vault');
        expect(destinationsStore.promoteTarget).toBe('vault');
    });

    it('is ambiguous (null) when the chosen default no longer exists', async () => {
        harness = installIpcHarness({
            list_destinations: [dest('notes'), dest('vault')],
            set_setting: null,
        });
        destinationsStore.prime();
        await flushIpc();
        await destinationsStore.setSelected('gone');
        expect(destinationsStore.promoteTarget).toBeNull();
    });

    it('uses the sole destination when there is exactly one', async () => {
        harness = installIpcHarness({ list_destinations: [dest('notes')] });
        destinationsStore.prime();
        await flushIpc();
        expect(destinationsStore.promoteTarget).toBe('notes');
    });

    it('remove calls remove_destination and refreshes the list', async () => {
        let registry: DestinationDto[] = [dest('notes')];
        harness = installIpcHarness({
            list_destinations: () => registry,
            remove_destination: () => {
                registry = [];
                return null;
            },
        });
        destinationsStore.prime();
        await flushIpc();
        await destinationsStore.remove('notes');
        await flushIpc();
        expect(harness.callsFor('remove_destination')).toEqual([{ name: 'notes' }]);
        expect(destinationsStore.destinations).toEqual([]);
    });

    it('exportAll bulk-promotes the whole library into a destination', async () => {
        harness = installIpcHarness({
            promote_all: { created: 4, updated: 1, unchanged: 7 },
        });
        const result = await destinationsStore.exportAll('vault');
        expect(result.status).toBe('ok');
        if (result.status === 'ok') {
            expect(result.data.created).toBe(4);
        }
        const call = harness.callsFor('promote_all')[0] as {
            destination: string;
            filter: { read_later: null };
        };
        expect(call.destination).toBe('vault');
        expect(call.filter.read_later).toBeNull();
    });
});
