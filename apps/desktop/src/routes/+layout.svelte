<script lang="ts">
import '../app.css';
import { onMount } from 'svelte';
import { localeStore, t } from '$lib/i18n';
import { wireMenuActions } from '$lib/state/actions';
import { feedsStore } from '$lib/state/feeds.svelte';
import { wireInvalidation } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { themeStore } from '$lib/state/theme.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { typographyStore } from '$lib/state/typography.svelte';
import { announceLegacyProfiles } from '$lib/utils/legacy-profiles';
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
        themeStore.initCustomThemes();
        themeStore.initTheme();
        layoutStore.initLayout();
        typographyStore.initTypography();
        prefsStore.initReading();
        prefsStore.initRefresh();
        prefsStore.initNotifications();
        prefsStore.initUpdates();
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

{@render children()}
