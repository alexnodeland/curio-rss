/**
 * The startup recovery screen: when the profile would not open, the layout
 * shows it instead of the reader. Covers the plan per failure class, the
 * plain-text report, the store's command round trips, the screen's confirm
 * flow and the recovered state, and the system-language fallback.
 */
import RecoveryScreen from '$components/layout/RecoveryScreen.svelte';
import { localeStore, t } from '$lib/i18n';
import { matchLocale } from '$lib/i18n/locale.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import { failureReport, fileName, recoveryCopy, recoveryPlan } from '$lib/utils/recovery';
import { cleanup, fireEvent, render, screen } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    recoveryOutcomeFixture,
    rejectWith,
    startupFailureFixture,
} from './ipc-harness';

const updater = vi.hoisted(() => ({
    checkForUpdate: vi.fn(),
    installAndRelaunch: vi.fn(),
}));
vi.mock('$lib/utils/updates', () => updater);

describe('recoveryPlan', () => {
    it('leads with the backup for a corrupt database that has one', () => {
        expect(recoveryPlan(startupFailureFixture())).toEqual({
            primary: 'restore',
            secondary: ['fresh'],
        });
        expect(recoveryPlan(startupFailureFixture({ backup: null }))).toEqual({
            primary: 'fresh',
            secondary: [],
        });
    });

    it('only retries a locked database — it is never moved aside', () => {
        expect(recoveryPlan(startupFailureFixture({ class: 'locked' }))).toEqual({
            primary: 'retry',
            secondary: [],
        });
    });

    it('leads with updating Curio for a too-new database — lossless, unlike the backup', () => {
        expect(recoveryPlan(startupFailureFixture({ class: 'too_new', backup: null }))).toEqual({
            primary: 'update',
            secondary: ['fresh'],
        });
        expect(recoveryPlan(startupFailureFixture({ class: 'too_new' }))).toEqual({
            primary: 'update',
            secondary: ['restore', 'fresh'],
        });
    });

    it('only retries when the database is not what failed', () => {
        expect(recoveryPlan(startupFailureFixture({ class: 'profile', backup: null }))).toEqual({
            primary: 'retry',
            secondary: [],
        });
        // Even a stray backup is not offered: replacing the database cannot help.
        expect(recoveryPlan(startupFailureFixture({ class: 'profile' })).secondary).toEqual([]);
    });

    it('only retries a profile on network storage — the database is healthy', () => {
        expect(recoveryPlan(startupFailureFixture({ class: 'network_fs', backup: null }))).toEqual({
            primary: 'retry',
            secondary: [],
        });
        expect(recoveryPlan(startupFailureFixture({ class: 'network_fs' }))).toEqual({
            primary: 'retry',
            secondary: [],
        });
    });

    it('retries first where the cause can clear by itself', () => {
        for (const failureClass of ['io', 'upgrade', 'other'] as const) {
            expect(recoveryPlan(startupFailureFixture({ class: failureClass }))).toEqual({
                primary: 'retry',
                secondary: ['restore', 'fresh'],
            });
        }
    });

    it('offers no action without a profile', () => {
        expect(
            recoveryPlan(
                startupFailureFixture({ class: 'no_profile', profile_dir: null, backup: null }),
            ),
        ).toEqual({ primary: null, secondary: [] });
    });
});

describe('recovery copy and report', () => {
    it('maps every class to its own title and body', () => {
        const keys = (
            [
                'corrupt',
                'too_new',
                'locked',
                'io',
                'profile',
                'network_fs',
                'upgrade',
                'no_profile',
                'other',
            ] as const
        ).map((failureClass) => recoveryCopy(failureClass));
        expect(new Set(keys.map((k) => k.title)).size).toBe(9);
        expect(new Set(keys.map((k) => k.body)).size).toBe(9);
    });

    it('names a backup by its file name on either separator', () => {
        expect(fileName('/p/curio.db.pre-v7.bak')).toBe('curio.db.pre-v7.bak');
        expect(fileName('C:\\p\\curio.db.pre-v7.bak')).toBe('curio.db.pre-v7.bak');
    });

    it('builds a plain-text report with the class, message and paths', () => {
        const report = failureReport(startupFailureFixture());
        expect(report.split('\n')[0]).toBe('Curio 0.4.0 could not start (corrupt)');
        expect(report).toContain('file is not a database');
        expect(report).toContain('Profile: /home/user/.local/share/curio');
        expect(report).toContain('Backup: /home/user/.local/share/curio/curio.db.pre-v7.bak');
        const bare = failureReport(
            startupFailureFixture({ class: 'no_profile', profile_dir: null, backup: null }),
        );
        expect(bare).not.toContain('Profile:');
        expect(bare.endsWith('\n')).toBe(false);
    });
});

