<!--
  Full-size image viewer. Scroll to zoom (towards the cursor), drag to pan,
  double-click to switch between "fit to window" and actual size.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { formatBytes, fullUrl, imageSize, previewSizeForScreen, previewUrl, thumbUrl, type ImageEntry } from "./api";

  interface Props {
    images: ImageEntry[];
    index: number;
    /** False while something (like the tag editor) is open on top, so keys go there instead. */
    active?: boolean;
    /** The tag names to show for an image. */
    tagsOf?: (path: string) => string[];
    onclose: () => void;
  }

  let { images, index = $bindable(), active = true, tagsOf, onclose }: Props = $props();

  const MARGIN = 24;
  /** Height of the info bar at the bottom, kept clear when fitting the image. */
  const BAR_HEIGHT = 46;
  /** Maximum zoom, as a multiple of the image's actual size. */
  const MAX_ZOOM = 32;
  // "100%" means one image pixel per screen pixel, even with Windows display scaling.
  const dpr = window.devicePixelRatio || 1;

  const previewSize = previewSizeForScreen();

  const image = $derived(images[index]);
  const imageTags = $derived(tagsOf?.(image.path) ?? []);

  let stage: HTMLDivElement;
  let stageWidth = $state(0);
  let stageHeight = $state(0);
  /** Real size of the image, from Rust; `null` while unknown, "failed" if it couldn't be read. */
  let realSize = $state<{ width: number; height: number } | "failed" | null>(null);
  /** Size of the preview as loaded (equals the real size when the original is served as the preview). */
  let previewDims = $state<{ width: number; height: number } | null>(null);
  let failed = $state(false);
  /** Whether the full-resolution original is needed at the current zoom, and whether it has loaded. */
  let wantFull = $state(false);
  let fullReady = $state(false);

  const naturalWidth = $derived(
    realSize && realSize !== "failed" ? realSize.width : realSize === "failed" ? (previewDims?.width ?? 0) : 0,
  );
  const naturalHeight = $derived(
    realSize && realSize !== "failed" ? realSize.height : realSize === "failed" ? (previewDims?.height ?? 0) : 0,
  );
  const loaded = $derived(previewDims !== null && naturalWidth > 0);
  /** While true the image follows the window size; zooming or panning switches to a free view. */
  let fitted = $state(true);
  let freeView = $state({ scale: 1, x: 0, y: 0 });
  let dragFrom = $state<{ x: number; y: number } | null>(null);

  // Fit inside the window, but never enlarge small images beyond actual size.
  const fitScale = $derived(
    naturalWidth && stageWidth
      ? Math.min(
          (stageWidth - MARGIN * 2) / naturalWidth,
          (stageHeight - BAR_HEIGHT - MARGIN * 2) / naturalHeight,
          1 / dpr,
        )
      : 1,
  );
  const view = $derived(
    fitted
      ? {
          scale: fitScale,
          x: (stageWidth - naturalWidth * fitScale) / 2,
          y: (stageHeight - BAR_HEIGHT - naturalHeight * fitScale) / 2,
        }
      : freeView,
  );
  const zoomPercent = $derived(Math.round(view.scale * dpr * 100));

  // Every newly shown image starts out fitted to the window, showing its preview.
  $effect.pre(() => {
    const path = image.path;
    untrack(() => {
      realSize = null;
      previewDims = null;
      failed = false;
      wantFull = false;
      fitted = true;
    });
    imageSize(path)
      .then((size) => path === image.path && (realSize = size))
      .catch(() => path === image.path && (realSize = "failed"));
  });

  // The preview covers zooms up to its own resolution. Beyond that, load the original,
  // and let it go again when zooming back out, so its memory is freed.
  // (The gap between the two thresholds stops it flipping back and forth.)
  $effect(() => {
    const longest = Math.max(naturalWidth, naturalHeight);
    if (!loaded || longest <= previewSize) {
      wantFull = false;
      return;
    }
    const detailNeeded = (view.scale * dpr * longest) / previewSize;
    if (detailNeeded > 1.05) wantFull = true;
    else if (detailNeeded < 0.9) wantFull = false;
  });

  $effect(() => {
    if (!wantFull) fullReady = false;
  });

  // Load events can arrive late, after switching to another image, so each <img> carries
  // the path it was created for (data-path) and stale events are ignored.
  const isCurrent = (img: HTMLImageElement) => img.dataset.path === image.path;

  async function onPreviewLoad(e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    // Wait until the image is fully decoded so the swap from the thumbnail is instant.
    await img.decode().catch(() => {});
    if (!isCurrent(img)) return;
    previewDims = { width: img.naturalWidth, height: img.naturalHeight };
    preloadNeighbours();
  }

  function onPreviewError(e: Event) {
    if (isCurrent(e.currentTarget as HTMLImageElement)) failed = true;
  }

  async function onFullLoad(e: Event) {
    const img = e.currentTarget as HTMLImageElement;
    await img.decode().catch(() => {});
    if (isCurrent(img)) fullReady = true;
  }

  function preloadNeighbours() {
    for (const step of [1, -1]) {
      const neighbour = images[(index + step + images.length) % images.length];
      if (neighbour) new Image().src = previewUrl(neighbour, previewSize);
    }
  }

  function zoomAt(x: number, y: number, factor: number) {
    if (!loaded) return;
    const current = view;
    const scale = Math.min(Math.max(current.scale * factor, Math.min(fitScale, 0.1)), MAX_ZOOM / dpr);
    const k = scale / current.scale;
    freeView = { scale, x: x - (x - current.x) * k, y: y - (y - current.y) * k };
    fitted = false;
  }

  function actualSize(x = stageWidth / 2, y = stageHeight / 2) {
    zoomAt(x, y, 1 / dpr / view.scale);
  }

  function go(step: number) {
    index = (index + step + images.length) % images.length;
  }

  // Wheel zoom needs a non-passive listener so the page itself doesn't scroll.
  $effect(() => {
    const el = stage;
    const onwheel = (e: WheelEvent) => {
      e.preventDefault();
      zoomAt(e.clientX, e.clientY, Math.exp(-e.deltaY * 0.0015));
    };
    el.addEventListener("wheel", onwheel, { passive: false });
    return () => el.removeEventListener("wheel", onwheel);
  });

  function onpointerdown(e: PointerEvent) {
    if (e.button !== 0 || !loaded) return;
    stage.setPointerCapture(e.pointerId);
    dragFrom = { x: e.clientX, y: e.clientY };
  }

  function onpointermove(e: PointerEvent) {
    if (!dragFrom) return;
    const dx = e.clientX - dragFrom.x;
    const dy = e.clientY - dragFrom.y;
    if (dx === 0 && dy === 0) return;
    const start = view;
    freeView = { scale: start.scale, x: start.x + dx, y: start.y + dy };
    fitted = false;
    dragFrom = { x: e.clientX, y: e.clientY };
  }

  function onpointerup() {
    dragFrom = null;
  }

  function ondblclick(e: MouseEvent) {
    if (fitted) actualSize(e.clientX, e.clientY);
    else fitted = true;
  }

  function onkeydown(e: KeyboardEvent) {
    if (!active || e.ctrlKey || e.altKey || e.metaKey) return;
    if (e.target instanceof HTMLInputElement) return;
    switch (e.key) {
      case "Escape": onclose(); break;
      case "ArrowRight": case "d": case "D": go(1); break;
      case "ArrowLeft": case "a": case "A": go(-1); break;
      case "Home": index = 0; break;
      case "End": index = images.length - 1; break;
      case "0": case "f": case "F": fitted = true; break;
      case "1": actualSize(); break;
      case "+": case "=": zoomAt(stageWidth / 2, stageHeight / 2, 1.25); break;
      case "-": zoomAt(stageWidth / 2, stageHeight / 2, 0.8); break;
      default: return;
    }
    e.preventDefault();
  }
