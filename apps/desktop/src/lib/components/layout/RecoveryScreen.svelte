<script lang="ts">
/**
 * The startup recovery screen: shown instead of the reader when the profile
 * would not open (see `startup.rs`). It says what went wrong in plain words,
 * leads with the action most likely to help, and keeps the escape hatches
 * (show the profile folder, copy the error, quit) always in reach. Restoring
 * a backup and starting fresh ask for confirmation first; both move the
 * current database aside, never delete it.
 *
 * Keyboard focus follows the flow: into the confirmation when it opens, back
 * to the action that opened it on cancel (or that is now running), and onto
 * "Open Curio" once recovered. While an action runs, every button stays
 * focusable but inert (`aria-disabled`), so focus is never dropped to the
 * page and nothing — quit included — can interrupt the file moves.
 */
import Icon from '$components/common/Icon.svelte';
import type { StartupFailureDto } from '$lib/bindings';
import { type MessageKey, t } from '$lib/i18n';
import { startupStore } from '$lib/state/startup.svelte';
import { type RecoveryAction, fileName, recoveryCopy, recoveryPlan } from '$lib/utils/recovery';
import { tick } from 'svelte';

let { failure, onopen }: { failure: StartupFailureDto; onopen: () => void } = $props();

const LABELS: Record<RecoveryAction, MessageKey> = {
    retry: 'recovery.action.retry',
    restore: 'recovery.action.restore',
    fresh: 'recovery.action.fresh',
    update: 'recovery.action.update',
};

/** The action buttons, to hand focus back to the one a flow started from. */
const actionButtons: Partial<Record<RecoveryAction, HTMLButtonElement>> = $state({});
let confirmButton: HTMLButtonElement | undefined = $state();
let openButton: HTMLButtonElement | undefined = $state();

$effect(() => {
    if (startupStore.recovered !== null) {
        openButton?.focus();
    }
});

const CONFIRMS: Partial<Record<RecoveryAction, MessageKey>> = {
    restore: 'recovery.confirm.restore',
    fresh: 'recovery.confirm.fresh',
};

/** The destructive-looking action awaiting confirmation, if any. */
let confirming: RecoveryAction | null = $state(null);

const plan = $derived(recoveryPlan(failure));
/** Every offered action, the recommended one first. */
const actions = $derived(
    plan.primary === null ? plan.secondary : [plan.primary, ...plan.secondary],
);
const copy = $derived(recoveryCopy(failure.class));
const replaces = $derived(actions.some((action) => action === 'restore' || action === 'fresh'));
const busy = $derived(startupStore.busy !== null);

/** The text either side of `{param}` in a message, so a path can be set in code. */
const SLOT = '\u0000';
function around(key: MessageKey, param: string): [string, string] {
    const [before = '', after = ''] = t(key, { [param]: SLOT }).split(SLOT);
    return [before, after];
}

async function choose(action: RecoveryAction): Promise<void> {
    if (busy) {
        return;
    }
    if (CONFIRMS[action] !== undefined) {
        confirming = action;
        await tick();
        confirmButton?.focus();
        return;
    }
    await startupStore.run(action);
}

/** Closes the confirmation and puts focus back on the action that opened it. */
async function closeConfirm(): Promise<RecoveryAction | null> {
    const action = confirming;
    confirming = null;
    await tick();
    if (action !== null) {
        actionButtons[action]?.focus();
    }
    return action;
}

async function confirm(): Promise<void> {
    const action = await closeConfirm();
    if (action !== null) {
        await startupStore.run(action);
    }
}

/** A footer action, inert while a recovery action runs. */
function whenIdle(run: () => Promise<void>): () => void {
    return () => {
        if (!busy) {
            void run();
        }
    };
}

function label(action: RecoveryAction): string {
    return startupStore.busy === action ? t('recovery.working') : t(LABELS[action]);
}
</script>

