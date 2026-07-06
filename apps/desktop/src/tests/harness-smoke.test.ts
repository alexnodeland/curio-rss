/**
 * Stack smoke: @testing-library/svelte renders a Svelte 5 RUNES component
 * under jsdom and reactivity flows on interaction — the foundation the WP3
 * component tests build on.
 */
import { fireEvent, render, screen } from '@testing-library/svelte';
import { describe, expect, it } from 'vitest';
import RunesProbe from './fixtures/RunesProbe.svelte';

describe('component test stack', () => {
    it('renders a runes component and reacts to events', async () => {
        render(RunesProbe, { label: 'clicks' });

        const button = screen.getByRole('button');
        expect(button.textContent).toContain('clicks: 0 (doubled 0)');

        await fireEvent.click(button);
        await fireEvent.click(button);
        expect(button.textContent).toContain('clicks: 2 (doubled 4)');
    });
});
