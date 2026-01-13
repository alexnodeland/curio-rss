<script lang="ts">
import '../app.css';
import { loadArticles, loadFeeds, loadFolderTree } from '$lib/stores/feeds';
import { setTheme } from '$lib/stores/ui';
import { onMount } from 'svelte';

onMount(async () => {
    // Apply saved theme
    setTheme($currentTheme);

    // Load initial data
    await Promise.all([loadFeeds(), loadFolderTree()]);
    await loadArticles();
});
</script>

<slot />
