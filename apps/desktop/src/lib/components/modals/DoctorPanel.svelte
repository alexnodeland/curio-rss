<script lang="ts">
/**
 * The database doctor: the health/maintenance commands (integrity checks,
 * vacuum, event-log retention sweep) plus the redacted, zero-telemetry
 * diagnostics bundle and the about-box facts. Every button is a thin call
 * to a health command; results surface as toasts, and the stats reload
 * after a vacuum.
 */
import { type AppInfoDto, type DbStatsDto, commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import type { CommandResult } from '$lib/state/query-cache.svelte';
import { uiStore } from '$lib/state/ui.svelte';

let stats: DbStatsDto | null = $state(null);
let info: AppInfoDto | null = $state(null);
let running: string | null = $state(null);

$effect(() => {
    void loadStats();
    void loadInfo();
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
        uiStore.showToast(success, 'success');
        await after?.();
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
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
        uiStore.showToast(t('doctor.swept', { count: result.data.length }), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
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
        uiStore.showToast(t('doctor.diagnosticsReady', { path: result.data.path }), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
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

    <div class="doctor-actions">
        <button type="button" onclick={integrity} disabled={running !== null}>
            {running === 'integrity' ? t('doctor.running') : t('doctor.integrity')}
        </button>
        <button type="button" onclick={ftsIntegrity} disabled={running !== null}>
            {running === 'fts' ? t('doctor.running') : t('doctor.ftsIntegrity')}
        </button>
        <button type="button" onclick={vacuum} disabled={running !== null}>
            {running === 'vacuum' ? t('doctor.running') : t('doctor.vacuum')}
        </button>
        <button type="button" onclick={() => void sweep()} disabled={running !== null}>
            {running === 'sweep' ? t('doctor.running') : t('doctor.sweep')}
        </button>
        <button type="button" onclick={() => void diagnostics()} disabled={running !== null}>
            {running === 'diagnostics' ? t('doctor.running') : t('doctor.diagnostics')}
        </button>
    </div>
</div>

<style>
    .doctor {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .about,
    .stat {
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }

    .mono {
        font-family: var(--font-mono);
        font-size: 0.75rem;
    }

    .doctor-actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        margin-top: var(--space-2);
    }

    .doctor-actions button {
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--bg-tertiary);
        color: var(--fg);
        border: 1px solid var(--border);
        font-size: 0.8125rem;
    }

    .doctor-actions button:hover:not(:disabled) {
        background: var(--bg-hover);
    }
</style>