describe('matchLocale', () => {
    afterEach(() => {
        localeStore.reset();
    });

    it('matches exact ids, then base languages, then gives up', () => {
        expect(matchLocale(['fr'])).toBe('fr');
        expect(matchLocale(['de-AT', 'en'])).toBe('de');
        expect(matchLocale(['zh-CN'])).toBe('zh-Hans');
        expect(matchLocale(['ZH-HANS'])).toBe('zh-Hans');
        expect(matchLocale(['ja', 'pl-PL'])).toBe('pl');
        // Traditional script goes to the Traditional-script catalog, never Simplified.
        for (const tag of ['zh-HK', 'zh-MO', 'zh-TW', 'zh-Hant', 'zh-Hant-HK', 'yue-HK']) {
            expect(matchLocale([tag])).toBe('yue');
        }
        expect(matchLocale(['zh-Hans-HK'])).toBe('zh-Hans');
        expect(matchLocale(['zh-SG'])).toBe('zh-Hans');
        expect(matchLocale(['ja'])).toBeNull();
        expect(matchLocale([])).toBeNull();
    });

    it('adopts the system language without persisting it', () => {
        localeStore.initFromSystem(['es-MX']);
        expect(localeStore.active).toBe('es');
        localeStore.initFromSystem(['ja']);
        expect(localeStore.active).toBe('es');
    });
});

describe('startupStore', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        startupStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('reports a normal start as no failure', async () => {
        harness = installIpcHarness({ get_startup_failure: null });
        expect(await startupStore.check()).toBe(false);
        expect(startupStore.failure).toBeNull();
    });

    it('holds the failure the backend reports', async () => {
        harness = installIpcHarness({ get_startup_failure: startupFailureFixture() });
        expect(await startupStore.check()).toBe(true);
        expect(startupStore.failure?.class).toBe('corrupt');
    });

    it('falls through to the reader when the check itself fails', async () => {
        harness = installIpcHarness({
            get_startup_failure: rejectWith(commandErrorFixture()),
        });
        expect(await startupStore.check()).toBe(false);
        harness.teardown();
        harness = installIpcHarness({});
        expect(await startupStore.check()).toBe(false);
    });

    it('marks a successful recovery with where the old database went', async () => {
        harness = installIpcHarness({ start_fresh_profile: recoveryOutcomeFixture() });
        startupStore.failure = startupFailureFixture();
        await startupStore.run('fresh');
        expect(startupStore.recovered).toEqual({
            setAside: '/home/user/.local/share/curio/set-aside/20261031T120000Z',
        });
        expect(startupStore.busy).toBeNull();
        expect(harness.calls.map((c) => c.cmd)).toEqual(['start_fresh_profile']);
    });

    it('keeps the screen with the new failure when recovery did not help', async () => {
        const next = startupFailureFixture({ class: 'io', backup: null });
        harness = installIpcHarness({
            restore_startup_backup: recoveryOutcomeFixture({ failure: next }),
        });
        startupStore.failure = startupFailureFixture();
        await startupStore.run('restore');
        expect(startupStore.recovered).toBeNull();
        expect(startupStore.failure).toEqual(next);
        expect(startupStore.notice?.kind).toBe('error');
        expect(startupStore.notice?.text).toContain(t('recovery.stillFailing'));
        expect(startupStore.notice?.text).toContain('set-aside/20261031T120000Z');
    });

    it('surfaces a refused action and ignores clicks while one runs', async () => {
        const refusal = commandErrorFixture({
            kind: 'user',
            code: 'STORAGE',
            message: 'the database is in use by another copy of Curio; quit it and try again',
            recoverable: true,
        });
        harness = installIpcHarness({ retry_startup: rejectWith(refusal) });
        startupStore.busy = 'fresh';
        await startupStore.run('retry');
        expect(harness.calls).toHaveLength(0);
        startupStore.busy = null;
        await startupStore.run('retry');
        expect(startupStore.notice).toEqual({ kind: 'error', text: refusal.message });
    });

    it('reports an unexpected IPC failure generically', async () => {
        harness = installIpcHarness({});
        await startupStore.run('retry');
        expect(startupStore.notice).toEqual({ kind: 'error', text: t('app.error.internal') });
        await startupStore.quit();
        expect(startupStore.notice?.text).toBe(t('app.error.internal'));
    });

    it('reveals, quits, and reports a refused reveal', async () => {
        harness = installIpcHarness({ reveal_profile_folder: null, quit_app: null });
        await startupStore.reveal();
        await startupStore.quit();
        expect(harness.calls.map((c) => c.cmd)).toEqual(['reveal_profile_folder', 'quit_app']);
        expect(startupStore.notice).toBeNull();
        harness.respond(
            'reveal_profile_folder',
            rejectWith(commandErrorFixture({ kind: 'user', message: 'no such folder' })),
        );
        await startupStore.reveal();
        expect(startupStore.notice).toEqual({ kind: 'error', text: 'no such folder' });
    });

    it('installs a newer Curio for a too-new database, or says none is available', async () => {
        const update = { version: '0.5.0' };
        updater.checkForUpdate.mockResolvedValueOnce(update);
        updater.installAndRelaunch.mockResolvedValueOnce(undefined);
        await startupStore.run('update');
        expect(updater.installAndRelaunch).toHaveBeenCalledWith(update);
        expect(startupStore.busy).toBeNull();

        updater.checkForUpdate.mockResolvedValueOnce(null);
        await startupStore.run('update');
        expect(startupStore.notice).toEqual({ kind: 'info', text: t('recovery.update.current') });

        // Outside the packaged app (or offline) the updater throws.
        updater.checkForUpdate.mockRejectedValueOnce(new Error('no runtime'));
        await startupStore.run('update');
        expect(startupStore.notice).toEqual({ kind: 'error', text: t('recovery.update.failed') });
        expect(startupStore.busy).toBeNull();
    });

    it('copies the report, and says so when the clipboard refuses', async () => {
        const writeText = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
        await startupStore.copyReport();
        expect(writeText).not.toHaveBeenCalled();

        startupStore.failure = startupFailureFixture();
        await startupStore.copyReport();
        expect(writeText).toHaveBeenCalledWith(failureReport(startupFailureFixture()));
        expect(startupStore.notice).toEqual({ kind: 'info', text: t('recovery.copied') });

        writeText.mockRejectedValueOnce(new Error('denied'));
        await startupStore.copyReport();
        expect(startupStore.notice).toEqual({ kind: 'error', text: t('recovery.copyFailed') });
    });
});