</script>

<svelte:window {onkeydown} />

<div class="viewer" role="dialog" aria-modal="true" aria-label={image.name}>
  <div
    class="stage"
    class:dragging={dragFrom}
    bind:this={stage}
    bind:clientWidth={stageWidth}
    bind:clientHeight={stageHeight}
    role="presentation"
    {onpointerdown}
    {onpointermove}
    {onpointerup}
    onpointercancel={onpointerup}
    {ondblclick}
  >
    {#key image.path}
      {#if !loaded && !failed}
        <!-- The small thumbnail stands in while a big image is still loading. -->
        <img class="placeholder" src={thumbUrl(image)} alt="" draggable="false" />
      {/if}
      <!-- Both layers are sized to the real image, so the preview simply stretches to fit. -->
      <img
        class="layer"
        class:ready={loaded}
        src={previewUrl(image, previewSize)}
        data-path={image.path}
        alt={image.name}
        draggable="false"
        style:width={naturalWidth ? `${naturalWidth}px` : null}
        style:height={naturalHeight ? `${naturalHeight}px` : null}
        style:transform="translate({view.x}px, {view.y}px) scale({view.scale})"
        style:image-rendering={zoomPercent >= 300 ? "pixelated" : null}
        onload={onPreviewLoad}
        onerror={onPreviewError}
      />
      {#if wantFull}
        <img
          class="layer"
          class:ready={fullReady}
          src={fullUrl(image)}
          data-path={image.path}
          alt=""
          draggable="false"
          style:width="{naturalWidth}px"
          style:height="{naturalHeight}px"
          style:transform="translate({view.x}px, {view.y}px) scale({view.scale})"
          style:image-rendering={zoomPercent >= 300 ? "pixelated" : null}
          onload={onFullLoad}
        />
      {/if}
    {/key}
    {#if failed}
      <div class="message">This image can't be displayed.</div>
    {/if}
  </div>

  {#if images.length > 1}
    <button class="nav prev" onclick={() => go(-1)} aria-label="Previous image">‹</button>
    <button class="nav next" onclick={() => go(1)} aria-label="Next image">›</button>
  {/if}

  <div class="bar">
    <div class="info">
      <span class="name" title={image.path}>{image.name}</span>
      {#if imageTags.length}
        <span class="tags" title="Tags">{imageTags.join(" · ")}</span>
      {/if}
      {#if naturalWidth}<span>{naturalWidth} × {naturalHeight}</span>{/if}
      <span>{formatBytes(image.size)}</span>
      <span>{index + 1} / {images.length}</span>
    </div>
    <div class="controls">
      <span class="zoom">{loaded ? `${zoomPercent}%` : ""}</span>
      <button class:on={fitted} onclick={() => (fitted = true)} title="Fit to window (F)">Fit</button>
      <button onclick={() => actualSize()} title="Actual size (1)">1:1</button>
      <button onclick={() => revealItemInDir(image.path)} title="Show the file in Explorer">Show in folder</button>
      <button onclick={onclose} title="Close (Esc)" aria-label="Close">✕</button>
    </div>
  </div>
</div>

<style>
  .viewer {
    position: fixed;
    inset: 0;
    z-index: 10;
    background: var(--viewer-bg);
  }
  .stage {
    position: absolute;
    inset: 0;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
  }
  .stage.dragging {
    cursor: grabbing;
  }
  .layer {
    position: absolute;
    top: 0;
    left: 0;
    max-width: none;
    transform-origin: 0 0;
    opacity: 0;
    user-select: none;
  }
  .layer.ready {
    opacity: 1;
  }
  .placeholder {
    position: absolute;
    top: 24px;
    left: 24px;
    width: calc(100% - 48px);
    height: calc(100% - 48px - 46px);
    object-fit: contain;
    opacity: 0;
    /* Only appears if the full image takes a moment, to avoid a flash on quick loads. */
    animation: appear 0s 120ms forwards;
    pointer-events: none;
  }
  @keyframes appear {
    to {
      opacity: 1;
    }
  }
  .message {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: var(--text-dim);
  }
  .nav {
    position: absolute;
    top: 50%;
    transform: translateY(-50%);
    width: 48px;
    height: 96px;
    border: none;
    border-radius: 6px;
    font-size: 40px;
    line-height: 1;
    color: #fff;
    background: rgba(0, 0, 0, 0.35);
    opacity: 0;
    transition: opacity 0.15s;
    cursor: pointer;
  }
  .viewer:hover .nav {
    opacity: 0.6;
  }
  .viewer .nav:hover {
    opacity: 1;
  }
  .prev {
    left: 12px;
  }
  .next {
    right: 12px;
  }
  .bar {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 8px 12px;
    font-size: 13px;
    background: rgba(20, 20, 22, 0.85);
    border-top: 1px solid var(--border);
  }
  .info {
    display: flex;
    gap: 16px;
    min-width: 0;
    color: var(--text-dim);
  }
  .info span {
    white-space: nowrap;
  }
  .info .name {
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .info .tags {
    overflow: hidden;
    text-overflow: ellipsis;
    color: #c8d8ff;
  }
  .controls {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .zoom {
    min-width: 48px;
    text-align: right;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
</style>
