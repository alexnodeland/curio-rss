<script lang="ts">
import { addFeed, folders } from '$lib/stores/feeds';
import { createEventDispatcher } from 'svelte';

const dispatch = createEventDispatcher<{ close: undefined }>();

// Extract error message from various error formats (Tauri, native Error, string)
function extractErrorMessage(e: unknown): string {
    if (e instanceof Error) {
        return e.message;
    }
    if (typeof e === 'string') {
        return e;
    }
    if (typeof e === 'object' && e !== null) {
        // Tauri CommandError format: { User: { message, ... } } or { Internal: { message, ... } }
        const obj = e as Record<string, unknown>;
        if ('User' in obj && typeof obj.User === 'object' && obj.User !== null) {
            const user = obj.User as Record<string, unknown>;
            if (typeof user.message === 'string') return user.message;
        }
        if ('Internal' in obj && typeof obj.Internal === 'object' && obj.Internal !== null) {
            const internal = obj.Internal as Record<string, unknown>;
            if (typeof internal.message === 'string') return internal.message;
        }
        // Direct message property
        if ('message' in obj && typeof obj.message === 'string') {
            return obj.message;
        }
    }
    return 'An unexpected error occurred';
}

let feedUrl = '';
let selectedFolderId: string | null = null;
let isLoading = false;
let error = '';

async function handleSubmit() {
    if (!feedUrl.trim()) {
        error = 'Please enter a feed URL';
        return;
    }

    isLoading = true;
    error = '';

    try {
        await addFeed(feedUrl.trim(), selectedFolderId ?? undefined);
        dispatch('close');
    } catch (e) {
        console.error('Add feed error:', e);
        // Tauri errors serialize as { User: { message, code, recoverable } } or { Internal: { message } }
        error = extractErrorMessage(e);
    } finally {
        isLoading = false;
    }
}

function handleKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
        dispatch('close');
    }
}

function handleBackdropClick(event: MouseEvent) {
    if (event.target === event.currentTarget) {
        dispatch('close');
    }
}
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="modal-backdrop" on:click={handleBackdropClick} role="presentation">
    <div class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <header class="modal-header">
            <h2 id="modal-title">Add Feed</h2>
            <button class="close-btn" on:click={() => dispatch('close')} aria-label="Close">
                <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <line x1="18" y1="6" x2="6" y2="18"/>
                    <line x1="6" y1="6" x2="18" y2="18"/>
                </svg>
            </button>
        </header>

        <form on:submit|preventDefault={handleSubmit}>
            <div class="form-group">
                <label for="feed-url">Feed URL</label>
                <input
                    id="feed-url"
                    type="url"
                    bind:value={feedUrl}
                    placeholder="https://example.com/feed.xml"
                    disabled={isLoading}
                    required
                />
                <p class="help-text">
                    Enter an RSS/Atom feed URL, YouTube channel, or subreddit URL
                </p>
            </div>

            <div class="form-group">
                <label for="feed-folder">Folder (optional)</label>
                <select id="feed-folder" bind:value={selectedFolderId} disabled={isLoading}>
                    <option value={null}>No folder</option>
                    {#each [...$folders.values()] as folder}
                        <option value={folder.id}>{folder.name}</option>
                    {/each}
                </select>
            </div>

            {#if error}
                <div class="error-message" role="alert">{error}</div>
            {/if}

            <div class="form-actions">
                <button type="button" class="btn-secondary" on:click={() => dispatch('close')} disabled={isLoading}>
                    Cancel
                </button>
                <button type="submit" class="btn-primary" disabled={isLoading}>
                    {isLoading ? 'Adding...' : 'Add Feed'}
                </button>
            </div>
        </form>
    </div>
</div>

<style>
    .modal-backdrop {
        position: fixed;
        inset: 0;
        background: rgba(0, 0, 0, 0.5);
        display: flex;
        align-items: center;
        justify-content: center;
        z-index: 1000;
    }

    .modal {
        background: var(--bg);
        border-radius: var(--radius-lg);
        box-shadow: 0 25px 50px -12px rgba(0, 0, 0, 0.25);
        width: 100%;
        max-width: 480px;
        max-height: 90vh;
        overflow: auto;
    }

    .modal-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-4);
        border-bottom: 1px solid var(--border);
    }

    .modal-header h2 {
        font-size: 18px;
        font-weight: 600;
        color: var(--fg);
    }

    .close-btn {
        padding: var(--space-1);
        color: var(--fg-muted);
        border-radius: var(--radius-sm);
    }

    .close-btn:hover {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    form {
        padding: var(--space-4);
    }

    .form-group {
        margin-bottom: var(--space-4);
    }

    .form-group label {
        display: block;
        margin-bottom: var(--space-2);
        font-size: 14px;
        font-weight: 500;
        color: var(--fg);
    }

    .form-group input,
    .form-group select {
        width: 100%;
        padding: var(--space-2) var(--space-3);
        font-size: 14px;
        color: var(--fg);
        background: var(--bg-secondary);
        border: 1px solid var(--border);
        border-radius: var(--radius-md);
    }

    .form-group input:focus,
    .form-group select:focus {
        outline: none;
        border-color: var(--accent);
        box-shadow: 0 0 0 3px var(--accent-muted);
    }

    .form-group input:disabled,
    .form-group select:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .help-text {
        margin-top: var(--space-1);
        font-size: 12px;
        color: var(--fg-muted);
    }

    .error-message {
        padding: var(--space-3);
        margin-bottom: var(--space-4);
        font-size: 14px;
        color: var(--error);
        background: var(--error-bg);
        border-radius: var(--radius-md);
    }

    .form-actions {
        display: flex;
        gap: var(--space-3);
        justify-content: flex-end;
    }

    .btn-primary,
    .btn-secondary {
        padding: var(--space-2) var(--space-4);
        font-size: 14px;
        font-weight: 500;
        border-radius: var(--radius-md);
    }

    .btn-primary {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .btn-primary:hover:not(:disabled) {
        background: var(--accent-hover);
    }

    .btn-primary:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }

    .btn-secondary {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    .btn-secondary:hover:not(:disabled) {
        background: var(--bg-hover);
    }

    .btn-secondary:disabled {
        opacity: 0.6;
        cursor: not-allowed;
    }
</style>
