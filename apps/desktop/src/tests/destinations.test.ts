/**
 * The destinations store: the backend-owned registry list, the promote-target
 * resolution (chosen default / sole / ambiguous), and add/remove refreshing
 * the list. The panel's token flow is covered in promote-flow.test.ts.
 */
import DestinationsManager from '$components/modals/DestinationsManager.svelte';
import type { DestinationDto } from '$lib/bindings';
import { localeStore } from '$lib/i18n';
import { fr } from '$lib/i18n/fr';
import { destinationsStore } from '$lib/state/destinations.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    destinationFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';

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

describe('destinations manager', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        localeStore.reset();
        resetQueryCache();
        destinationsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('shows a failed registry load in the active locale, never the engine text', async () => {
        const engine = 'destination registry is unreadable: expected `=` at line 1';
        harness = installIpcHarness({
            list_destinations: rejectWith(
                commandErrorFixture({ kind: 'internal', code: 'IO', message: engine }),
            ),
        });
        localeStore.active = 'fr';
        render(DestinationsManager);
        await flushIpc();
        const alert = document.querySelector('[role="alert"]');
        expect(alert?.textContent).toBe(fr['app.error.internal']);
        expect(alert?.textContent).not.toContain('registry');
    });

    it('shows a busy profile as its localized line, without the lock path', async () => {
        harness = installIpcHarness({
            list_destinations: rejectWith(
                commandErrorFixture({
                    kind: 'user',
                    code: 'PROFILE_BUSY',
                    recoverable: true,
                    message: 'another Curio process holds /Users/me/.curio/events.lock',
                }),
            ),
        });
        localeStore.active = 'fr';
        render(DestinationsManager);
        await flushIpc();
        const alert = document.querySelector('[role="alert"]');
        expect(alert?.textContent).toBe(fr['app.error.profileBusy']);
        expect(alert?.textContent).not.toContain('events.lock');
    });
});
