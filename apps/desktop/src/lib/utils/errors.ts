/**
 * Error presentation — the one place the three-tier `CommandError` contract
 * turns into copy: `user`-tier messages surface verbatim (they are written
 * to be actionable), `internal` failures get the generic line so raw
 * internals never leak into the UI.
 */
import type { CommandError } from '$lib/bindings';
import { t } from '$lib/i18n';

/** The display string for a command failure. */
export function commandErrorMessage(error: CommandError): string {
    return error.kind === 'user' ? error.message : t('app.error.internal');
}
