/**
 * The help overlay renders the shortcut registry verbatim — every entry,
 * grouped, with its keys — so the overlay can never drift from the map.
 */
import HelpOverlay from '$components/modals/HelpOverlay.svelte';
import { t } from '$lib/i18n';
import { SHORTCUTS } from '$lib/keyboard/registry';
import { fireEvent, render } from '@testing-library/svelte';
import { describe, expect, it, vi } from 'vitest';

describe('HelpOverlay', () => {
    it('is a modal dialog listing every registered shortcut', () => {
        const onclose = vi.fn();
        const { getByRole, getByText, getAllByText } = render(HelpOverlay, {
            props: { onclose },
        });

        const dialog = getByRole('dialog');
        expect(dialog.getAttribute('aria-modal')).toBe('true');
        expect(getByText('Keyboard shortcuts')).toBeTruthy();

        for (const shortcut of SHORTCUTS) {
            expect(getByText(t(shortcut.description))).toBeTruthy();
        }
        // Chords render both keys (g appears once per g-chord).
        expect(getAllByText('g').length).toBe(4);
        expect(getAllByText('then').length).toBe(4);
    });

    it('renders the category headings in registry order', () => {
        const { container } = render(HelpOverlay, { props: { onclose: vi.fn() } });
        const headings = [...container.querySelectorAll('.shortcut-group h3')].map(
            (heading) => heading.textContent,
        );
        expect(headings).toEqual(['Navigation', 'Actions', 'Views', 'Application']);
    });

    it('the close button calls onclose', async () => {
        const onclose = vi.fn();
        const { getByLabelText } = render(HelpOverlay, { props: { onclose } });
        await fireEvent.click(getByLabelText('Close'));
        expect(onclose).toHaveBeenCalledOnce();
    });
});
