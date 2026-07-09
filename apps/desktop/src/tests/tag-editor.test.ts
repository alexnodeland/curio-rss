/**
 * The chip tag editor: existing tags render as removable pills; the input
 * adds new ones (Enter or comma), and Backspace on an empty field removes the
 * last. Every mutation emits the next tag list through `onchange` — the
 * component never persists.
 */
import TagEditor from '$components/common/TagEditor.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

afterEach(cleanup);

function mount(tags: string[]) {
    const onchange = vi.fn();
    const utils = render(TagEditor, { tags, onchange, label: 'Folders & tags' });
    const input = utils.getByPlaceholderText('Add a tag…') as HTMLInputElement;
    return { ...utils, onchange, input };
}

describe('TagEditor', () => {
    it('renders each tag as a chip', () => {
        const { getByText } = mount(['Tech', 'Tech/Databases']);
        expect(getByText('Tech')).toBeTruthy();
        expect(getByText('Tech/Databases')).toBeTruthy();
    });

    it('adds a tag on Enter, appending to the list', async () => {
        const { input, onchange } = mount(['Tech']);
        await fireEvent.input(input, { target: { value: 'News' } });
        await fireEvent.keyDown(input, { key: 'Enter' });
        expect(onchange).toHaveBeenCalledWith(['Tech', 'News']);
    });

    it('adds on comma and splits a pasted comma list, dropping duplicates', async () => {
        const { input, onchange } = mount(['Tech']);
        await fireEvent.input(input, { target: { value: 'News, Tech, Sports' } });
        await fireEvent.keyDown(input, { key: ',' });
        // Existing "Tech" is dropped; the two new ones are appended.
        expect(onchange).toHaveBeenCalledWith(['Tech', 'News', 'Sports']);
    });

    it('removes a tag when its × is clicked', async () => {
        const { getByLabelText, onchange } = mount(['Tech', 'News']);
        await fireEvent.click(getByLabelText('Remove Tech'));
        expect(onchange).toHaveBeenCalledWith(['News']);
    });

    it('removes the last tag on Backspace when the field is empty', async () => {
        const { input, onchange } = mount(['Tech', 'News']);
        await fireEvent.keyDown(input, { key: 'Backspace' });
        expect(onchange).toHaveBeenCalledWith(['Tech']);
    });

    it('does not fire onchange for a blank or duplicate-only entry', async () => {
        const { input, onchange } = mount(['Tech']);
        await fireEvent.input(input, { target: { value: '  Tech  ' } });
        await fireEvent.keyDown(input, { key: 'Enter' });
        expect(onchange).not.toHaveBeenCalled();
    });
});
