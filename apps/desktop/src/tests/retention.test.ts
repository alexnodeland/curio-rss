/**
 * Article retention (#137): the Settings → Data controls that edit the
 * policy `curio retention` edits, and the pure helpers behind them and the
 * doctor's maintenance toasts.
 */
import RetentionPanel from '$components/modals/RetentionPanel.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import {
    RETENTION_AGE_DAYS,
    limitChoices,
    maintenanceToasts,
    parseLimit,
} from '$lib/utils/maintenance';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    maintenanceReportFixture,
    rejectWith,
    retentionPolicyFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('retention helpers', () => {
    it('offers the presets, plus a custom limit set from the command line', () => {
        expect(limitChoices(RETENTION_AGE_DAYS, null)).toEqual([30, 90, 180, 365]);
        expect(limitChoices(RETENTION_AGE_DAYS, 90)).toEqual([30, 90, 180, 365]);
        expect(limitChoices(RETENTION_AGE_DAYS, 45)).toEqual([30, 45, 90, 180, 365]);
        expect(limitChoices([100], 0)).toEqual([100]);
    });

    it('reads the "off" option and junk as no limit', () => {
        expect(parseLimit('')).toBeNull();
        expect(parseLimit('0')).toBeNull();
        expect(parseLimit('-5')).toBeNull();
        expect(parseLimit('2.5')).toBeNull();
        expect(parseLimit('abc')).toBeNull();
        expect(parseLimit('90')).toBe(90);
    });

    it('summarises a routine pass', () => {
        const report = maintenanceReportFixture({
            articles_reaped: 3,
            tags_dropped: 1,
            event_files_removed: 2,
        });
        expect(maintenanceToasts(report, false)).toEqual([
            {
                message:
                    'Maintenance finished: removed 3 articles, 1 unused tag and 2 old log files',
                tone: 'success',
            },
        ]);
    });

    it('reports an orphan purge by what it purged', () => {
        const report = maintenanceReportFixture({ orphans_purged: 1 });
        expect(maintenanceToasts(report, true)).toEqual([
            { message: 'Deleted 1 article of removed feeds', tone: 'success' },
        ]);
    });

    it('warns when nothing could be deleted, and when upkeep is left over', () => {
        const report = maintenanceReportFixture({
            reap_skipped: 'manifest unreadable',
            upkeep_skipped: 'database busy',
        });
        const toasts = maintenanceToasts(report, true);
        expect(toasts.map((toast) => toast.tone)).toEqual(['warning', 'warning']);
        expect(toasts[0]?.message).toContain('manifest unreadable');
        expect(toasts[1]?.message).toBe(
            'Some database upkeep was skipped; it runs again next time.',
        );
    });
});

describe('RetentionPanel', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    function base(overrides: Record<string, Responder> = {}): Record<string, Responder> {
        return {
            retention_policy: retentionPolicyFixture(),
            set_retention_policy: (args: Record<string, unknown>) => args.policy,
            ...overrides,
        };
    }

    function select(label: string, container: HTMLElement): HTMLSelectElement {
        const field = [...container.querySelectorAll('label.field')].find((node) =>
            node.textContent?.includes(label),
        );
        return field?.querySelector('select') as HTMLSelectElement;
    }

    it('shows the stored policy: off by default, keeping unread', async () => {
        harness = installIpcHarness(base());
        const { container, getByLabelText, getByText } = render(RetentionPanel);
        await flushIpc();

        expect(getByText('Article retention')).toBeTruthy();
        expect(select('Delete articles older than', container).value).toBe('');
        expect(select('Keep per feed', container).value).toBe('');
        expect((getByLabelText(/Keep unread articles/) as HTMLInputElement).checked).toBe(true);
    });

    it('shows a custom limit set with `curio retention` instead of snapping it', async () => {
        harness = installIpcHarness(
            base({ retention_policy: retentionPolicyFixture({ max_per_feed: 42 }) }),
        );
        const { container } = render(RetentionPanel);
        await flushIpc();

        const perFeed = select('Keep per feed', container);
        expect(perFeed.value).toBe('42');
        expect(perFeed.selectedOptions[0]?.textContent).toBe('Newest 42');
    });

    it('stores each change at once', async () => {
        harness = installIpcHarness(base());
        const { container, getByLabelText } = render(RetentionPanel);
        await flushIpc();

        await fireEvent.change(select('Delete articles older than', container), {
            target: { value: '90' },
        });
        await flushIpc();
        await fireEvent.change(select('Keep per feed', container), {
            target: { value: '500' },
        });
        await flushIpc();
        await fireEvent.click(getByLabelText(/Keep unread articles/));
        await flushIpc();

        expect(harness.callsFor('set_retention_policy')).toEqual([
            { policy: { max_age_days: 90, max_per_feed: null, keep_unread: true } },
            { policy: { max_age_days: 90, max_per_feed: 500, keep_unread: true } },
            { policy: { max_age_days: 90, max_per_feed: 500, keep_unread: false } },
        ]);
    });

    it('turns a limit off again', async () => {
        harness = installIpcHarness(
            base({ retention_policy: retentionPolicyFixture({ max_age_days: 30 }) }),
        );
        const { container } = render(RetentionPanel);
        await flushIpc();

        await fireEvent.change(select('Delete articles older than', container), {
            target: { value: '' },
        });
        await flushIpc();

        expect(harness.callsFor('set_retention_policy')).toEqual([
            { policy: { max_age_days: null, max_per_feed: null, keep_unread: true } },
        ]);
    });

    it('snaps back and explains when the change cannot be stored', async () => {
        harness = installIpcHarness(
            base({
                set_retention_policy: rejectWith(
                    commandErrorFixture({ kind: 'internal', code: 'INTERNAL' }),
                ),
            }),
        );
        const { container } = render(RetentionPanel);
        await flushIpc();

        const age = select('Delete articles older than', container);
        await fireEvent.change(age, { target: { value: '365' } });
        await flushIpc();

        expect(age.value).toBe('');
        expect(toastStore.toasts.at(-1)?.tone).toBe('error');
    });
});
