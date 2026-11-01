<script lang="ts">
/**
 * The toast outlet — renders `toastStore.toasts` (queued by the action layer
 * on command failures, and by anything else that needs a notice), each with
 * its optional action button (Undo / Retry) and a close button.
 *
 * Announcements ride two permanent, visually hidden live regions mounted with
 * the outlet — polite for everything, assertive for errors — that exist
 * before the first toast; a node that mounts already carrying a live role is
 * often not read (VoiceOver), so the toast cards themselves carry none.
 * Hovering or focusing the stack holds every dismiss timer (WCAG 2.2.1).
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import { toastStore } from '$lib/state/toasts.svelte';
import { fly } from 'svelte/transition';

/**
 * True when motion should be suppressed: the OS asks for reduced motion, or
 * the `prefers-reduced-motion` query is unavailable (e.g. jsdom in tests) —
 * in which case the leave is instant. The packaged webview always has the
 * query, so real users only lose the animation when they ask to.
 */
function reducedMotion(): boolean {
    return (
        typeof window === 'undefined' ||
        typeof window.matchMedia !== 'function' ||
        window.matchMedia('(prefers-reduced-motion: reduce)').matches
    );
}

// The leave transition (the entry stays a CSS keyframe, already collapsed by
// the global reduced-motion rule). Read once at mount, like the theme probe.
const leave = { y: 8, duration: reducedMotion() ? 0 : 200 };
</script>

<div class="sr-only" role="status" aria-live="polite" data-live-region="polite">
    {#each toastStore.politeAnnouncements as line (line.id)}
        <p>{line.text}</p>
    {/each}
</div>
<div class="sr-only" role="alert" aria-live="assertive" data-live-region="assertive">
    {#each toastStore.assertiveAnnouncements as line (line.id)}
        <p>{line.text}</p>
    {/each}
</div>

<!-- Pointer or focus on the stack holds every timer; leaving resumes it. A
     button that closes its own toast ends the focus hold in the store (the
     removed node need not fire focusout). A labelled landmark, so a
     screen-reader user can jump to an Undo / Retry. -->
<div
    class="toasts"
    role="region"
    aria-label={t('toast.region')}
    onmouseenter={() => toastStore.hold('pointer')}
    onmouseleave={() => toastStore.release('pointer')}
    onfocusin={() => toastStore.hold('focus')}
    onfocusout={(event) => {
        const next = event.relatedTarget;
        if (!(next instanceof Node && event.currentTarget.contains(next))) {
            toastStore.release('focus');
        }
    }}
>
    {#each toastStore.toasts as toast (toast.id)}
        <div class="toast toast-{toast.tone}" out:fly={leave}>
            <span class="toast-message">{toast.message}</span>
            {#if toast.action !== undefined}
                <button class="toast-action" onclick={() => toastStore.runAction(toast.id)}
                    >{toast.action.label}</button
                >
            {/if}
            <button
                class="toast-dismiss"
                onclick={() => toastStore.dismissToast(toast.id)}
                aria-label={t('toast.dismiss')}
            >
                <Icon name="close" size={16} />
            </button>
        </div>
    {/each}
</div>

<style>
    .toasts {
        position: fixed;
        right: var(--space-4);
        bottom: var(--space-4);
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
        z-index: 200;
        max-width: min(360px, calc(100vw - var(--space-8)));
    }

    .toast {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        padding-left: var(--space-5);
        border-radius: var(--radius-lg);
        border: 1px solid var(--hairline);
        background: var(--surface-overlay);
        color: var(--fg);
        box-shadow: var(--shadow-lg);
        font-size: var(--text-md);
        overflow: hidden;
        animation: toast-in var(--dur-base) var(--ease);
    }

    /* Tone accent as a left bar, not a full border tint. */
    .toast::before {
        content: '';
        position: absolute;
        left: 0;
        top: 0;
        bottom: 0;
        width: 3px;
        background: var(--accent);
    }

    .toast-error::before {
        background: var(--error);
    }
    .toast-success::before {
        background: var(--success);
    }
    .toast-warning::before {
        background: var(--warning);
    }

    @keyframes toast-in {
        from {
            opacity: 0;
            transform: translateY(6px);
        }
    }

    .toast-message {
        flex: 1 1 auto;
        min-width: 0;
        overflow-wrap: break-word;
    }

    .toast-action {
        flex: 0 0 auto;
        padding: var(--space-1) var(--space-2);
        margin-block: calc(-1 * var(--space-1));
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--accent);
        font-size: var(--text-sm);
        font-weight: 600;
        white-space: nowrap;
        transition: background var(--dur-fast) var(--ease);
    }

    .toast-action:hover {
        background: var(--hover);
    }

    .toast-dismiss {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 1.35rem;
        height: 1.35rem;
        font-size: 1rem;
        line-height: 1;
        color: var(--fg-muted);
        background: transparent;
        border-radius: var(--radius-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .toast-dismiss:hover {
        color: var(--fg);
        background: var(--hover);
    }
</style>
