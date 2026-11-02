/**
 * Article retention and the maintenance pass (#137): the choices the
 * Settings → Data retention controls offer, and how a pass's report reads
 * as toasts in the doctor panel. Pure helpers — the components only call
 * the commands and show what these return.
 */
import type { MaintenanceReportDto } from '$lib/bindings';
import { t } from '$lib/i18n';
import type { ToastTone } from '$lib/state/toasts.svelte';

/** The age limits offered, in days. */
export const RETENTION_AGE_DAYS: readonly number[] = [30, 90, 180, 365];

/** The per-feed limits offered, in articles. */
export const RETENTION_PER_FEED: readonly number[] = [100, 250, 500, 1000];

/**
 * The limits a select offers: the presets, plus the current one when it is
 * not a preset (set with `curio retention`), so the control shows what is
 * really stored instead of snapping it to a preset. Ascending.
 */
export function limitChoices(presets: readonly number[], current: number | null): number[] {
    const choices = [...presets];
    if (current !== null && current > 0 && !choices.includes(current)) {
        choices.push(current);
    }
    return choices.sort((a, b) => a - b);
}

/** A select's value as a limit: `''` (the "off" option) or anything not a positive whole number is no limit. */
export function parseLimit(value: string): number | null {
    const limit = Number(value);
    return value !== '' && Number.isInteger(limit) && limit > 0 ? limit : null;
}

/** A toast to show. */
export interface MaintenanceToast {
    readonly message: string;
    readonly tone: ToastTone;
}

/**
 * What a pass's report tells the user: a summary (the purge count for an
 * orphan purge), or why nothing was deleted when the exported set could not
 * be read; plus a note when some upkeep is left for the next pass.
 */
export function maintenanceToasts(
    report: MaintenanceReportDto,
    purgedOrphans: boolean,
): MaintenanceToast[] {
    const toasts: MaintenanceToast[] = [];
    if (report.reap_skipped !== null) {
        toasts.push({
            message: t('doctor.maintenance.reapSkipped', { reason: report.reap_skipped }),
            tone: 'warning',
        });
    } else if (purgedOrphans) {
        toasts.push({
            message: t('doctor.purged', { count: report.orphans_purged }),
            tone: 'success',
        });
    } else {
        toasts.push({
            message: t('doctor.maintenance.done', {
                articles: report.articles_reaped,
                tags: report.tags_dropped,
                logs: report.event_files_removed,
            }),
            tone: 'success',
        });
    }
    if (report.event_sweep_skipped !== null || report.upkeep_skipped !== null) {
        toasts.push({ message: t('doctor.maintenance.partial'), tone: 'warning' });
    }
    return toasts;
}
