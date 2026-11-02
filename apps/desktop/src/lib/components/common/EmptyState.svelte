<script lang="ts">
/**
 * The one centered zero-state: an icon medallion, a title, an optional hint
 * and optional actions. Every empty view (no article selected, no search
 * matches, an empty feed/view) uses this so they read as one considered
 * family rather than a mix of centered medallions and bare top-left text.
 * The `error` tone marks a zero-state that is really a failure (a feed that
 * could not be fetched).
 */
import Icon, { type IconName } from '$components/common/Icon.svelte';
import type { Snippet } from 'svelte';

let {
    icon,
    title,
    hint,
    actions,
    tone = 'accent',
}: {
    icon: IconName;
    title: string;
    /** Optional secondary line (e.g. keyboard hints). */
    hint?: Snippet;
    /** Optional buttons under the copy (e.g. "Show read"). */
    actions?: Snippet;
    tone?: 'accent' | 'error';
} = $props();
</script>

<div class="empty-state">
    <div class="empty-mark" class:error={tone === 'error'} aria-hidden="true">
        <Icon name={icon} size={30} strokeWidth={1.5} />
    </div>
    <p class="empty-title">{title}</p>
    {#if hint}
        <p class="empty-hint">{@render hint()}</p>
    {/if}
    {#if actions}
        <div class="empty-actions">{@render actions()}</div>
    {/if}
</div>

<style>
    .empty-state {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-4);
        padding: var(--space-8);
        text-align: center;
    }

    .empty-mark {
        display: grid;
        place-items: center;
        width: 68px;
        height: 68px;
        border-radius: var(--radius-xl);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 70%);
    }

    .empty-mark.error {
        color: var(--error-text);
        background: var(--error-bg);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--error), transparent 70%);
    }

    .empty-title {
        font-size: var(--text-lg);
        font-weight: 560;
        color: var(--fg);
        letter-spacing: var(--tracking-snug);
    }

    .empty-hint {
        max-width: 34em;
        font-size: var(--text-sm);
        line-height: 1.5;
        color: var(--fg-subtle);
    }

    .empty-actions {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--space-2);
    }

    .empty-hint :global(kbd) {
        display: inline-block;
        min-width: 1.5em;
        padding: 0.1em 0.4em;
        margin: 0 0.1em;
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        box-shadow: 0 1px 0 var(--hairline-strong);
        font-family: var(--font-mono);
        font-size: 0.82em;
        color: var(--fg-muted);
        text-align: center;
    }
</style>
