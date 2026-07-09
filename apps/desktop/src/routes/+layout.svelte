<script lang="ts">
import '../app.css';
import { onMount } from 'svelte';
import { localeStore, t } from '$lib/i18n';
import { wireMenuActions } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { wireInvalidation } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { runStartupUpdateCheck } from '$lib/utils/updates';
import type { Snippet } from 'svelte';

let { children }: { children: Snippet } = $props();

onMount(() => {
    const teardowns: Array<() => void> = [];
    let unmounted = false;

    void (async () => {
        // Settings first: the persisted theme wins over the preload's
        // localStorage mirror once the backend answers.
        await settingsStore.load();
        localeStore.init();
        // Custom themes before initTheme: a custom preference must resolve
        // against a populated list and have its rule injected pre-paint.
        uiStore.initCustomThemes();
        uiStore.initTheme();
        uiStore.initLayout();
        uiStore.initTypography();
        uiStore.initReading();
        uiStore.initRefresh();
        uiStore.initNotifications();
        uiStore.initUpdates();
        feedsStore.initSidebarState();

        // Event-driven invalidation: the query cache and the refresh
        // progress fields subscribe to the Rust-emitted specta events.
        const unsubscribers = await Promise.all([
            wireInvalidation(),
            feedsStore.wireRefreshEvents(),
            wireMenuActions(),
        ]);
        if (unmounted) {
            for (const unsubscribe of unsubscribers) {
                unsubscribe();
            }
            return;
        }
        teardowns.push(...unsubscribers);

        // Auto-update: check the release feed once (best-effort, off in the dev
        // browser). Auto-install relaunches; otherwise a toast points at Settings.
        void runStartupUpdateCheck({
            autoCheck: uiStore.updatesAutoCheck,
            autoInstall: uiStore.updatesAutoInstall,
            onAvailable: (version) =>
                uiStore.showToast(t('updates.available', { version }), 'info'),
        });
    })();

    return () => {
        unmounted = true;
        for (const teardown of teardowns) {
            teardown();
        }
    };
});
</script>

{@render children()}
