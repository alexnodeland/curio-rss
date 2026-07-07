/**
 * Accessibility smoke test: runs axe-core over the components most likely
 * to regress on ARIA semantics — the shared modal shell (via the help
 * dialog) and the toast outlet — and fails on any violation.
 *
 * jsdom can't lay elements out, so `color-contrast` (which needs real
 * geometry) is disabled here; contrast is enforced separately over the
 * theme tokens. Page-scoped rules (landmarks, a single main, one h1) don't
 * apply to a component rendered in isolation and are disabled too.
 */
import Toasts from '$components/common/Toasts.svelte';
import HelpOverlay from '$components/modals/HelpOverlay.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { render } from '@testing-library/svelte';
import axe from 'axe-core';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

const AXE_OPTIONS: axe.RunOptions = {
    rules: {
        'color-contrast': { enabled: false },
        region: { enabled: false },
        'landmark-one-main': { enabled: false },
        'page-has-heading-one': { enabled: false },
    },
};

/** Runs axe over a node and returns a readable list of any violations. */
async function auditViolations(node: Element): Promise<string[]> {
    const results = await axe.run(node, AXE_OPTIONS);
    return results.violations.map((violation) => {
        const where = violation.nodes.map((n) => n.target.join(' ')).join(', ');
        return `${violation.id} (${violation.impact}): ${violation.help} — at ${where}`;
    });
}

describe('accessibility smoke', () => {
    afterEach(() => {
        uiStore.reset();
    });

    it('the help dialog has no axe violations', async () => {
        const { container } = render(HelpOverlay, { props: { onclose: vi.fn() } });
        const violations = await auditViolations(container);
        expect(violations, violations.join('\n')).toEqual([]);
    });

    it('the toast outlet has no axe violations', async () => {
        const { container } = render(Toasts);
        uiStore.showToast('note saved', 'success', 0);
        uiStore.showToast('fetch failed', 'error', 0);
        await tick();
        const violations = await auditViolations(container);
        expect(violations, violations.join('\n')).toEqual([]);
    });
});
