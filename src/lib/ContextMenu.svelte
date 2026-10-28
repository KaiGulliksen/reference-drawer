<!--
  A right-click menu, shared by everything that has one.
  Closes on click outside, Escape, or when the window loses focus.
  Arrow keys move between items (←/→ between colours in a colour row), Enter picks one.
-->
<script lang="ts">
  import { COLORS, colorHex } from "./colors";
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
    menu.querySelector<HTMLButtonElement>("[data-nav]:not(:disabled)")?.focus();
  });

  function run(item: { action: () => void }) {
    onclose();
    item.action();
  }

  function pick(item: { pick: (color: string | null) => void }, color: string | null) {
    onclose();
    item.pick(color);
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "Escape" || e.key === "Tab") {
      e.preventDefault();
      onclose();
      return;
    }
    const focused = document.activeElement as HTMLElement | null;
    const row = focused?.closest<HTMLElement>(".swatches");
    if (row && (e.key === "ArrowLeft" || e.key === "ArrowRight")) {
      e.preventDefault();
      const swatches = [...row.querySelectorAll<HTMLButtonElement>("button")];
      const i = swatches.indexOf(focused as HTMLButtonElement);
      swatches[Math.max(0, Math.min(swatches.length - 1, i + (e.key === "ArrowRight" ? 1 : -1)))]?.focus();
      return;
    }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    // A colour row is one stop for ↑/↓ (its current colour); ←/→ move along it.
    const stops = [...menu.querySelectorAll<HTMLButtonElement>("[data-nav]:not(:disabled)")];
    const current = stops.indexOf((row?.querySelector("[data-nav]") ?? focused) as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    stops[(current + step + stops.length) % stops.length]?.focus();
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
    {:else if "pick" in item}
      <div class="swatches" role="group" aria-label="Color">
        {#each [null, ...COLORS.map((c) => c.name)] as color (color)}
          {@const label = color ? COLORS.find((c) => c.name === color)?.label : "No color"}
          <button
            role="menuitemradio"
            class="swatch"
            class:none={!color}
            class:current={color === item.current}
            aria-checked={color === item.current}
            data-nav={color === item.current ? "" : undefined}
            style:--swatch={colorHex(color)}
            title={label}
            aria-label={label}
            onclick={() => pick(item, color)}
          ></button>
        {/each}
      </div>
    {:else}
      <button role="menuitem" data-nav class:danger={item.danger} disabled={item.disabled} onclick={() => run(item)}>
        <span class="label">
          {#if item.color !== undefined}
            <span class="dot" class:empty={!colorHex(item.color)} style:--swatch={colorHex(item.color)}></span>
          {/if}
          {item.label}
        </span>
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
  .label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--swatch);
  }
  .dot.empty {
    background: none;
    border: 1px solid var(--text-dim);
  }
  /* A row of round colour swatches; the current one has a ring around it. */
  .swatches {
    display: flex;
    gap: 2px;
    padding: 4px 8px;
  }
  .swatch {
    flex: none;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 2px solid transparent;
    border-radius: 50%;
    background: var(--swatch) content-box;
  }
  .swatch:hover,
  .swatch:focus-visible {
    background: var(--swatch) content-box;
    border-color: var(--text-dim);
    outline: none;
  }
  .swatch.current {
    border-color: var(--text);
  }
  /* "No colour": an empty circle with a line through it. */
  .swatch.none {
    background:
      linear-gradient(135deg, transparent 45%, var(--text-dim) 45%, var(--text-dim) 55%, transparent 55%) content-box;
    box-shadow: inset 0 0 0 3px var(--panel), inset 0 0 0 4px var(--text-dim);
  }
  .swatch.none:hover,
  .swatch.none:focus-visible {
    background:
      linear-gradient(135deg, transparent 45%, var(--text-dim) 45%, var(--text-dim) 55%, transparent 55%) content-box;
  }
  .separator {
    height: 1px;
    margin: 4px 6px;
    background: var(--border);
  }
</style>
