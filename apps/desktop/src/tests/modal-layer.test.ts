/**
 * The in-frame modal action: everything in the app frame outside the
 * dialog goes inert (its own ancestors stay live), the walk stops at the
 * main region, only what it marked is restored, and focus returns to the
 * trigger once the trigger is live again.
 */
import { inertOutside, modalLayer } from '$lib/actions/modal-layer';
import { afterEach, describe, expect, it } from 'vitest';

function shell(): Record<string, HTMLElement> {
    document.body.innerHTML = `
        <div id="toasts"></div>
        <main id="main-content" tabindex="-1">
            <aside id="sidebar"><button id="sidebar-button">x</button></aside>
            <section id="reader">
                <div id="body"><img id="trigger" tabindex="0" alt="a"></div>
                <div id="already" inert></div>
                <div id="layer"><div id="dialog" tabindex="-1"><button id="close">x</button></div></div>
            </section>
        </main>`;
    const byId = (id: string): HTMLElement => {
        const element = document.getElementById(id);
        if (element === null) {
            throw new Error(`fixture #${id} missing`);
        }
        return element;
    };
    return Object.fromEntries(
        ['toasts', 'sidebar', 'reader', 'body', 'trigger', 'already', 'layer', 'dialog'].map(
            (id) => [id, byId(id)],
        ),
    );
}

describe('modalLayer', () => {
    afterEach(() => {
        document.body.innerHTML = '';
    });

    it('makes the frame outside the dialog inert, up to the main region', () => {
        const el = shell();
        const marked = inertOutside(el.dialog as HTMLElement);
        expect(marked).toEqual([el.body, el.sidebar]);
        expect(el.body?.hasAttribute('inert')).toBe(true);
        expect(el.sidebar?.hasAttribute('inert')).toBe(true);
        // The dialog's own ancestors stay live; outside main is untouched.
        for (const live of [el.layer, el.reader, el.dialog, el.toasts]) {
            expect(live?.hasAttribute('inert')).toBe(false);
        }
    });

    it('restores only what it marked and hands focus back to the trigger', () => {
        const el = shell();
        el.trigger?.focus();
        const layer = modalLayer(el.dialog as HTMLElement);
        expect(document.activeElement).toBe(el.dialog);
        expect(el.body?.hasAttribute('inert')).toBe(true);

        layer.destroy();
        expect(el.body?.hasAttribute('inert')).toBe(false);
        expect(el.sidebar?.hasAttribute('inert')).toBe(false);
        expect(el.already?.hasAttribute('inert')).toBe(true);
        expect(document.activeElement).toBe(el.trigger);
    });

    it('stops at <body> when there is no main region', () => {
        document.body.innerHTML = '<p id="other">x</p><div id="host"><div id="d"></div></div>';
        const dialog = document.getElementById('d');
        if (dialog === null) {
            throw new Error('fixture');
        }
        const marked = inertOutside(dialog);
        expect(marked.map((element) => element.id)).toEqual(['other']);
        expect(document.body.hasAttribute('inert')).toBe(false);
    });
});
