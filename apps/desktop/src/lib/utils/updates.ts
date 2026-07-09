/**
 * The app self-updater over the Tauri updater + process plugins. Uses the
 * plugin JS APIs directly — the same pattern as the opener (allowed by the
 * frontend-bans, which forbid only raw `@tauri-apps/api/core` invoke). Every
 * call is a no-op outside the packaged app: the plugins throw without a Tauri
 * runtime, so callers catch and degrade — the dev browser and tests never
 * reach a real updater. Updates are minisign-verified against the pubkey in
 * `tauri.conf.json` (Apple notarization is separate and intentionally skipped).
 */
import { relaunch } from '@tauri-apps/plugin-process';
import { type DownloadEvent, type Update, check } from '@tauri-apps/plugin-updater';

export type { Update };

/** Checks the release feed; resolves the available `Update`, or `null` if current. */
export function checkForUpdate(): Promise<Update | null> {
    return check();
}

/** Downloaded fraction in `0..1`; `0` while the total length is unknown. */
export function updateFraction(downloaded: number, total: number): number {
    return total > 0 ? Math.min(1, downloaded / total) : 0;
}

/**
 * Downloads + installs `update`, reporting `0..1` progress, then relaunches
 * into the new version (macOS/Linux need the explicit relaunch).
 */
export async function installAndRelaunch(
    update: Update,
    onProgress?: (fraction: number) => void,
): Promise<void> {
    let total = 0;
    let downloaded = 0;
    await update.downloadAndInstall((event: DownloadEvent) => {
        if (event.event === 'Started') {
            total = event.data.contentLength ?? 0;
        } else if (event.event === 'Progress') {
            downloaded += event.data.chunkLength;
            onProgress?.(updateFraction(downloaded, total));
        }
    });
    await relaunch();
}

/**
 * Startup auto-update: when enabled, checks once and either installs silently
 * (auto-install) or hands the caller the available version to surface. Every
 * failure is swallowed — a bad check must never block launch (no runtime,
 * offline, an unpublished release whose endpoint 404s, …).
 */
export async function runStartupUpdateCheck(opts: {
    autoCheck: boolean;
    autoInstall: boolean;
    onAvailable: (version: string) => void;
}): Promise<void> {
    if (!opts.autoCheck) {
        return;
    }
    try {
        const update = await checkForUpdate();
        if (update === null) {
            return;
        }
        if (opts.autoInstall) {
            await installAndRelaunch(update);
        } else {
            opts.onAvailable(update.version);
        }
    } catch {
        // No Tauri runtime / offline / endpoint not yet published — stay quiet.
    }
}
