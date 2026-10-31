<script lang="ts">
import '../app.css';
import { onMount } from 'svelte';
import RecoveryScreen from '$components/layout/RecoveryScreen.svelte';
import { localeStore, t } from '$lib/i18n';
import { wireMenuActions } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { wireInvalidation } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { runStartupUpdateCheck } from '$lib/utils/updates';
import type { Snippet } from 'svelte';

let { children }: { children: Snippet } = $props();

/**
 * `checking` until the backend says how startup went; the reader only mounts
 * on `app` (its commands need the core), the recovery screen on `recovery`.
 */
let phase: 'checking' | 'app' | 'recovery' = $state('checking');

onMount(() => {
    const teardowns: Array<() => void> = [];
    let unmounted = false;

    void (async () => {
        // The profile may not have opened: then there are no settings to
        // load, and the recovery screen speaks the system language.
        if (await startupStore.check()) {
            localeStore.initFromSystem();
            phase = 'recovery';
            return;
        }
        phase = 'app';
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

{#if phase === 'app'}
    {@render children()}
{:else if phase === 'recovery' && startupStore.failure !== null}
    <!-- After a successful recovery the core is running: reload into the reader. -->
    <RecoveryScreen failure={startupStore.failure} onopen={() => window.location.reload()} />
{/if}
