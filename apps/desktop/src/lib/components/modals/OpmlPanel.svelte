<script lang="ts">
/**
 * OPML import / export over the PathToken flow: the file is chosen by a
 * native Rust-side dialog (`pick_import_file` / `pick_export_path`), which
 * returns an opaque token; only that token crosses back to the IO command.
 * A raw path never leaves this process as a free string (ipc_policy / D13).
 * Import invalidation rides the Rust-emitted `FeedsChanged` event — no
 * client-side cache bookkeeping.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { toastCommandError } from '$lib/state/actions';
import { uiStore } from '$lib/state/ui.svelte';

let importing = $state(false);
let exporting = $state(false);

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
        const result = await commands.importOpml(picked.data.token);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        uiStore.showToast(
            t('opml.imported', { added: result.data.added, skipped: result.data.skipped }),
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
    <div class="opml-actions">
        <button class="opml-button" type="button" onclick={() => void runImport()} disabled={importing}>
            {importing ? t('opml.importing') : t('opml.import')}
        </button>
        <button class="opml-button" type="button" onclick={() => void runExport()} disabled={exporting}>
            {exporting ? t('opml.exporting') : t('opml.export')}
        </button>
    </div>
    <p class="opml-hint">{t('opml.hint')}</p>
</div>

<style>
    .opml {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }

    .opml-actions {
        display: flex;
        gap: var(--space-2);
    }

    .opml-button {
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: var(--bg-tertiary);
        color: var(--fg);
        border: 1px solid var(--border);
        font-size: 0.8125rem;
    }

    .opml-button:hover:not(:disabled) {
        background: var(--bg-hover);
    }

    .opml-hint {
        font-size: 0.75rem;
        color: var(--fg-muted);
    }
</style>
