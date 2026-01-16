<script lang="ts">
import '../app.css';
import { loadArticles, loadFeeds, loadFolderTree } from '$lib/stores/feeds';
import { setTheme, currentTheme } from '$lib/stores/ui';
import { onMount } from 'svelte';

let initError = '';

onMount(async () => {
    // Apply saved theme
    setTheme($currentTheme);

    // Load initial data
    try {
        await Promise.all([loadFeeds(), loadFolderTree()]);
        await loadArticles();
    } catch (e) {
        initError = e instanceof Error ? e.message : 'Failed to initialize';
        console.error('Initialization error:', e);
    }
});
</script>

{#if initError}
    <div class="init-error">
        <h1>Curio Reader</h1>
        <p>{initError}</p>
        <p class="hint">Make sure you're running the app via <code>make dev</code> or the built application.</p>
    </div>
{:else}
    <slot />
{/if}

<style>
    .init-error {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        height: 100vh;
        text-align: center;
        padding: 2rem;
        font-family: system-ui, sans-serif;
    }
    .init-error h1 {
        font-size: 1.5rem;
        margin-bottom: 1rem;
    }
    .init-error p {
        color: #666;
        margin: 0.5rem 0;
    }
    .init-error .hint {
        font-size: 0.875rem;
    }
    .init-error code {
        background: #f0f0f0;
        padding: 0.2em 0.4em;
        border-radius: 3px;
    }
</style>
