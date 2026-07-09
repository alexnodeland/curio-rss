<script lang="ts">
/**
 * A single shimmer placeholder bar for loading states. Decorative
 * (`aria-hidden`) by design: a consumer renders a set of these to sketch the
 * shape of the content that is loading, then pairs the set with one
 * `.sr-only` label that actually announces what is loading, so screen
 * readers hear "Loading articles…" while sighted users see the shimmer.
 *
 * Motion respects the user's preference: the global `prefers-reduced-motion`
 * rule collapses the shimmer, and this component also stops the animation
 * explicitly and leaves a static tinted fill in its place.
 */
let {
    width = '100%',
    height = '1em',
    radius = 'var(--radius-sm)',
}: {
    width?: string;
    height?: string;
    radius?: string;
} = $props();
</script>

<span
    class="skeleton"
    aria-hidden="true"
    style="width: {width}; height: {height}; border-radius: {radius};"
></span>

<style>
    .skeleton {
        display: block;
        background: linear-gradient(
            90deg,
            var(--surface-raised) 25%,
            var(--hover) 37%,
            var(--surface-raised) 63%
        );
        background-size: 400% 100%;
        animation: skeleton-shimmer 1.4s ease-in-out infinite;
    }

    @keyframes skeleton-shimmer {
        from {
            background-position: 100% 0;
        }
        to {
            background-position: 0 0;
        }
    }

    @media (prefers-reduced-motion: reduce) {
        .skeleton {
            animation: none;
            background: var(--surface-raised);
        }
    }
</style>
