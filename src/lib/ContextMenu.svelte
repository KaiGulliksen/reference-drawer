<!--
  A right-click menu. Shared by workspaces and images (and tags later).
  Closes on click outside, Escape, or when the window loses focus.
  Arrow keys move between items, Enter picks one.
-->
<script lang="ts">
  import type { MenuItem } from "./menu";

  interface Props {
    x: number;
    y: number;
    items: MenuItem[];
    onclose: () => void;
  }

  let { x, y, items, onclose }: Props = $props();

  let menu: HTMLDivElement;
  let left = $state(0);
  let top = $state(0);

  // Open at the cursor, but keep the whole menu inside the window.
  $effect(() => {
    const { width, height } = menu.getBoundingClientRect();
    left = Math.max(4, Math.min(x, innerWidth - width - 4));
    top = Math.max(4, Math.min(y, innerHeight - height - 4));
    menu.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  });

  function run(item: Exclude<MenuItem, "separator">) {
    onclose();
    item.action();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onclose();
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    buttons[(current + step + buttons.length) % buttons.length]?.focus();
  }
</script>

<svelte:window {onkeydown} onblur={onclose} onresize={onclose} />

<div
  class="backdrop"
  role="presentation"
  onpointerdown={(e) => e.button <= 2 && onclose()}
  oncontextmenu={(e) => {
    e.preventDefault();
    onclose();
  }}
></div>

<div class="menu" role="menu" bind:this={menu} style:left="{left}px" style:top="{top}px">
  {#each items as item, i (i)}
    {#if item === "separator"}
      <div class="separator" role="separator"></div>
    {:else}
      <button role="menuitem" class:danger={item.danger} disabled={item.disabled} onclick={() => run(item)}>
        <span>{item.label}</span>
        {#if item.hint}<span class="hint">{item.hint}</span>{/if}
      </button>
    {/if}
  {/each}
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 29;
  }
  .menu {
    position: fixed;
    z-index: 30;
    min-width: 180px;
    padding: 4px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 7px;
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
  }
  button {
    display: flex;
    justify-content: space-between;
    gap: 24px;
    width: 100%;
    padding: 6px 12px;
    text-align: left;
    background: transparent;
    border: none;
    border-radius: 4px;
    white-space: nowrap;
  }
  button:hover,
  button:focus-visible {
    background: var(--button-hover);
    outline: none;
  }
  button:disabled {
    color: var(--text-dim);
    background: transparent;
    cursor: default;
  }
  .danger {
    color: #ff8f8f;
  }
  .hint {
    color: var(--text-dim);
    font-size: 12px;
  }
  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
