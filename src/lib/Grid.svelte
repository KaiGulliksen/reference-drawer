<!--
  The thumbnail grid. It is "virtualized": only the rows currently on screen
  (plus a couple above and below) exist in the page, so a folder with 20,000
  images costs about as much as one with 200.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { thumbUrl, type ImageEntry } from "./api";
  import type { TileFit } from "./settings.svelte";

  interface Props {
    images: ImageEntry[];
    tileSize: number;
    tileFit: TileFit;
    /** Index of the highlighted image, or -1. */
    selected: number;
    /** False while the viewer is open, so arrow keys go to the viewer instead. */
    active: boolean;
    onopen: (index: number) => void;
  }

  let { images, tileSize, tileFit, selected = $bindable(), active, onopen }: Props = $props();

  const GAP = 6;
  const PADDING = 12;
  /** Extra rows rendered above and below the visible area, so fast scrolling doesn't show blanks. */
  const OVERSCAN_ROWS = 2;

  let scroller: HTMLDivElement;
  let width = $state(0);
  let height = $state(0);
  let scrollTop = $state(0);
  const broken = new SvelteSet<string>();
  /** Tiles allowed to load their thumbnail. Tiles that only flash past during fast scrolling never get added. */
  const loadable = new SvelteSet<string>();
  let scrollingFast = $state(false);
  let lastScroll = { top: 0, time: 0 };
  let settleTimer: ReturnType<typeof setTimeout> | undefined;

  /** Scroll speed (pixels per millisecond) above which thumbnail loading pauses. */
  const FAST_SCROLL_SPEED = 4;
  /** How long scrolling must calm down before paused tiles load. */
  const SETTLE_MS = 100;

  const columns = $derived(Math.max(1, Math.floor((width - PADDING * 2 + GAP) / (tileSize + GAP))));
  // Tiles stretch slightly so each row fills the full width.
  const cell = $derived(Math.max(1, (width - PADDING * 2 - GAP * (columns - 1)) / columns));
  const rowHeight = $derived(cell + GAP);
  const rowCount = $derived(Math.ceil(images.length / columns));
  const totalHeight = $derived(rowCount * rowHeight - GAP + PADDING * 2);

  const visible = $derived.by(() => {
    const first = Math.max(0, Math.floor((scrollTop - PADDING) / rowHeight) - OVERSCAN_ROWS);
    const last = Math.min(rowCount - 1, Math.floor((scrollTop + height - PADDING) / rowHeight) + OVERSCAN_ROWS);
    const indices: number[] = [];
    for (let row = first; row <= last; row++) {
      for (let col = 0; col < columns; col++) {
        const i = row * columns + col;
        if (i < images.length) indices.push(i);
      }
    }
    return indices;
  });

  /** First image index of the top visible row; used to keep your place when the layout changes. */
  let anchor = 0;

  function onscroll() {
    scrollTop = scroller.scrollTop;
    anchor = Math.floor(Math.max(0, scrollTop - PADDING) / rowHeight) * columns;

    const now = performance.now();
    const speed = Math.abs(scrollTop - lastScroll.top) / Math.max(1, now - lastScroll.time);
    lastScroll = { top: scrollTop, time: now };
    if (speed > FAST_SCROLL_SPEED) scrollingFast = true;
    clearTimeout(settleTimer);
    settleTimer = setTimeout(() => (scrollingFast = false), SETTLE_MS);
  }

  // Let on-screen tiles load, except while scrolling fast.
  $effect(() => {
    if (scrollingFast) return;
    const paths = visible.map((i) => images[i].path);
    untrack(() => {
      for (const path of paths) loadable.add(path);
    });
  });

  // When the tile size or window width changes, keep the same images at the top of the view.
  $effect(() => {
    const top = Math.floor(anchor / columns) * rowHeight;
    untrack(() => {
      if (Math.abs(scroller.scrollTop - top) > 1) scroller.scrollTop = top;
    });
  });

  // A new folder starts at the top.
  $effect(() => {
    void images;
    untrack(() => {
      scroller.scrollTop = 0;
      scrollTop = 0;
      anchor = 0;
      broken.clear();
    });
  });

  // Scroll the selected tile into view (e.g. after keyboard navigation or closing the viewer).
  $effect(() => {
    const index = selected;
    untrack(() => {
      if (index < 0 || index >= images.length) return;
      const top = PADDING + Math.floor(index / columns) * rowHeight;
      if (top < scroller.scrollTop) {
        scroller.scrollTop = top - PADDING;
      } else if (top + cell > scroller.scrollTop + height) {
        scroller.scrollTop = top + cell - height + PADDING;
      }
    });
  });

  function onkeydown(e: KeyboardEvent) {
    if (!active || images.length === 0 || e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;

    const rowsPerPage = Math.max(1, Math.floor(height / rowHeight));
    const current = selected < 0 ? -1 : selected;
    let next: number;
    switch (e.key) {
      case "ArrowRight": next = current + 1; break;
      case "ArrowLeft": next = current - 1; break;
      case "ArrowDown": next = current < 0 ? 0 : current + columns; break;
      case "ArrowUp": next = current - columns; break;
      case "PageDown": next = current + columns * rowsPerPage; break;
      case "PageUp": next = current - columns * rowsPerPage; break;
      case "Home": next = 0; break;
      case "End": next = images.length - 1; break;
      case "Enter":
        if (current >= 0) onopen(current);
        e.preventDefault();
        return;
      default:
        return;
    }
    e.preventDefault();
    selected = Math.min(images.length - 1, Math.max(0, next));
  }
</script>

<svelte:window {onkeydown} />

<div class="scroller" bind:this={scroller} bind:clientWidth={width} bind:clientHeight={height} {onscroll}>
  <div class="canvas" style:height="{totalHeight}px">
    {#each visible as i (images[i].path)}
      {@const image = images[i]}
      <button
        class="tile"
        class:selected={i === selected}
        class:contain={tileFit === "contain"}
        style:width="{cell}px"
        style:height="{cell}px"
        style:transform="translate({PADDING + (i % columns) * (cell + GAP)}px, {PADDING +
          Math.floor(i / columns) * rowHeight}px)"
        title={image.name}
        onclick={() => (selected = i)}
        ondblclick={() => onopen(i)}
      >
        {#if broken.has(image.path)}
          <span class="broken">Can't preview</span>
        {:else if loadable.has(image.path)}
          <img src={thumbUrl(image)} alt={image.name} decoding="async" draggable="false" onerror={() => broken.add(image.path)} />
        {/if}
        <span class="label">{image.name}</span>
      </button>
    {/each}
  </div>
</div>

<style>
  .scroller {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    outline: none;
  }
  .canvas {
    position: relative;
  }
  .tile {
    position: absolute;
    top: 0;
    left: 0;
    padding: 0;
    border: none;
    border-radius: 4px;
    overflow: hidden;
    background: var(--tile-bg);
    cursor: pointer;
    outline: 2px solid transparent;
    outline-offset: 1px;
    contain: strict;
  }
  .tile.selected {
    outline-color: var(--accent);
  }
  .tile img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    display: block;
  }
  .tile.contain img {
    object-fit: contain;
  }
  .label {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    padding: 18px 8px 6px;
    font-size: 12px;
    color: #fff;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.75));
    opacity: 0;
    transition: opacity 0.12s;
    pointer-events: none;
  }
  .tile:hover .label,
  .tile.selected .label {
    opacity: 1;
  }
  .broken {
    display: grid;
    place-items: center;
    height: 100%;
    font-size: 12px;
    color: var(--text-dim);
  }
</style>
