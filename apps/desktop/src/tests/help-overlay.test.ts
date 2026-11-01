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
        expect(getAllByText('g').length).toBe(5);
        expect(getAllByText('then').length).toBe(5);
    });

    it('lists the reading keys: pane keys beside their global key', () => {
        const { container } = render(HelpOverlay, { props: { onclose: vi.fn() } });
        const rowKeys = (description: string): string[] => {
            const row = [...container.querySelectorAll('li')].find(
                (item) => item.querySelector('.description')?.textContent === description,
            );
            return [...(row?.querySelectorAll('kbd') ?? [])].map((kbd) => kbd.textContent ?? '');
        };
        expect(rowKeys('Next article')).toEqual(['j', '↓']);
        expect(rowKeys('Previous article')).toEqual(['k', '↑']);
        expect(rowKeys('Read the selected article (focus the reader)')).toEqual(['Enter', '→']);
        expect(rowKeys('Back to the article list')).toEqual(['←', 'Esc']);
        expect(rowKeys('Page down; at the end, next unread')).toEqual(['Space']);
        expect(rowKeys('Page up')).toEqual(['⇧ Space']);
        expect(rowKeys('Archive / unarchive article')).toEqual(['e']);
        expect(rowKeys('Mark all read in the current view')).toEqual(['A']);
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
