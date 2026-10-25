<!--
  The thumbnail grid, as square tiles in rows or as a masonry layout (see layout.ts).
  It is "virtualized": only the tiles currently on screen (plus a margin above and
  below) exist in the page, so a folder with 20,000 images costs about as much as
  one with 200.
-->
<script lang="ts">
  import { tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { thumbUrl, type ImageEntry } from "./api";
  import { computeLayout, firstTileFrom, neighbour, PADDING, tileHeight, tilesBetween, tileTop } from "./layout";
  import { hasOtherModifier, hasPrimaryModifier } from "./platform";
  import type { GridLayout, TileFit } from "./settings.svelte";

  interface Props {
    images: ImageEntry[];
    mode: GridLayout;
    /** Target column width, in CSS pixels. */
    tileSize: number;
    /** How square tiles show their image (the masonry layout always shows the whole image). */
    tileFit: TileFit;
    /** Show each image's tags, and/or its file name, on the thumbnail. */
    showTags: boolean;
    showNames: boolean;
    /** Paths of the selected images. The grid changes this set as you click and use the keyboard. */
    selection: SvelteSet<string>;
    /** Index of the image the keyboard is on (and that Enter opens), or -1. */
    focus: number;
    /** False while the viewer, a menu or a dialog is open, so keys go there instead. */
    active: boolean;
    /** Index of the image to start at the top of the view (to return to where you were). */
    initialTopIndex?: number;
    /** The tag names to show under an image's name. */
    tagsOf?: (path: string) => string[];
    onopen: (index: number) => void;
    /** Right-click on an image. */
    onitemmenu?: (index: number, x: number, y: number) => void;
  }

  let {
    images,
    mode,
    tileSize,
    tileFit,
    showTags,
    showNames,
    selection,
    focus = $bindable(),
    active,
    initialTopIndex = 0,
    tagsOf,
    onopen,
    onitemmenu,
  }: Props = $props();

  /** Where a Shift-click or Shift+arrow range starts from. */
  let rangeStart = untrack(() => focus);

  function selectOnly(i: number) {
    selection.clear();
    selection.add(images[i].path);
    rangeStart = i;
  }

  function toggle(i: number) {
    const path = images[i].path;
    if (selection.has(path)) selection.delete(path);
    else selection.add(path);
    rangeStart = i;
  }

  /** Selects every image from the range start to `i`, keeping the rest of the selection if `add`. */
  function selectRange(i: number, add: boolean) {
    if (rangeStart < 0 || rangeStart >= images.length) rangeStart = i;
    if (!add) selection.clear();
    const [from, to] = rangeStart < i ? [rangeStart, i] : [i, rangeStart];
    for (let k = from; k <= to; k++) selection.add(images[k].path);
  }

  function onTileClick(e: MouseEvent, i: number) {
    const add = hasPrimaryModifier(e); // Ctrl, or Cmd on a Mac
    if (e.shiftKey) selectRange(i, add);
    else if (add) toggle(i);
    else selectOnly(i);
    focus = i;
  }

  /** Extra tiles rendered above and below the visible area (in pixels), so fast scrolling doesn't show blanks. */
  const OVERSCAN_PX = 500;

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

  const layout = $derived(computeLayout(images, width, tileSize, mode));
  const visible = $derived(tilesBetween(layout, scrollTop - OVERSCAN_PX, scrollTop + height + OVERSCAN_PX));

  /** The first image at the top of the view; used to keep your place when the layout changes. */
  let anchor = untrack(() => initialTopIndex);
  /** The last scroll position set by the grid itself (rather than by you scrolling). */
  let ownScrollTop = -1;

  /** The image at the top of the view, so the page can bring you back here later. */
  export function topIndex() {
    return anchor;
  }

  function onscroll() {
    scrollTop = scroller.scrollTop;
    // The browser reports scrolls a moment later. If this is just the echo of the grid moving
    // itself, the layout may have changed since, so don't recalculate where you are from it.
    if (Math.abs(scrollTop - ownScrollTop) < 1) {
      ownScrollTop = -1;
    } else {
      anchor = firstTileFrom(layout, scrollTop);
    }

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

  // When the layout, tile size or window width changes, keep the same images at the top of
  // the view. (This is also what scrolls to `initialTopIndex` once the grid knows its size.)
  $effect(() => {
    if (width === 0) return; // not laid out yet, so the column count isn't known
    const top = anchor > 0 && anchor < images.length ? Math.max(0, tileTop(layout, anchor) - PADDING) : 0;
    // Wait until the page shows the new layout. Scrolling before the grid has grown to its
    // new height would get cut short.
    tick().then(() => {
      if (Math.abs(scroller.scrollTop - top) > 1) {
        scroller.scrollTop = top;
        ownScrollTop = scroller.scrollTop; // may be less than `top` near the end
      }
    });
  });

  // Files may have been fixed or replaced when the list is refreshed, so let broken ones retry.
  $effect(() => {
    void images;
    untrack(() => broken.clear());
  });

  // Scroll the focused tile into view (e.g. after keyboard navigation or closing the viewer).
  $effect(() => {
    const index = focus;
    untrack(() => {
      // Before the grid has a size (just after opening) the restored scroll position wins.
      if (index < 0 || index >= images.length || height === 0) return;
      const top = tileTop(layout, index);
      const bottom = top + tileHeight(layout, index);
      if (top < scroller.scrollTop) {
        scroller.scrollTop = top - PADDING;
      } else if (bottom > scroller.scrollTop + height) {
        // Tiles taller than the window are lined up by their top instead.
        scroller.scrollTop = Math.min(top - PADDING, bottom - height + PADDING);
      }
    });
  });

  function onkeydown(e: KeyboardEvent) {
    if (!active || images.length === 0 || hasOtherModifier(e)) return;
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;

    if (hasPrimaryModifier(e)) {
      if (e.key === "a" || e.key === "A") {
        for (const image of images) selection.add(image.path);
        e.preventDefault();
      }
      return;
    }
    if (e.key === "Escape" && selection.size > 0) {
      selection.clear();
      e.preventDefault();
      return;
    }

    const current = focus < 0 || focus >= images.length ? -1 : focus;
    if (e.key === "Enter") {
      if (current >= 0) onopen(current);
      e.preventDefault();
      return;
    }
    const moves = ["ArrowRight", "ArrowLeft", "ArrowDown", "ArrowUp", "PageDown", "PageUp", "Home", "End"];
    if (!moves.includes(e.key)) return;
    e.preventDefault();
    const next = neighbour(layout, current, e.key, height);
    // Shift extends the selection from where it started; plain arrows move a single selection.
    if (e.shiftKey) {
      if (current >= 0 && rangeStart < 0) rangeStart = current;
      selectRange(next, false);
    } else {
      selectOnly(next);
    }
    focus = next;
  }
</script>

<svelte:window {onkeydown} />

<div class="scroller" bind:this={scroller} bind:clientWidth={width} bind:clientHeight={height} {onscroll}>
  <div class="canvas" style:height="{layout.totalHeight}px">
    {#each visible as i (images[i].path)}
      {@const image = images[i]}
      {@const tileTags = tagsOf?.(image.path) ?? []}
      {@const r = layout.rects.subarray(i * 4, i * 4 + 4)}
      <button
        class="tile"
        class:selected={selection.has(image.path)}
        class:focused={i === focus}
        class:contain={mode === "grid" && tileFit === "contain"}
        style:width="{r[2]}px"
        style:height="{r[3]}px"
        style:transform="translate({r[0]}px, {r[1]}px)"
        title={tileTags.length ? `${image.name}\nTags: ${tileTags.join(", ")}` : image.name}
        onclick={(e) => onTileClick(e, i)}
        ondblclick={(e) => !hasPrimaryModifier(e) && !e.shiftKey && onopen(i)}
        oncontextmenu={(e) => {
          e.preventDefault();
          // Right-clicking inside the selection keeps it, so a menu action can apply to all of it.
          if (!selection.has(image.path)) selectOnly(i);
          focus = i;
          onitemmenu?.(i, e.clientX, e.clientY);
        }}
      >
        {#if broken.has(image.path)}
          <span class="broken">Can't preview</span>
        {:else if loadable.has(image.path)}
          <img src={thumbUrl(image)} alt={image.name} decoding="async" draggable="false" onerror={() => broken.add(image.path)} />
        {/if}
        {#if selection.has(image.path)}
          <span class="check" aria-hidden="true">
            <svg viewBox="0 0 12 12" width="11" height="11"><path d="M2.5 6.2l2.3 2.3 4.7-5" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" /></svg>
          </span>
        {/if}
        {#if tileTags.length && !showTags}
          <!-- With tags hidden, a small dot still shows that the image has some. -->
          <span class="tag-dot" aria-hidden="true"></span>
        {/if}
        {#if showNames || (showTags && tileTags.length)}
          <span class="overlay">
            {#if showTags && tileTags.length}
              <span class="chips">
                {#each tileTags as tag (tag)}<span class="chip">{tag}</span>{/each}
              </span>
            {/if}
            {#if showNames}<span class="name">{image.name}</span>{/if}
          </span>
        {/if}
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
  .tile.focused {
    outline: 2px dashed var(--text-dim);
  }
  .tile.selected {
    outline: 2px solid var(--accent);
  }
  /* A tint over selected tiles, so a selection of many images is easy to see. */
  .tile.selected::after {
    content: "";
    position: absolute;
    inset: 0;
    background: var(--accent);
    opacity: 0.18;
    pointer-events: none;
  }
  .check {
    position: absolute;
    top: 6px;
    left: 6px;
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    color: #fff;
    background: var(--accent);
    box-shadow: 0 0 0 2px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }
  .tag-dot {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 2px rgba(0, 0, 0, 0.45);
    pointer-events: none;
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
  /* Tags and/or the file name along the bottom of the tile. */
  .overlay {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    padding: 18px 6px 6px;
    text-align: left;
    background: linear-gradient(transparent, rgba(0, 0, 0, 0.6));
    pointer-events: none;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    /* At most two rows of tags; the full list is in the tooltip and the status bar. */
    max-height: calc(2 * 18px + 3px);
    overflow: hidden;
  }
  .chip {
    padding: 1px 7px;
    font-size: 11px;
    line-height: 16px;
    white-space: nowrap;
    color: #e8eeff;
    background: rgba(28, 44, 78, 0.85);
    border: 1px solid rgba(110, 150, 230, 0.45);
    border-radius: 999px;
  }
  .name {
    max-width: 100%;
    padding: 0 2px;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
    font-size: 12px;
    color: #fff;
  }
  .broken {
    display: grid;
    place-items: center;
    height: 100%;
    font-size: 12px;
    color: var(--text-dim);
  }
</style>
