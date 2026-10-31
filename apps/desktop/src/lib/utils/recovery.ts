/**
 * The recovery screen's decisions, kept out of the component so they are
 * unit-tested: which actions a startup failure offers (and which one leads),
 * the copy for each class, and the plain-text report "Copy error" puts on the
 * clipboard. The backend enforces the same safety rules (a locked database is
 * never moved aside); this only decides what to show.
 */
import type { StartupFailureClass, StartupFailureDto } from '$lib/bindings';
import type { MessageKey } from '$lib/i18n';

/**
 * A recovery-screen action: re-open the profile (possibly after changing it),
 * or — for a database a newer Curio wrote — look for that newer Curio.
 */
export type RecoveryAction = 'retry' | 'restore' | 'fresh' | 'update';

export interface RecoveryPlan {
    /** The recommended action, drawn as the primary button. */
    primary: RecoveryAction | null;
    /** The other available actions. */
    secondary: RecoveryAction[];
}

/**
 * Classes where Curio may move the database aside (mirrors the backend's
 * `may_replace_database`): never while another process holds it, never when
 * the database is not what failed, never without a profile.
 */
function mayReplace(failureClass: StartupFailureClass): boolean {
    return !['locked', 'profile', 'no_profile'].includes(failureClass);
}

/**
 * The actions a failure offers, most useful first. A too-new database is
 * best fixed by updating Curio — lossless, unlike restoring a backup from
 * before the newer build's upgrade. Restoring needs a backup this build can
 * open; a retry only helps where the cause can go away (a lock, a refused
 * read or write, a failed upgrade, the unknown).
 */
export function recoveryPlan(failure: StartupFailureDto): RecoveryPlan {
    const actions: RecoveryAction[] = [];
    if (failure.class === 'too_new') {
        actions.push('update');
    }
    const retryHelps = ['locked', 'io', 'profile', 'upgrade', 'other'].includes(failure.class);
    if (retryHelps) {
        actions.push('retry');
    }
    if (mayReplace(failure.class)) {
        if (failure.backup !== null) {
            actions.push('restore');
        }
        actions.push('fresh');
    }
    const [primary = null, ...secondary] = actions;
    return { primary, secondary };
}

const TITLES: Record<StartupFailureClass, MessageKey> = {
    corrupt: 'recovery.title.corrupt',
    too_new: 'recovery.title.tooNew',
    locked: 'recovery.title.locked',
    io: 'recovery.title.io',
    profile: 'recovery.title.profile',
    upgrade: 'recovery.title.upgrade',
    no_profile: 'recovery.title.noProfile',
    other: 'recovery.title.other',
};

const BODIES: Record<StartupFailureClass, MessageKey> = {
    corrupt: 'recovery.body.corrupt',
    too_new: 'recovery.body.tooNew',
    locked: 'recovery.body.locked',
    io: 'recovery.body.io',
    profile: 'recovery.body.profile',
    upgrade: 'recovery.body.upgrade',
    no_profile: 'recovery.body.noProfile',
    other: 'recovery.body.other',
};

/** The heading and explanation for a failure class. */
export function recoveryCopy(failureClass: StartupFailureClass): {
    title: MessageKey;
    body: MessageKey;
} {
    return { title: TITLES[failureClass], body: BODIES[failureClass] };
}

/** The last path segment, for showing a backup by name. */
export function fileName(path: string): string {
    return path.split(/[\\/]/).pop() ?? path;
}

/**
 * The plain-text report "Copy error" puts on the clipboard. Deliberately not
 * localized: it is pasted into bug reports. Leads with the running version —
 * the first thing a too-new or failed-upgrade report needs.
 */
export function failureReport(failure: StartupFailureDto): string {
    const lines = [
        `Curio ${failure.app_version} could not start (${failure.class})`,
        '',
        failure.message,
    ];
    if (failure.profile_dir !== null || failure.backup !== null) {
        lines.push('');
    }
    if (failure.profile_dir !== null) {
        lines.push(`Profile: ${failure.profile_dir}`);
    }
    if (failure.backup !== null) {
        lines.push(`Backup: ${failure.backup}`);
    }
    return lines.join('\n');
}
