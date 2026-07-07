<script lang="ts" module>
/**
 * The icon set — a small, consistent library of stroked 24×24 glyphs
 * (Lucide-style), rendered with `currentColor` so they inherit text color.
 * Bundled inline (the CSP blocks remote assets and we ship no icon font).
 * Rendered as literal SVG children so the single-sanitized-render rule holds.
 */
export type IconName =
    | 'plus'
    | 'refresh'
    | 'settings'
    | 'search'
    | 'star'
    | 'star-filled'
    | 'check'
    | 'bookmark'
    | 'archive'
    | 'save'
    | 'type'
    | 'external'
    | 'close'
    | 'rss'
    | 'inbox'
    | 'sparkle'
    | 'tag'
    | 'folder'
    | 'activity'
    | 'pause'
    | 'message'
    | 'link'
    | 'play'
    | 'grid'
    | 'rows';
</script>

<script lang="ts">
let { name, size = 18, strokeWidth = 1.75, class: className = '' }: {
    name: IconName;
    size?: number;
    strokeWidth?: number;
    class?: string;
} = $props();
</script>

<svg
    class={className}
    width={size}
    height={size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width={strokeWidth}
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
>
    {#if name === 'plus'}
        <path d="M12 5v14M5 12h14" />
    {:else if name === 'refresh'}
        <path d="M3 12a9 9 0 0 1 15-6.7L21 8" /><path d="M21 3v5h-5" /><path
            d="M21 12a9 9 0 0 1-15 6.7L3 16"
        /><path d="M3 21v-5h5" />
    {:else if name === 'settings'}
        <circle cx="12" cy="12" r="3" /><path
            d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"
        />
    {:else if name === 'search'}
        <circle cx="11" cy="11" r="7" /><path d="m21 21-4.3-4.3" />
    {:else if name === 'star'}
        <path d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 17l-5.2 2.6 1-5.8L3.5 9.7l5.9-.9z" />
    {:else if name === 'star-filled'}
        <path
            d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8L12 17l-5.2 2.6 1-5.8L3.5 9.7l5.9-.9z"
            fill="currentColor"
            stroke="none"
        />
    {:else if name === 'check'}
        <path d="M20 6 9 17l-5-5" />
    {:else if name === 'bookmark'}
        <path d="M19 21l-7-5-7 5V5a2 2 0 0 1 2-2h10a2 2 0 0 1 2 2z" />
    {:else if name === 'archive'}
        <rect x="3" y="4" width="18" height="4" rx="1" /><path
            d="M5 8v11a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8M10 12h4"
        />
    {:else if name === 'save'}
        <path d="M12 3v12" /><path d="m8 11 4 4 4-4" /><path
            d="M4 17v2a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-2"
        />
    {:else if name === 'type'}
        <path d="M4 7V5h16v2" /><path d="M9 19h6" /><path d="M12 5v14" />
    {:else if name === 'external'}
        <path d="M15 3h6v6" /><path d="M10 14 21 3" /><path
            d="M18 13v6a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h6"
        />
    {:else if name === 'close'}
        <path d="M18 6 6 18M6 6l12 12" />
    {:else if name === 'rss'}
        <path d="M4 11a9 9 0 0 1 9 9" /><path d="M4 4a16 16 0 0 1 16 16" /><circle
            cx="5"
            cy="19"
            r="1.5"
            fill="currentColor"
            stroke="none"
        />
    {:else if name === 'inbox'}
        <path d="M22 12h-6l-2 3h-4l-2-3H2" /><path
            d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"
        />
    {:else if name === 'sparkle'}
        <path d="M12 3l1.9 5.3L19 10l-5.1 1.7L12 17l-1.9-5.3L5 10l5.1-1.7z" />
    {:else if name === 'tag'}
        <path d="M20.6 13.4 12 22l-9-9V4a1 1 0 0 1 1-1h8.6z" /><circle
            cx="7.5"
            cy="7.5"
            r="1"
            fill="currentColor"
            stroke="none"
        />
    {:else if name === 'folder'}
        <path d="M4 20a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2z" />
    {:else if name === 'activity'}
        <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
    {:else if name === 'pause'}
        <rect x="6" y="5" width="4" height="14" rx="1" /><rect
            x="14"
            y="5"
            width="4"
            height="14"
            rx="1"
        />
    {:else if name === 'message'}
        <path d="M21 11.5a8.38 8.38 0 0 1-8.5 8.5 9 9 0 0 1-4-.9L3 21l1.9-5.5A8.38 8.38 0 0 1 4 11.5 8.5 8.5 0 0 1 12.5 3 8.38 8.38 0 0 1 21 11.5z" />
    {:else if name === 'link'}
        <path d="M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7" /><path
            d="M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7"
        />
    {:else if name === 'play'}
        <path d="M6 4.5v15l13-7.5z" fill="currentColor" stroke="none" />
    {:else if name === 'grid'}
        <rect x="3" y="3" width="7" height="7" rx="1.5" /><rect
            x="14"
            y="3"
            width="7"
            height="7"
            rx="1.5"
        /><rect x="3" y="14" width="7" height="7" rx="1.5" /><rect
            x="14"
            y="14"
            width="7"
            height="7"
            rx="1.5"
        />
    {:else if name === 'rows'}
        <path d="M3 6h18M3 12h18M3 18h18" />
    {/if}
</svg>
