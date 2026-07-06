<script lang="ts">
/** Test host for the generic VirtualList — plain numeric items, 40px rows. */
import VirtualList from '$components/articles/VirtualList.svelte';

let {
    items,
    rowHeight = 40,
    selectedIndex = -1,
    overscan = 4,
    nearEndMargin = 10,
    fallbackViewportHeight = 240,
    onnearend = undefined,
}: {
    items: number[];
    rowHeight?: number;
    selectedIndex?: number;
    overscan?: number;
    nearEndMargin?: number;
    fallbackViewportHeight?: number;
    onnearend?: () => void;
} = $props();
</script>

<VirtualList
    {items}
    {rowHeight}
    {selectedIndex}
    {overscan}
    {nearEndMargin}
    {fallbackViewportHeight}
    {onnearend}
    key={(item) => item}
    label="test list"
>
    {#snippet row(item, index)}
        <div
            class="test-row"
            role="option"
            aria-selected={index === selectedIndex}
            aria-posinset={index + 1}
            aria-setsize={items.length}
            style="height: 40px"
        >
            item-{item}
        </div>
    {/snippet}
</VirtualList>
