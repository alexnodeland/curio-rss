/**
 * Destinations + promote state. The registry list is backend-owned (cached
 * in the query cache); the chosen default promote target is a persisted UI
 * preference. The cardinal rule (D6, and the named-destination contract):
 * promote crosses IPC by destination NAME only — a raw filesystem path
 * never leaves this process as a free string. Adding a destination sends the
 * dialog-pick TOKEN (`pick_destination_root` → `PathTokenDto`), never a path.
 */
import {
    type CommandError,
    type DestinationDto,
    type PathTokenDto,
    type SaveOutcomeDto,
    commands,
} from '$lib/bindings';
import {
    type CommandResult,
    type Query,
    ensureQuery,
    invalidatePrefix,
    queryKeys,
} from './query-cache.svelte';
import { SETTING_KEYS, settingsStore } from './settings.svelte';

export class DestinationsStore {
    get #query(): Query<DestinationDto[]> {
        return ensureQuery(queryKeys.destinations, commands.listDestinations);
    }

    /** Primes the registry query outside a render reaction (see feeds.prime). */
    prime(): void {
        void this.#query;
    }

    get destinations(): DestinationDto[] {
        return this.#query.data ?? [];
    }

    get loading(): boolean {
        return this.#query.loading;
    }

    get loaded(): boolean {
        return this.#query.loaded;
    }

    get error(): CommandError | null {
        return this.#query.error;
    }

    /** The persisted default promote target name, if one is set. */
    get selectedName(): string | null {
        return settingsStore.get(SETTING_KEYS.promoteDestination) ?? null;
    }

    /**
     * The destination `p` promotes to without asking: the chosen default if
     * it still exists, else the sole destination when there is exactly one,
     * else `null` (the caller opens the panel to pick).
     */
    get promoteTarget(): string | null {
        const list = this.destinations;
        const chosen = this.selectedName;
        if (chosen !== null && list.some((destination) => destination.name === chosen)) {
            return chosen;
        }
        return list.length === 1 ? list[0].name : null;
    }

    /** Persists the default promote target. */
    setSelected(name: string): Promise<CommandResult<null>> {
        return settingsStore.set(SETTING_KEYS.promoteDestination, name);
    }

    /**
     * Registers a destination from a dialog-pick token (never a raw path)
     * and refreshes the registry list on success.
     */
    async add(name: string, pathToken: string): Promise<CommandResult<null>> {
        const result = await commands.addDestination(name, pathToken);
        if (result.status === 'ok') {
            invalidatePrefix(queryKeys.destinations);
        }
        return result;
    }

    /** Unregisters a destination name (exported notes are untouched). */
    async remove(name: string): Promise<CommandResult<null>> {
        const result = await commands.removeDestination(name);
        if (result.status === 'ok') {
            invalidatePrefix(queryKeys.destinations);
        }
        return result;
    }

    /** Opens the native folder picker; `null` = cancelled. */
    pickRoot(): Promise<CommandResult<PathTokenDto | null>> {
        return commands.pickDestinationRoot();
    }

    /** Promotes an article into a destination by NAME. */
    promote(articleId: number, name: string): Promise<CommandResult<SaveOutcomeDto>> {
        return commands.promoteArticle(articleId, name);
    }

    /** Test isolation — the registry lives in the query cache, reset there. */
    reset(): void {
        // no local mutable state beyond the persisted preference
    }
}

/** The app-wide singleton. */
export const destinationsStore = new DestinationsStore();
