<script lang="ts">
// A minimal runes component proving the vitest + jsdom + testing-library
// stack compiles and renders Svelte 5 runes mode (no legacy stores).
let { label }: { label: string } = $props();
let count = $state(0);
const doubled = $derived(count * 2);
</script>

<button
    type="button"
    onclick={() => {
        count += 1;
    }}
>
    {label}: {count} (doubled {doubled})
</button>
