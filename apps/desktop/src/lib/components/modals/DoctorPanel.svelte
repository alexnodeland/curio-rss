<script lang="ts">
/**
 * The database doctor: the health/maintenance commands (integrity checks,
 * the search-index rebuild that repairs a failed index check, vacuum, the
 * maintenance pass, event-log retention sweep, and — behind a confirmation
 * — deleting the articles of removed feeds) plus the redacted,
 * zero-telemetry diagnostics bundle and the about-box facts, plus what
 * startup did with a library from an earlier release. Every button is a
 * thin call to a health command; results surface as toasts, and the stats
 * reload after anything that can shrink the library.
 */
import { type AppInfoDto, type DbStatsDto, type LegacyProfileDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import type { CommandResult } from '$lib/state/query-cache.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { legacyProfileDetail, loadLegacyProfiles } from '$lib/utils/legacy-profiles';
import { maintenanceToasts } from '$lib/utils/maintenance';

let stats: DbStatsDto | null = $state(null);
let info: AppInfoDto | null = $state(null);
let legacy: LegacyProfileDto[] = $state([]);
let running: string | null = $state(null);
let confirmingPurge = $state(false);

$effect(() => {
    void loadStats();
    void loadInfo();
    void loadLegacyProfiles().then((outcomes) => {
        legacy = outcomes;
    });
});

async function loadStats(): Promise<void> {
    const result = await commands.dbStats();
    if (result.status === 'ok') {
        stats = result.data;
    }
}

async function loadInfo(): Promise<void> {
    const result = await commands.getAppInfo();
    if (result.status === 'ok') {
        info = result.data;
    }
}

/** Runs one doctor action, toasting the outcome; `after` runs on success. */
async function run(
    id: string,
    invoke: () => Promise<CommandResult<unknown>>,
    success: string,
    after?: () => Promise<void>,
): Promise<void> {
    running = id;
    try {
        const result = await invoke();
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        toastStore.showToast(success, 'success');
        await after?.();
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        running = null;
    }
}

function integrity(): void {
    void run(
        'integrity',
        () => commands.integrityCheck(),
        t('doctor.passed', { check: t('doctor.check.integrity') }),
    );
}

function ftsIntegrity(): void {
    void run(
        'fts',
        () => commands.ftsIntegrityCheck(),
        t('doctor.passed', { check: t('doctor.check.fts') }),
    );
}

function rebuildSearchIndex(): void {
    void run('rebuild', () => commands.rebuildSearchIndex(), t('doctor.rebuiltFts'));
}

function vacuum(): void {
    void run('vacuum', () => commands.vacuumDb(), t('doctor.vacuumed'), loadStats);
}

async function sweep(): Promise<void> {
    running = 'sweep';
    try {
        const result = await commands.sweepEventRetention();
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        toastStore.showToast(t('doctor.swept', { count: result.data.length }), 'success');
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        running = null;
    }
}

/** Runs the maintenance pass now — with `purgeOrphans`, the orphan purge too. */
async function maintenance(purgeOrphans: boolean): Promise<void> {
    running = purgeOrphans ? 'purge' : 'maintenance';
    confirmingPurge = false;
    try {
        const result = await commands.runMaintenance(purgeOrphans);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        for (const toast of maintenanceToasts(result.data, purgeOrphans)) {
            toastStore.showToast(toast.message, toast.tone);
        }
        await loadStats();
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        running = null;
    }
}

async function diagnostics(): Promise<void> {
    running = 'diagnostics';
    try {
        const result = await commands.copyDiagnosticsBundle();
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        toastStore.showToast(t('doctor.diagnosticsReady', { path: result.data.path }), 'success');
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    } finally {
        running = null;
    }
}
</script>

<div class="doctor">
    {#if info !== null}
        <p class="about">{t('settings.about.version', { version: info.version })}</p>
        <p class="about mono truncate" title={info.profile_dir}>
            {t('settings.about.profile', { path: info.profile_dir })}
        </p>
        <p class="about">{t('settings.about.schema', { version: info.db_schema_version })}</p>
    {/if}

    {#if stats !== null}
        <p class="stat">{t('doctor.stats.articles', { count: stats.article_count })}</p>
        <p class="stat mono truncate" title={stats.db_path}>
            {t('doctor.stats.db', { path: stats.db_path })}
        </p>
    {/if}

    {#if legacy.length > 0}
        <section class="legacy" aria-labelledby="doctor-legacy-title">
            <h4 id="doctor-legacy-title">{t('legacy.title')}</h4>
            {#each legacy as outcome (outcome.path)}
                <p class="legacy-item" class:left={outcome.status !== 'moved'}>
                    {legacyProfileDetail(outcome)}
                </p>
            {/each}
        </section>
    {/if}

    <div class="doctor-actions">
        <button type="button" onclick={integrity} disabled={running !== null}>
            {running === 'integrity' ? t('doctor.running') : t('doctor.integrity')}
        </button>
        <button type="button" onclick={ftsIntegrity} disabled={running !== null}>
            {running === 'fts' ? t('doctor.running') : t('doctor.ftsIntegrity')}
        </button>
        <button type="button" onclick={rebuildSearchIndex} disabled={running !== null}>
            {running === 'rebuild' ? t('doctor.running') : t('doctor.rebuildFts')}
        </button>
        <button type="button" onclick={vacuum} disabled={running !== null}>
            {running === 'vacuum' ? t('doctor.running') : t('doctor.vacuum')}
        </button>
        <button type="button" onclick={() => void maintenance(false)} disabled={running !== null}>
            {running === 'maintenance' ? t('doctor.running') : t('doctor.maintenance')}
        </button>
        <button type="button" onclick={() => void sweep()} disabled={running !== null}>
            {running === 'sweep' ? t('doctor.running') : t('doctor.sweep')}
        </button>
        <button type="button" onclick={() => void diagnostics()} disabled={running !== null}>
            {running === 'diagnostics' ? t('doctor.running') : t('doctor.diagnostics')}
        </button>
    </div>

    <section class="purge" aria-label={t('doctor.purgeOrphans')}>
        {#if confirmingPurge}
            <p class="purge-prompt">{t('doctor.purgeOrphans.confirm')}</p>
            <div class="purge-actions">
                <button
                    type="button"
                    class="danger"
                    onclick={() => void maintenance(true)}
                    disabled={running !== null}>{t('doctor.purgeOrphans.delete')}</button
                >
                <button type="button" onclick={() => (confirmingPurge = false)}
                    >{t('doctor.purgeOrphans.cancel')}</button
                >
            </div>
        {:else}
            <button
                type="button"
                onclick={() => (confirmingPurge = true)}
                disabled={running !== null}
            >
                {running === 'purge' ? t('doctor.running') : t('doctor.purgeOrphans')}
            </button>
        {/if}
    </section>
</div>

<style>
    .doctor {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .about,
    .stat {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .mono {
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }

    .legacy {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding-top: var(--space-4);
        margin-top: var(--space-2);
        border-top: 1px solid var(--hairline);
    }

    h4 {
        font-size: 0.6875rem;
        font-weight: var(--weight-heading);
        letter-spacing: var(--tracking-caps);
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .legacy-item {
        font-size: var(--text-sm);
        line-height: 1.5;
        color: var(--fg-muted);
        overflow-wrap: anywhere;
    }

    .legacy-item.left {
        padding-left: var(--space-3);
        border-left: 2px solid var(--warning);
    }

    .doctor-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        margin-top: var(--space-4);
    }

    .doctor-actions button {
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        font-weight: var(--weight-medium);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .doctor-actions button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    /* Deleting articles sits apart from the routine actions, behind a
       confirmation — the same shape as a feed's unsubscribe. */
    .purge {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        padding-top: var(--space-3);
        margin-top: var(--space-2);
        border-top: 1px solid var(--hairline);
    }

    .purge-prompt {
        font-size: var(--text-sm);
        line-height: 1.45;
        color: var(--fg-muted);
    }

    .purge-actions {
        display: flex;
        gap: var(--space-2);
    }

    .purge button {
        align-self: flex-start;
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        font-weight: var(--weight-medium);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease),
            border-color var(--dur-fast) var(--ease);
    }

    .purge button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .purge .danger {
        color: var(--error-text);
        border-color: color-mix(in srgb, var(--error), transparent 40%);
    }

    .purge .danger:hover:not(:disabled) {
        background: color-mix(in srgb, var(--error), transparent 88%);
        border-color: var(--error);
        color: var(--error-text);
    }
</style>
