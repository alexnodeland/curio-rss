/**
 * The recovery screen's decisions, kept out of the component so they are
 * unit-tested: which actions a startup failure offers (and which one leads),
 * the copy for each class, and the plain-text report "Copy error" puts on the
 * clipboard. The backend enforces the same safety rules (a locked database is
 * never moved aside); this only decides what to show.
 */
import type { StartupFailureClass, StartupFailureDto } from '$lib/bindings';
import type { MessageKey } from '$lib/i18n';

/** An action that re-opens the profile, possibly after changing it. */
export type RecoveryAction = 'retry' | 'restore' | 'fresh';

export interface RecoveryPlan {
    /** The recommended action, drawn as the primary button. */
    primary: RecoveryAction | null;
    /** The other available actions. */
    secondary: RecoveryAction[];
}

/** Classes where Curio may move the database aside (mirrors the backend). */
function mayReplace(failureClass: StartupFailureClass): boolean {
    return failureClass !== 'locked' && failureClass !== 'no_profile';
}

/**
 * The actions a failure offers, most useful first. Restoring needs a backup
 * this build can open; a retry only helps where the cause can go away on its
 * own (a lock, an IO refusal, a failed upgrade, the unknown).
 */
export function recoveryPlan(failure: StartupFailureDto): RecoveryPlan {
    const actions: RecoveryAction[] = [];
    const retryHelps = ['locked', 'io', 'upgrade', 'other'].includes(failure.class);
    if (retryHelps) {
        actions.push('retry');
    }
    if (mayReplace(failure.class)) {
        if (failure.backup !== null) {
            actions.push('restore');
        }
        actions.push('fresh');
    }
    // A too-new database without a backup is best fixed by updating Curio:
    // starting fresh stays available, but is not the recommendation.
    if (failure.class === 'too_new' && failure.backup === null) {
        return { primary: null, secondary: actions };
    }
    const [primary = null, ...secondary] = actions;
    return { primary, secondary };
}

const TITLES: Record<StartupFailureClass, MessageKey> = {
    corrupt: 'recovery.title.corrupt',
    too_new: 'recovery.title.tooNew',
    locked: 'recovery.title.locked',
    io: 'recovery.title.io',
    upgrade: 'recovery.title.upgrade',
    no_profile: 'recovery.title.noProfile',
    other: 'recovery.title.other',
};

const BODIES: Record<StartupFailureClass, MessageKey> = {
    corrupt: 'recovery.body.corrupt',
    too_new: 'recovery.body.tooNew',
    locked: 'recovery.body.locked',
    io: 'recovery.body.io',
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
 * localized: it is pasted into bug reports.
 */
export function failureReport(failure: StartupFailureDto): string {
    const lines = [`Curio could not start (${failure.class})`, '', failure.message];
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
