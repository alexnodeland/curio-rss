/**
 * The Reddit API credentials panel (D15): the secret goes down through
 * `set_reddit_api` and never renders back; status shows only the client
 * id; removal returns to the unauthenticated state.
 */
import RedditApiPanel from '$components/modals/RedditApiPanel.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, type Responder, flushIpc, installIpcHarness } from './ipc-harness';

function base(overrides: Record<string, Responder> = {}): Record<string, Responder> {
    return {
        get_reddit_api_status: { configured: false, client_id: null },
        set_reddit_api: { configured: true, client_id: 'cid123' },
        clear_reddit_api: { configured: false, client_id: null },
        ...overrides,
    };
}

describe('RedditApiPanel', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('saves trimmed credentials and flips to the connected state', async () => {
        harness = installIpcHarness(base());
        const { getByLabelText, getByText, container } = render(RedditApiPanel);
        await flushIpc();

        expect(getByText(/Not connected/)).toBeTruthy();
        await fireEvent.input(getByLabelText('Client id'), { target: { value: '  cid123 ' } });
        await fireEvent.input(getByLabelText('Client secret'), { target: { value: ' shh ' } });
        await fireEvent.click(getByText('Save'));
        await flushIpc();

        expect(harness.callsFor('set_reddit_api')).toEqual([
            { clientId: 'cid123', clientSecret: 'shh' },
        ]);
        expect(getByText(/Connected — client id cid123/)).toBeTruthy();
        // The secret never renders anywhere after saving.
        expect(container.innerHTML).not.toContain('shh');
        expect(uiStore.toasts.at(-1)?.tone).toBe('success');
    });

    it('save stays disabled until both fields are filled', async () => {
        harness = installIpcHarness(base());
        const { getByLabelText, getByText } = render(RedditApiPanel);
        await flushIpc();

        const save = getByText('Save').closest('button') as HTMLButtonElement;
        expect(save.disabled).toBe(true);
        await fireEvent.input(getByLabelText('Client id'), { target: { value: 'cid' } });
        expect(save.disabled).toBe(true);
        await fireEvent.input(getByLabelText('Client secret'), { target: { value: 's' } });
        expect(save.disabled).toBe(false);
    });

    it('removes stored credentials and returns to the input state', async () => {
        harness = installIpcHarness(
            base({ get_reddit_api_status: { configured: true, client_id: 'cid123' } }),
        );
        const { getByText, getByLabelText } = render(RedditApiPanel);
        await flushIpc();

        expect(getByText(/Connected — client id cid123/)).toBeTruthy();
        await fireEvent.click(getByText('Remove credentials'));
        await flushIpc();

        expect(harness.callsFor('clear_reddit_api')).toHaveLength(1);
        expect(getByText(/Not connected/)).toBeTruthy();
        expect(getByLabelText('Client secret')).toBeTruthy();
    });
});
