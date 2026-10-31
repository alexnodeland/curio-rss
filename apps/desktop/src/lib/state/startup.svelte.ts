/**
 * The startup gate on the frontend: asks the backend whether the profile
 * opened, and drives the recovery screen when it did not. The recovery
 * commands work without a core; the reader's commands do not, so the layout
 * renders the reader only once `check()` says the app started normally.
 */
import { type RecoveryOutcomeDto, type StartupFailureDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import type { CommandResult } from '$lib/state/query-cache.svelte';
import { copyText } from '$lib/utils/clipboard';
import { commandErrorMessage } from '$lib/utils/errors';
import { type RecoveryAction, failureReport } from '$lib/utils/recovery';
import { checkForUpdate, installAndRelaunch } from '$lib/utils/updates';

/** A one-line status under the recovery actions. */
export interface RecoveryNotice {
    kind: 'error' | 'info';
    text: string;
}

/** The actions that re-open the profile through a backend command. */
type ReopenAction = Exclude<RecoveryAction, 'update'>;

const RUNNERS: Record<ReopenAction, () => Promise<CommandResult<RecoveryOutcomeDto>>> = {
    retry: () => commands.retryStartup(),
    restore: () => commands.restoreStartupBackup(),
    fresh: () => commands.startFreshProfile(),
};

class StartupStore {
    /** Why startup failed; `null` when it did not (or before `check()`). */
    failure: StartupFailureDto | null = $state(null);
    /** Set once a recovery action opened the profile. */
    recovered: { setAside: string | null } | null = $state(null);
    /** The action in flight, if any. */
    busy: RecoveryAction | null = $state(null);
    notice: RecoveryNotice | null = $state(null);

    /**
     * Asks the backend how startup went. Returns `true` when the profile
     * failed to open (show the recovery screen). A failed or unanswered
     * check falls through to the reader, the pre-gate behaviour.
     */
    async check(): Promise<boolean> {
        try {
            const result = await commands.getStartupFailure();
            this.failure = result.status === 'ok' ? result.data : null;
        } catch {
            this.failure = null;
        }
        return this.failure !== null;
    }

    /** Runs a recovery action; on success the screen offers to open Curio. */
    async run(action: RecoveryAction): Promise<void> {
        if (this.busy !== null) {
            return;
        }
        this.busy = action;
        this.notice = null;
        try {
            if (action === 'update') {
                await this.#update();
            } else {
                await this.#reopen(action);
            }
        } finally {
            this.busy = null;
        }
    }

    /**
     * Looks for the newer Curio a too-new database needs, and installs it
     * (which relaunches the app). The updater plugin needs no core, so this
     * works on the recovery screen; outside the packaged app it throws.
     */
    async #update(): Promise<void> {
        try {
            const update = await checkForUpdate();
            if (update === null) {
                this.notice = { kind: 'info', text: t('recovery.update.current') };
                return;
            }
            await installAndRelaunch(update);
        } catch {
            this.notice = { kind: 'error', text: t('recovery.update.failed') };
        }
    }

    async #reopen(action: ReopenAction): Promise<void> {
        try {
            const result = await RUNNERS[action]();
            if (result.status === 'error') {
                this.notice = { kind: 'error', text: commandErrorMessage(result.error) };
                return;
            }
            const { failure, set_aside: setAside } = result.data;
            if (failure === null) {
                this.recovered = { setAside };
                return;
            }
            this.failure = failure;
            const moved =
                setAside === null ? '' : ` ${t('recovery.recovered.setAside', { path: setAside })}`;
            this.notice = { kind: 'error', text: `${t('recovery.stillFailing')}${moved}` };
        } catch {
            this.notice = { kind: 'error', text: t('app.error.internal') };
        }
    }

    /** Copies the plain-text failure report for a bug report. */
    async copyReport(): Promise<void> {
        if (this.failure === null) {
            return;
        }
        const copied = await copyText(failureReport(this.failure));
        this.notice = copied
            ? { kind: 'info', text: t('recovery.copied') }
            : { kind: 'error', text: t('recovery.copyFailed') };
    }

    /** Shows the profile's database in the file manager. */
    async reveal(): Promise<void> {
        await this.#fireAndReport(() => commands.revealProfileFolder());
    }

    /** Quits the app. */
    async quit(): Promise<void> {
        await this.#fireAndReport(() => commands.quitApp());
    }

    async #fireAndReport(invoke: () => Promise<CommandResult<null>>): Promise<void> {
        try {
            const result = await invoke();
            if (result.status === 'error') {
                this.notice = { kind: 'error', text: commandErrorMessage(result.error) };
            }
        } catch {
            this.notice = { kind: 'error', text: t('app.error.internal') };
        }
    }

    /** Test isolation. */
    reset(): void {
        this.failure = null;
        this.recovered = null;
        this.busy = null;
        this.notice = null;
    }
}

export const startupStore = new StartupStore();