describe('RecoveryScreen', () => {
    let harness: IpcHarness | null = null;

    beforeEach(() => {
        startupStore.failure = startupFailureFixture();
    });

    afterEach(() => {
        cleanup();
        startupStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('explains the failure, names the backup and shows the error details', () => {
        render(RecoveryScreen, { failure: startupFailureFixture(), onopen: () => {} });
        expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(
            t('recovery.title.corrupt'),
        );
        expect(screen.getByText('curio.db.pre-v7.bak')).toBeTruthy();
        expect(screen.getByText(t('recovery.safety'))).toBeTruthy();
        expect(screen.getByText(/file is not a database/)).toBeTruthy();
        expect(screen.getByRole('button', { name: t('recovery.action.reveal') })).toBeTruthy();
    });

    it('asks before restoring, and cancel runs nothing', async () => {
        harness = installIpcHarness({ restore_startup_backup: recoveryOutcomeFixture() });
        render(RecoveryScreen, { failure: startupFailureFixture(), onopen: () => {} });
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.restore') }));
        expect(screen.getByText(t('recovery.confirm.restore'))).toBeTruthy();
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.cancel') }));
        expect(screen.queryByText(t('recovery.confirm.restore'))).toBeNull();
        expect(harness.calls).toHaveLength(0);
    });

    it('confirms, recovers, and opens Curio', async () => {
        harness = installIpcHarness({ restore_startup_backup: recoveryOutcomeFixture() });
        const onopen = vi.fn();
        render(RecoveryScreen, { failure: startupFailureFixture(), onopen });
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.restore') }));
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.restore') }));
        await flushIpc();
        expect(harness.calls.map((c) => c.cmd)).toEqual(['restore_startup_backup']);
        expect(screen.getByRole('heading', { level: 1 }).textContent).toBe(
            t('recovery.recovered.title'),
        );
        expect(
            screen.getByText('/home/user/.local/share/curio/set-aside/20261031T120000Z'),
        ).toBeTruthy();
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.open') }));
        expect(onopen).toHaveBeenCalledOnce();
    });

    it('retries a locked database without confirmation and offers no replacement', async () => {
        const locked = startupFailureFixture({ class: 'locked', backup: null });
        harness = installIpcHarness({
            retry_startup: recoveryOutcomeFixture({ failure: locked, set_aside: null }),
        });
        render(RecoveryScreen, { failure: locked, onopen: () => {} });
        expect(screen.queryByRole('button', { name: t('recovery.action.fresh') })).toBeNull();
        expect(screen.queryByText(t('recovery.safety'))).toBeNull();
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.retry') }));
        await flushIpc();
        expect(harness.calls.map((c) => c.cmd)).toEqual(['retry_startup']);
        expect(screen.getByRole('status').textContent).toContain(t('recovery.stillFailing'));
    });

    it('moves focus into the confirmation, back on cancel, and onto Open Curio', async () => {
        harness = installIpcHarness({ start_fresh_profile: recoveryOutcomeFixture() });
        render(RecoveryScreen, {
            failure: startupFailureFixture({ backup: null }),
            onopen: () => {},
        });
        const fresh = screen.getByRole('button', { name: t('recovery.action.fresh') });
        fresh.focus();
        await fireEvent.click(fresh);
        await tick();
        const confirmButton = screen.getByRole('button', { name: t('recovery.action.fresh') });
        expect(document.activeElement).toBe(confirmButton);
        expect(confirmButton.getAttribute('aria-describedby')).toBe('recovery-confirm');

        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.cancel') }));
        await tick();
        expect(document.activeElement).toBe(
            screen.getByRole('button', { name: t('recovery.action.fresh') }),
        );

        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.fresh') }));
        await tick();
        await fireEvent.click(screen.getByRole('button', { name: t('recovery.action.fresh') }));
        await flushIpc();
        await tick();
        expect(document.activeElement).toBe(
            screen.getByRole('button', { name: t('recovery.action.open') }),
        );
    });

    it('keeps the running action focused and every button inert until it ends', async () => {
        let finish: (value: unknown) => void = () => {};
        harness = installIpcHarness({
            retry_startup: () =>
                new Promise((resolve) => {
                    finish = resolve;
                }),
            quit_app: null,
        });
        const locked = startupFailureFixture({ class: 'locked', backup: null });
        render(RecoveryScreen, { failure: locked, onopen: () => {} });
        const retry = screen.getByRole('button', { name: t('recovery.action.retry') });
        retry.focus();
        await fireEvent.click(retry);
        await tick();
        const working = screen.getByRole('button', { name: t('recovery.working') });
        expect(document.activeElement).toBe(working);
        expect(working.getAttribute('aria-disabled')).toBe('true');
        const quit = screen.getByRole('button', { name: t('recovery.action.quit') });
        expect(quit.getAttribute('aria-disabled')).toBe('true');
        await fireEvent.click(quit);
        await fireEvent.click(working);
        expect(harness.calls.map((c) => c.cmd)).toEqual(['retry_startup']);

        finish(recoveryOutcomeFixture({ failure: locked, set_aside: null }));
        await flushIpc();
        await tick();
        expect(quit.getAttribute('aria-disabled')).toBe('false');
        await fireEvent.click(quit);
        await flushIpc();
        expect(harness.calls.map((c) => c.cmd)).toEqual(['retry_startup', 'quit_app']);
    });

    it('offers Check for updates first for a too-new database', async () => {
        updater.checkForUpdate.mockResolvedValueOnce(null);
        render(RecoveryScreen, {
            failure: startupFailureFixture({ class: 'too_new', backup: null }),
            onopen: () => {},
        });
        const update = screen.getByRole('button', { name: t('recovery.action.update') });
        expect(update.classList.contains('btn-primary')).toBe(true);
        await fireEvent.click(update);
        await flushIpc();
        expect(screen.getByRole('status').textContent).toBe(t('recovery.update.current'));
    });

    it('hides the profile actions when there is no profile', () => {
        render(RecoveryScreen, {
            failure: startupFailureFixture({
                class: 'no_profile',
                profile_dir: null,
                backup: null,
            }),
            onopen: () => {},
        });
        expect(screen.queryByRole('button', { name: t('recovery.action.reveal') })).toBeNull();
        expect(screen.getByRole('button', { name: t('recovery.action.copy') })).toBeTruthy();
        expect(screen.getByRole('button', { name: t('recovery.action.quit') })).toBeTruthy();
    });
});
