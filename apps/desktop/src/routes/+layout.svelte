<script lang="ts">
import '../app.css';
import { onMount } from 'svelte';
import RecoveryScreen from '$components/layout/RecoveryScreen.svelte';
import { localeStore, t } from '$lib/i18n';
import { wireMenuActions } from '$lib/state/actions';
import { connectivityStore } from '$lib/state/connectivity.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { wireInvalidation } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import { themeStore } from '$lib/state/theme.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { typographyStore } from '$lib/state/typography.svelte';
import { announceLegacyProfiles } from '$lib/utils/legacy-profiles';
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
        themeStore.initCustomThemes();
        themeStore.initTheme();
        layoutStore.initLayout();
        typographyStore.initTypography();
        prefsStore.initReading();
        prefsStore.initRefresh();
        prefsStore.initNotifications();
        prefsStore.initUpdates();
        feedsStore.initSidebarState();

        // Event-driven invalidation: the query cache, the refresh progress
        // fields and the offline badge subscribe to the Rust-emitted specta
        // events.
        const unsubscribers = await Promise.all([
            wireInvalidation(),
            feedsStore.wireRefreshEvents(),
            connectivityStore.wire(),
            wireMenuActions(),
        ]);
        if (unmounted) {
            for (const unsubscribe of unsubscribers) {
                unsubscribe();
            }
            return;
        }
        teardowns.push(...unsubscribers);

        // A library from an earlier release that startup moved, or had to
        // leave where it is: say so once (the doctor panel keeps the why).
        void announceLegacyProfiles();

        // Auto-update: check the release feed once (best-effort, off in the dev
        // browser). Auto-install relaunches; otherwise a toast points at Settings.
        void runStartupUpdateCheck({
            autoCheck: prefsStore.updatesAutoCheck,
            autoInstall: prefsStore.updatesAutoInstall,
            onAvailable: (version) =>
                toastStore.showToast(t('updates.available', { version }), 'info'),
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