{#snippet withPath(key: MessageKey, param: string, value: string)}
    {@const [before, after] = around(key, param)}
    <p class="meta">{before}<code class="path">{value}</code>{after}</p>
{/snippet}

<main class="recovery" aria-labelledby="recovery-title">
    <section class="card">
        {#if startupStore.recovered !== null}
            <div class="mark mark-ok" aria-hidden="true">
                <Icon name="check" size={26} strokeWidth={2} />
            </div>
            <h1 id="recovery-title" class="title">{t('recovery.recovered.title')}</h1>
            <p class="body">{t('recovery.recovered.body')}</p>
            {#if startupStore.recovered.setAside !== null}
                {@render withPath('recovery.recovered.setAside', 'path', startupStore.recovered.setAside)}
            {/if}
            <div class="actions">
                <button type="button" class="btn btn-primary" bind:this={openButton} onclick={onopen}>
                    {t('recovery.action.open')}
                </button>
            </div>
        {:else}
            <div class="mark" aria-hidden="true">
                <Icon name="alert" size={26} strokeWidth={1.75} />
            </div>
            <h1 id="recovery-title" class="title">{t(copy.title)}</h1>
            <p class="body">{t(copy.body)}</p>
            {#if failure.backup !== null}
                {@render withPath('recovery.backup', 'name', fileName(failure.backup))}
            {/if}

            {#if confirming !== null}
                <div class="confirm" role="group" aria-labelledby="recovery-confirm">
                    <p id="recovery-confirm" class="confirm-text">
                        {t(CONFIRMS[confirming] ?? 'recovery.confirm.fresh')}
                    </p>
                    <div class="actions">
                        <button
                            type="button"
                            class="btn btn-primary"
                            aria-describedby="recovery-confirm"
                            bind:this={confirmButton}
                            onclick={confirm}
                        >
                            {t(LABELS[confirming])}
                        </button>
                        <button type="button" class="btn" onclick={() => void closeConfirm()}>
                            {t('recovery.action.cancel')}
                        </button>
                    </div>
                </div>
            {:else if actions.length > 0}
                <div class="actions">
                    {#each actions as action (action)}
                        <button
                            type="button"
                            class="btn"
                            class:btn-primary={action === plan.primary}
                            aria-disabled={busy}
                            bind:this={actionButtons[action]}
                            onclick={() => void choose(action)}
                        >
                            {label(action)}
                        </button>
                    {/each}
                </div>
            {/if}

            {#if replaces}
                <p class="hint">{t('recovery.safety')}</p>
            {/if}

            <details class="details">
                <summary>{t('recovery.details')}</summary>
                <pre class="message">{failure.message}</pre>
                {#if failure.profile_dir !== null}
                    {@render withPath('recovery.profile', 'path', failure.profile_dir)}
                {/if}
            </details>

            <footer class="footer">
                {#if failure.profile_dir !== null}
                    <button
                        type="button"
                        class="link-btn"
                        aria-disabled={busy}
                        onclick={whenIdle(() => startupStore.reveal())}
                    >
                        {t('recovery.action.reveal')}
                    </button>
                {/if}
                <button
                    type="button"
                    class="link-btn"
                    aria-disabled={busy}
                    onclick={whenIdle(() => startupStore.copyReport())}
                >
                    {t('recovery.action.copy')}
                </button>
                <span class="spacer"></span>
                <button
                    type="button"
                    class="link-btn"
                    aria-disabled={busy}
                    onclick={whenIdle(() => startupStore.quit())}
                >
                    {t('recovery.action.quit')}
                </button>
            </footer>
        {/if}

        <p
            class="notice"
            class:notice-error={startupStore.notice?.kind === 'error'}
            role="status"
            aria-live="polite"
        >{startupStore.notice?.text ?? ''}</p>
    </section>
</main>

<style>
    .recovery {
        min-height: 100vh;
        display: grid;
        place-items: center;
        padding: var(--space-8) var(--space-6);
        background: var(--surface-chrome);
        color: var(--fg);
    }

    .card {
        width: min(100%, 520px);
        display: flex;
        flex-direction: column;
        align-items: flex-start;
        gap: var(--space-3);
        padding: var(--space-8);
        border-radius: var(--radius-xl);
        background: var(--surface-app);
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-lg);
    }

    .mark {
        display: grid;
        place-items: center;
        width: 52px;
        height: 52px;
        margin-bottom: var(--space-1);
        border-radius: var(--radius-lg);
        color: var(--warning-text);
        background: color-mix(in srgb, transparent, var(--warning) 14%);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--warning), transparent 70%);
    }

    .mark-ok {
        color: var(--success-text);
        background: color-mix(in srgb, transparent, var(--success) 14%);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--success), transparent 70%);
    }

    .title {
        font-size: var(--text-xl);
        font-weight: 620;
        letter-spacing: var(--tracking-tight);
        line-height: 1.25;
    }

    .body {
        font-size: var(--text-base);
        color: var(--fg-muted);
        line-height: 1.5;
    }

    .meta {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.5;
        overflow-wrap: anywhere;
    }

    .path {
        font-family: var(--font-mono);
        font-size: 0.92em;
        color: var(--fg-muted);
    }

    .actions {
        display: flex;
        flex-wrap: wrap;
        gap: var(--space-2);
        margin-top: var(--space-2);
    }

    .btn {
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        border: 1px solid var(--hairline-strong);
        background: transparent;
        color: var(--fg);
        font-size: var(--text-md);
        font-weight: 520;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .btn:hover:not([aria-disabled='true']) {
        background: var(--hover);
    }

    /* Inert while an action runs, but still focusable (see the header). */
    .btn[aria-disabled='true'],
    .link-btn[aria-disabled='true'] {
        cursor: default;
        opacity: 0.55;
    }

    .btn-primary {
        background: var(--accent);
        border-color: transparent;
        color: var(--accent-fg);
    }

    .btn-primary:hover:not([aria-disabled='true']) {
        background: var(--accent-hover);
    }

    .confirm {
        width: 100%;
        margin-top: var(--space-2);
        padding: var(--space-4);
        border-radius: var(--radius-lg);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
    }

    .confirm-text {
        font-size: var(--text-md);
        line-height: 1.5;
    }

    .hint {
        font-size: var(--text-sm);
        color: var(--fg-subtle);
        line-height: 1.5;
    }

    .details {
        width: 100%;
        margin-top: var(--space-1);
        font-size: var(--text-sm);
        color: var(--fg-muted);
    }

    .details summary {
        cursor: pointer;
        width: fit-content;
        border-radius: var(--radius-sm);
    }

    .details[open] summary {
        margin-bottom: var(--space-2);
    }

    .message {
        margin: 0 0 var(--space-2);
        padding: var(--space-3);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        line-height: 1.55;
        white-space: pre-wrap;
        overflow-wrap: anywhere;
        color: var(--fg-muted);
    }

    .footer {
        width: 100%;
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-1) var(--space-4);
        margin-top: var(--space-3);
        padding-top: var(--space-4);
        border-top: 1px solid var(--hairline);
    }

    .spacer {
        flex: 1 1 auto;
    }

    .link-btn {
        padding: var(--space-1) 0;
        font-size: var(--text-sm);
        font-weight: 500;
        color: var(--fg-muted);
    }

    .link-btn:hover:not([aria-disabled='true']) {
        color: var(--fg);
    }

    .notice {
        font-size: var(--text-sm);
        line-height: 1.5;
        color: var(--fg-muted);
    }

    /* The live region stays in the tree (so it announces), but an empty one
       takes no room: cancel the card's gap above it. */
    .notice:empty {
        margin-top: calc(-1 * var(--space-3));
    }

    .notice-error {
        color: var(--error-text);
    }
</style>
