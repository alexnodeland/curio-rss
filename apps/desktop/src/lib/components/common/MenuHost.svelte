<script lang="ts">
/**
 * The single mount point for the app's one-at-a-time menu (context menus,
 * dropdowns). Renders the open menu from `menuStore` with a transparent
 * backdrop that catches outside clicks, and dismisses on scroll/resize.
 * Escape and keyboard nav are owned by the Menu level itself.
 */
import Menu from '$components/common/Menu.svelte';
import { menuStore } from '$lib/state/menu.svelte';

function onScroll(): void {
    if (menuStore.isOpen) menuStore.close(false);
}
</script>

<svelte:window onresize={onScroll} onscrollcapture={onScroll} />

{#if menuStore.current !== null}
    <!-- Backdrop: outside-click dismissal without restoring invoker focus. -->
    <button
        class="menu-backdrop"
        type="button"
        tabindex="-1"
        aria-hidden="true"
        onpointerdown={(event) => {
            event.preventDefault();
            menuStore.close(false);
        }}
        oncontextmenu={(event) => {
            event.preventDefault();
            menuStore.close(false);
        }}
    ></button>
    <Menu
        items={menuStore.current.items}
        x={menuStore.current.x}
        y={menuStore.current.y}
        ariaLabel={menuStore.current.ariaLabel}
        onclose={() => menuStore.close(true)}
    />
{/if}

<style>
    .menu-backdrop {
        position: fixed;
        inset: 0;
        z-index: 999;
        background: transparent;
        border: none;
        padding: 0;
        cursor: default;
    }
</style>
