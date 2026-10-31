/**
 * Error presentation — the one place the three-tier `CommandError` contract
 * turns into copy: `user`-tier messages surface verbatim (they are written
 * to be actionable), `internal` failures get the generic line so raw
 * internals never leak into the UI. `PROFILE_BUSY` (another Curio process,
 * e.g. a CLI run, holds the profile) gets its own localized line: the
 * engine's message for it is English and names the lock file.
 */
import type { CommandError } from '$lib/bindings';
import { t } from '$lib/i18n';

/** The display string for a command failure. */
export function commandErrorMessage(error: CommandError): string {
    if (error.code === 'PROFILE_BUSY') return t('app.error.profileBusy');
    return error.kind === 'user' ? error.message : t('app.error.internal');
}
