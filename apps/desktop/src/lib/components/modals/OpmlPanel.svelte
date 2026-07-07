<script lang="ts">
/**
 * Import (OPML + Pocket/Instapaper/Readwise CSV) and OPML export over the
 * PathToken flow: the file is chosen by a native Rust-side dialog
 * (`pick_import_file` / `pick_export_path`), which returns an opaque token;
 * only that token crosses back to the IO command. A raw path never leaves
 * this process as a free string (ipc_policy / D13). Invalidation rides the
 * Rust-emitted `FeedsChanged` / `ArticlesChanged` events — no client-side
 * cache bookkeeping. The chosen `source` tells Rust which parser to run.
 */
import { commands, type ImportSourceDto } from '$lib/bindings';
import { type MessageKey, t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { uiStore } from '$lib/state/ui.svelte';

const SOURCES: readonly ImportSourceDto[] = [
    'opml',
    'pocket_csv',
    'instapaper_csv',
    'readwise_csv',
];

let source = $state<ImportSourceDto>('opml');
let importing = $state(false);
let exporting = $state(false);

/** The i18n key for a source's human label, typed so the catalog is checked. */
function sourceLabel(value: ImportSourceDto): MessageKey {
    return `import.source.${value}`;
}

async function runImport(): Promise<void> {
    importing = true;
    try {
        const picked = await commands.pickImportFile();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            uiStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const result = await commands.importFile(picked.data.token, source);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        const { feeds_added, articles_added, feeds_skipped, articles_skipped } = result.data;
        uiStore.showToast(
            t('import.done', {
                feeds: feeds_added,
                articles: articles_added,
                skipped: feeds_skipped + articles_skipped,
            }),
            'success',
        );
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        importing = false;
    }
}

async function runExport(): Promise<void> {
    exporting = true;
    try {
        const picked = await commands.pickExportPath();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            uiStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const result = await commands.exportOpml(picked.data.token);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        uiStore.showToast(t('opml.exported'), 'success');
    } catch {
        uiStore.showToast(t('app.error.internal'), 'error');
    } finally {
        exporting = false;
    }
}
</script>

<div class="opml">
    <label class="opml-format">
        <span class="opml-format-label">{t('import.format')}</span>
        <select class="opml-select" bind:value={source} disabled={importing}>
            {#each SOURCES as value (value)}
                <option {value}>{t(sourceLabel(value))}</option>
            {/each}
        </select>
    </label>
    <div class="opml-actions">
        <button class="opml-button" type="button" onclick={() => void runImport()} disabled={importing}>
            {importing ? t('import.running') : t('import.run')}
        </button>
        <button class="opml-button" type="button" onclick={() => void runExport()} disabled={exporting}>
            {exporting ? t('opml.exporting') : t('opml.export')}
        </button>
    </div>
    <p class="opml-hint">{t('import.hint')}</p>
</div>

<style>
    .opml {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .opml-format {
        display: flex;
        align-items: center;
        gap: var(--space-2);
    }

    .opml-format-label {
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .opml-select {
        flex: 1;
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background: var(--bg-inset, transparent);
        color: var(--fg);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
    }

    .opml-actions {
        display: flex;
        gap: var(--space-2);
    }

    .opml-button {
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        font-weight: 500;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .opml-button:hover:not(:disabled) {
        background: var(--hover);
        color: var(--fg);
    }

    .opml-hint {
        font-size: var(--text-xs);
        color: var(--fg-subtle);
    }
</style>
