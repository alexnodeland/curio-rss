<script lang="ts">
    import '../app.css';
    import { onMount } from 'svelte';
    import { loadFeeds, loadFolderTree, loadArticles } from '$lib/stores/feeds';
    import { setTheme, currentTheme } from '$lib/stores/ui';

    onMount(async () => {
        // Apply saved theme
        setTheme($currentTheme);

        // Load initial data
        await Promise.all([loadFeeds(), loadFolderTree()]);
        await loadArticles();
    });
</script>

<slot />
