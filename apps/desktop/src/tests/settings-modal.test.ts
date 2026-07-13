/**
 * The settings modal's tablist: WAI-ARIA roles/wiring, the General tab active
 * on open, and roving-tabindex keyboard navigation (arrow move + wraparound,
 * Home/End) with focus following the active tab. All six panels mount at once
 * (inactive ones carry `hidden`), so the harness declares the commands the
 * mounted panels fire on open. Roving-focus assertions follow menu.test.ts.
 */
import SettingsModal from '$components/modals/SettingsModal.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    appInfoFixture,
    dbStatsFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';

function mountResponders(): Record<string, Responder> {
    // The panels that prime on open: Data (destinations), Advanced
    // (doctor), and Media & Privacy (the Reddit API status).
    return {
        list_destinations: [],
        db_stats: dbStatsFixture(),
        get_app_info: appInfoFixture(),
        get_setting: null,
        set_setting: null,
        get_reddit_api_status: { configured: false, client_id: null },
    };
}

function tabs(): HTMLButtonElement[] {
    return [...document.querySelectorAll('[role="tab"]')] as HTMLButtonElement[];
}

function panels(): HTMLElement[] {
    return [...document.querySelectorAll('[role="tabpanel"]')] as HTMLElement[];
}

describe('SettingsModal — tablist', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('opens as a six-tab tablist with General selected and panels wired', async () => {
        harness = installIpcHarness(mountResponders());
        render(SettingsModal, { props: { onclose: vi.fn() } });
        await flushIpc();

        const tablist = document.querySelector('[role="tablist"]');
        expect(tablist).not.toBeNull();
        expect(tablist?.getAttribute('aria-label')).toBe('Settings sections');

        const tabEls = tabs();
        expect(tabEls).toHaveLength(7);
        expect(tabEls[0].textContent?.trim()).toBe('General');

        // General active on open: aria-selected + the single roving tab stop.
        expect(tabEls[0].getAttribute('aria-selected')).toBe('true');
        expect(tabEls[0].getAttribute('tabindex')).toBe('0');
        for (const tab of tabEls.slice(1)) {
            expect(tab.getAttribute('aria-selected')).toBe('false');
            expect(tab.getAttribute('tabindex')).toBe('-1');
        }

        // Each tab controls a real tabpanel; only the active one is visible.
        for (const tab of tabEls) {
            const panel = document.getElementById(tab.getAttribute('aria-controls') ?? '');
            expect(panel?.getAttribute('role')).toBe('tabpanel');
            expect(panel?.getAttribute('aria-labelledby')).toBe(tab.id);
        }
        const panelEls = panels();
        expect(panelEls[0].hasAttribute('hidden')).toBe(false);
        expect(panelEls.slice(1).every((panel) => panel.hasAttribute('hidden'))).toBe(true);
    });

    it('moves selection and roving focus with arrows, wraps, and Home/End jump', async () => {
        harness = installIpcHarness(mountResponders());
        render(SettingsModal, { props: { onclose: vi.fn() } });
        await flushIpc();

        const tablist = document.querySelector('[role="tablist"]') as HTMLElement;
        const tabEls = tabs();

        // Normalize to General with focus inside the tablist (the focus-follows
        // effect only steals focus once it is already on a tab), independent of
        // any tab a prior test left remembered in the session.
        tabEls[0].focus();
        await fireEvent.keyDown(tablist, { key: 'Home' });
        expect(tabEls[0].getAttribute('aria-selected')).toBe('true');
        expect(document.activeElement).toBe(tabEls[0]);

        // ArrowRight advances selection, focus, and the roving tab stop.
        await fireEvent.keyDown(tablist, { key: 'ArrowRight' });
        expect(tabEls[1].getAttribute('aria-selected')).toBe('true');
        expect(document.activeElement).toBe(tabEls[1]);
        expect(tabEls[0].getAttribute('tabindex')).toBe('-1');
        expect(tabEls[1].getAttribute('tabindex')).toBe('0');

        // ArrowLeft goes back to the first…
        await fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
        expect(document.activeElement).toBe(tabEls[0]);
        // …and once more wraps to the last.
        await fireEvent.keyDown(tablist, { key: 'ArrowLeft' });
        expect(tabEls[6].getAttribute('aria-selected')).toBe('true');
        expect(document.activeElement).toBe(tabEls[6]);
        // ArrowRight from the last wraps to the first.
        await fireEvent.keyDown(tablist, { key: 'ArrowRight' });
        expect(document.activeElement).toBe(tabEls[0]);

        // End jumps to the last, Home back to the first.
        await fireEvent.keyDown(tablist, { key: 'End' });
        expect(document.activeElement).toBe(tabEls[6]);
        await fireEvent.keyDown(tablist, { key: 'Home' });
        expect(document.activeElement).toBe(tabEls[0]);

        // Activating a tab surfaces only its panel.
        const panelEls = panels();
        expect(panelEls[0].hasAttribute('hidden')).toBe(false);
        expect(panelEls.slice(1).every((panel) => panel.hasAttribute('hidden'))).toBe(true);
    });
});
