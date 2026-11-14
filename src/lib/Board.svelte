<!--
  A mood board: a canvas with no edges where images (and text notes) can be placed,
  resized, overlapped and cropped freely.

  Mouse: drag empty space (or use the middle button anywhere) to move around, scroll to zoom,
  Shift+drag to select with a box, drag an image to move it (and everything selected with it),
  drag the corner handles to resize. Double-click an image to view it full size, a note to edit
  it, or empty space to add a note. Keys: Delete removes from the board, F fits everything,
  Ctrl+A (⌘A) selects all, Ctrl+Z / Ctrl+Y (⌘Z / ⌘Y, or with Shift) undo and redo, Esc clears the
  selection.

  Images load at the resolution they're shown at: thumbnails when small, then a medium preview,
  then a screen-sized one, and the original only for images on screen and zoomed in close.
-->
<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import {
    addBoardNote,
    boardFiles,
    fullUrl,
    getBoard,
    MEDIUM_PREVIEW_SIZE,
    previewSizeForScreen,
    previewUrl,
    removeBoardItems,
    restoreBoardItems,
    setBoardView,
    THUMB_SIZE,
    thumbUrl,
    updateBoardItems,
    type BoardItem,
    type Crop,
    type ImageEntry,
  } from "./api";
  import {
    arrange,
    bounds,
    FULL_CROP,
    fitView,
    isFullCrop,
    NOTE_SIZE,
    overlaps,
    uncroppedRect,
    type Rect,
  } from "./board";
  import type { MenuItem, MenuRequest } from "./menu";
  import { baseName, fileManagerName, hasOtherModifier, hasPrimaryModifier, pathKey } from "./platform";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";

  interface Props {
    boardId: number;
    /** The workspace's images (from the folder scan). When this changes, the folder has changed. */
    images: ImageEntry[];
    /** False while something (a menu, a dialog, the viewer) is open on top, so keys go there. */
    active: boolean;
    /** Current zoom, for showing in the toolbar. */
    zoom?: number;
    /** Whether there's something to undo or redo, for the toolbar's buttons. */
    canUndo?: boolean;
    canRedo?: boolean;
    /** Open the viewer on these board images, starting at `index`. */
    onopen: (images: ImageEntry[], index: number) => void;
    onmenu: (menu: MenuRequest) => void;
    /** Called whenever the number of images or notes on the board changes. */
    oncount: (images: number, notes: number) => void;
    onerror: (message: string) => void;
  }

  let {
    boardId,
    images,
    active,
    zoom = $bindable(1),
    canUndo = $bindable(false),
    canRedo = $bindable(false),
    onopen,
    onmenu,
    oncount,
    onerror,
  }: Props = $props();

  const MIN_ZOOM = 0.03;
  const MAX_ZOOM = 16;
  /** Pointer movement (in screen pixels) before a press counts as a drag rather than a click. */
  const DRAG_THRESHOLD = 3;
  /** Smallest size an image can be resized to, in board units. */
  const MIN_SIZE = 24;
  const MIN_CROP = 0.04;
  /** How long after the last key press a note being typed is saved. */
  const NOTE_SAVE_DELAY_MS = 250;
  const dpr = window.devicePixelRatio || 1;
  const previewSize = previewSizeForScreen();

  let stage: HTMLDivElement;
  let stageWidth = $state(0);
  let stageHeight = $state(0);
  let items = $state<BoardItem[]>([]);
  let view = $state({ x: 0, y: 0, zoom: 1 });
  let loaded = $state(false);
  let needsFit = false;
  let neverViewed = false;
  const selected = new SvelteSet<number>();
  const broken = new SvelteSet<number>();
  let marquee = $state<Rect | null>(null);
  let editingNote = $state<number | null>(null);
  /** A note that was just added and is still empty: removed again if left empty. */
  let freshNote: number | null = null;
  /** The image being cropped: its whole image's rectangle, and the crop being drawn. */
  let cropping = $state<{ id: number; full: Rect; crop: Crop } | null>(null);
  /** The zoom once it stops changing, used to pick image resolutions (so they don't change mid-zoom). */
  let settledZoom = $state(1);

  type Drag =
    | { kind: "pan"; startX: number; startY: number; viewX: number; viewY: number; moved: boolean }
    | { kind: "move"; startX: number; startY: number; starts: Map<number, { x: number; y: number }>; moved: boolean }
    | { kind: "resize"; fixed: { x: number; y: number }; box: Rect; starts: Map<number, Rect>; free: boolean; moved: boolean }
    | { kind: "marquee"; startX: number; startY: number; base: number[] }
    | { kind: "crop"; handle: string; startX: number; startY: number; start: Crop };
  let drag: Drag | null = null;
  let panning = $state(false);
  /** Items changed by the current drag (moved, resized, brought to front), saved when it ends. */
  const dirty = new Set<number>();

  /** The board's image files that are really on disk, by `pathKey`. */
  let files = $state.raw(new Map<string, ImageEntry>());
  const ordered = $derived([...items].sort((a, b) => a.z - b.z || a.id - b.id));
  const selectedItems = $derived(items.filter((item) => selected.has(item.id)));
  const selectionBox = $derived(selectedItems.length > 0 ? bounds(selectedItems) : null);

  $effect(() => {
    zoom = view.zoom;
  });

  // ---- Loading and saving ----

  onMount(() => {
    load().catch((e) => onerror(String(e)));
  });

  async function load() {
    const data = await getBoard(boardId);
    await loadFiles();
    // A new note is saved before anything is typed in it, so closing the app at that moment leaves
    // an empty one behind. It holds nothing and can't be seen well, so tidy it away here.
    const empty = data.items.filter((item) => item.path === null && !item.text?.trim()).map((item) => item.id);
    if (empty.length > 0) await removeBoardItems(boardId, empty);
    items = data.items.filter((item) => !empty.includes(item.id));
    neverViewed = data.view.zoom <= 0;
    if (!neverViewed) view = { x: data.view.x, y: data.view.y, zoom: data.view.zoom };
    // Fit if it's never been looked at, or if nothing would be on screen where it was left
    // (e.g. it was empty then, and images were added from the grid since). Checked once the
    // canvas has a size, below.
    needsFit = true;
    settledZoom = view.zoom;
    loaded = true;
    report();
  }

  async function loadFiles() {
    files = new Map((await boardFiles(boardId)).map((file) => [pathKey(file.path), file]));
  }

  /** Reloads the items (e.g. after images were added from the grid), keeping the view. Can be undone. */
  export async function reload() {
    const before = snapshot();
    const data = await getBoard(boardId);
    await loadFiles();
    items = data.items;
    rememberIfChanged(before);
    for (const id of [...selected]) if (!items.some((i) => i.id === id)) selected.delete(id);
    report();
  }

  // The folder changed: files may have appeared or gone, and images may have been renamed (the
  // library has followed them, so only the paths here are out of date). Everything else about
  // the items stays as it is, as they may be being dragged right now.
  let seenFirstImages = false;
  $effect(() => {
    void images;
    if (!seenFirstImages) {
      seenFirstImages = true;
      return;
    }
    untrack(() => followFolder().catch((e) => onerror(String(e))));
  });

  async function followFolder() {
    const data = await getBoard(boardId);
    await loadFiles();
    const paths = new Map(data.items.map((item) => [item.id, item.path]));
    for (const item of items) {
      const path = paths.get(item.id);
      if (path !== undefined && path !== item.path) item.path = path;
    }
  }

  // When a board opens: if it was never viewed, or none of it is on screen, fit everything.
  $effect(() => {
    if (loaded && stageWidth > 0 && needsFit) {
      needsFit = false;
      untrack(() => {
        const viewport = toBoardViewport();
        if (neverViewed || !items.some((item) => overlaps(viewport, item))) fitAll();
      });
    }
  });

  // Remember where the board was looked at, a moment after moving or zooming stops.
  $effect(() => {
    const saved = { x: view.x, y: view.y, zoom: view.zoom };
    if (!loaded) return;
    const timer = setTimeout(() => setBoardView(boardId, saved).catch(() => {}), 500);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    const z = view.zoom;
    const timer = setTimeout(() => (settledZoom = z), 200);
    return () => clearTimeout(timer);
  });

  function report() {
    const notes = items.filter((item) => item.path === null).length;
    oncount(items.length - notes, notes);
  }

  async function save(ids: Iterable<number>) {
    const wanted = new Set(ids);
    if (wanted.size === 0) return;
    const changed = $state.snapshot(items).filter((item) => wanted.has(item.id));
    try {
      await updateBoardItems(boardId, changed);
    } catch (e) {
      onerror(String(e));
    }
  }

  // ---- Coordinates and the view ----

  function stagePoint(e: MouseEvent) {
    const r = stage.getBoundingClientRect();
    return { x: e.clientX - r.left, y: e.clientY - r.top };
  }

  const toBoard = (sx: number, sy: number) => ({ x: (sx - view.x) / view.zoom, y: (sy - view.y) / view.zoom });

  function toScreen(r: Rect): Rect {
    return { x: r.x * view.zoom + view.x, y: r.y * view.zoom + view.y, width: r.width * view.zoom, height: r.height * view.zoom };
  }

  function zoomAt(sx: number, sy: number, factor: number) {
    const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, view.zoom * factor));
    const k = next / view.zoom;
    view = { zoom: next, x: sx - (sx - view.x) * k, y: sy - (sy - view.y) * k };
  }

  function fitAll() {
    const area = bounds(items);
    if (!area) {
      view = { x: stageWidth / 2, y: stageHeight / 2, zoom: 1 };
      return;
    }
    view = fitView(area, stageWidth, stageHeight);
  }

  /** Zooms to show everything on the board. */
  export function fit() {
    fitAll();
  }

  $effect(() => {
    const el = stage;
    const onwheel = (e: WheelEvent) => {
      e.preventDefault();
      const p = stagePoint(e);
      zoomAt(p.x, p.y, Math.exp(-e.deltaY * 0.0015));
    };
    el.addEventListener("wheel", onwheel, { passive: false });
    return () => el.removeEventListener("wheel", onwheel);
  });

  // ---- Choosing how sharp each image needs to be ----

  function entryFor(path: string): ImageEntry {
    return (
      files.get(pathKey(path)) ?? { path, name: baseName(path), size: 0, modified: 0, width: 0, height: 0 }
    );
  }

  function isMissing(path: string) {
    return !files.has(pathKey(path));
  }

  /** The sharper version to show on top of the thumbnail, or null when the thumbnail is enough. */
  function sharperSource(item: BoardItem): string | null {
    if (item.path === null) return null;
    const visible = overlaps(toBoardViewport(), item);
    if (!visible) return null;
    const crop = item.crop ?? FULL_CROP;
    // How many screen pixels the whole image (not just the cropped part) covers on its longest side.
    const shown = Math.max(item.width / crop.w, item.height / crop.h) * settledZoom * dpr;
    if (shown <= THUMB_SIZE * 1.1) return null;
    const entry = entryFor(item.path);
    // Most images on a board are shown well below screen size: a medium preview is sharp enough
    // and several times lighter than a screen-sized one.
    if (shown <= MEDIUM_PREVIEW_SIZE * 1.1) return previewUrl(entry, MEDIUM_PREVIEW_SIZE);
    const longest = Math.max(entry.width, entry.height);
    if (shown <= previewSize * 1.1 || (longest > 0 && longest <= previewSize)) return previewUrl(entry, previewSize);
    return fullUrl(entry);
  }

  function toBoardViewport(): Rect {
    const topLeft = toBoard(0, 0);
    return { x: topLeft.x, y: topLeft.y, width: stageWidth / view.zoom, height: stageHeight / view.zoom };
  }

  // ---- Pointer: selecting, moving, resizing, panning, cropping ----

  function onpointerdown(e: PointerEvent) {
    if (!active || e.button === 2) return;
    const target = e.target as HTMLElement;
    if (target.closest(".note-editor")) return; // typing in a note
    if (editingNote !== null) finishEditing();
    const p = stagePoint(e);
    stage.setPointerCapture(e.pointerId);

    if (e.button === 1) {
      e.preventDefault(); // no autoscroll; the middle button moves around
      startPan(p);
      return;
    }
    if (e.button !== 0) return;
    const handle = target.closest<HTMLElement>("[data-handle]")?.dataset.handle;

    if (cropping) {
      if (handle || target.closest("[data-crop-window]")) {
        drag = { kind: "crop", handle: handle ?? "move", startX: p.x, startY: p.y, start: { ...cropping.crop } };
      } else {
        applyCrop();
        startPan(p);
      }
      return;
    }

    if (handle && selectionBox) {
      startResize(handle, selectionBox);
      return;
    }

    const itemEl = target.closest<HTMLElement>("[data-item]");
    if (itemEl) {
      const id = Number(itemEl.dataset.item);
      if (hasPrimaryModifier(e)) {
        if (selected.has(id)) {
          selected.delete(id);
          return;
        }
        selected.add(id);
      } else if (e.shiftKey) {
        selected.add(id);
      } else if (!selected.has(id)) {
        selected.clear();
        selected.add(id);
      }
      // Stacking only changes when asked for (Bring to front / Send to back in the menu), so
      // selecting or moving an image doesn't undo where you've put it.
      const starts = new Map(selectedItems.map((item) => [item.id, { x: item.x, y: item.y }]));
      drag = { kind: "move", startX: p.x, startY: p.y, starts, moved: false };
      return;
    }

    if (e.shiftKey) {
      drag = { kind: "marquee", startX: p.x, startY: p.y, base: [...selected] };
      marquee = { x: p.x, y: p.y, width: 0, height: 0 };
    } else {
      startPan(p);
    }
  }

  function startPan(p: { x: number; y: number }) {
    drag = { kind: "pan", startX: p.x, startY: p.y, viewX: view.x, viewY: view.y, moved: false };
    panning = true;
  }

  /** Corner handles scale the whole selection from the opposite corner, keeping shapes. A single note resizes freely. */
  function startResize(corner: string, box: Rect) {
    const fixed = {
      x: corner.includes("w") ? box.x + box.width : box.x,
      y: corner.includes("n") ? box.y + box.height : box.y,
    };
    const starts = new Map(selectedItems.map((item) => [item.id, { x: item.x, y: item.y, width: item.width, height: item.height }]));
    const free = selectedItems.length === 1 && selectedItems[0].path === null;
    drag = { kind: "resize", fixed, box, starts, free, moved: false };
  }

  function onpointermove(e: PointerEvent) {
    if (!drag) return;
    const p = stagePoint(e);
    switch (drag.kind) {
      case "pan": {
        const dx = p.x - drag.startX;
        const dy = p.y - drag.startY;
        if (Math.abs(dx) + Math.abs(dy) > DRAG_THRESHOLD) drag.moved = true;
        view = { ...view, x: drag.viewX + dx, y: drag.viewY + dy };
        break;
      }
      case "move": {
        const dx = (p.x - drag.startX) / view.zoom;
        const dy = (p.y - drag.startY) / view.zoom;
        if (!drag.moved) {
          if (Math.abs(p.x - drag.startX) + Math.abs(p.y - drag.startY) <= DRAG_THRESHOLD) return;
          remember();
          drag.moved = true;
        }
        for (const item of items) {
          const start = drag.starts.get(item.id);
          if (!start) continue;
          item.x = start.x + dx;
          item.y = start.y + dy;
          dirty.add(item.id);
        }
        break;
      }
      case "resize": {
        const w = toBoard(p.x, p.y);
        const { fixed, box, starts } = drag;
        if (!drag.moved) {
          remember();
          drag.moved = true;
        }
        if (drag.free) {
          const width = Math.max(80, Math.abs(w.x - fixed.x));
          const height = Math.max(50, Math.abs(w.y - fixed.y));
          const item = items.find((i) => starts.has(i.id));
          if (item) {
            item.x = w.x < fixed.x ? fixed.x - width : fixed.x;
            item.y = w.y < fixed.y ? fixed.y - height : fixed.y;
            item.width = width;
            item.height = height;
            dirty.add(item.id);
          }
          break;
        }
        const smallest = Math.min(...[...starts.values()].map((r) => Math.min(r.width, r.height)));
        const scale = Math.max(
          Math.abs(w.x - fixed.x) / box.width,
          Math.abs(w.y - fixed.y) / box.height,
          MIN_SIZE / smallest,
        );
        for (const item of items) {
          const start = starts.get(item.id);
          if (!start) continue;
          item.x = fixed.x + (start.x - fixed.x) * scale;
          item.y = fixed.y + (start.y - fixed.y) * scale;
          item.width = start.width * scale;
          item.height = start.height * scale;
          dirty.add(item.id);
        }
        break;
      }
      case "marquee": {
        marquee = {
          x: Math.min(drag.startX, p.x),
          y: Math.min(drag.startY, p.y),
          width: Math.abs(p.x - drag.startX),
          height: Math.abs(p.y - drag.startY),
        };
        const a = toBoard(marquee.x, marquee.y);
        const area = { x: a.x, y: a.y, width: marquee.width / view.zoom, height: marquee.height / view.zoom };
        selected.clear();
        for (const id of drag.base) selected.add(id);
        for (const item of items) if (overlaps(area, item)) selected.add(item.id);
        break;
      }
      case "crop":
        if (cropping) cropping.crop = dragCrop(drag, p);
        break;
    }
  }

  function onpointerup() {
    const done = drag;
    drag = null;
    panning = false;
    if (!done) return;
    if (done.kind === "pan" && !done.moved) selected.clear(); // a click on empty space
    if (done.kind === "marquee") marquee = null;
    if (dirty.size > 0) {
      save(dirty);
      dirty.clear();
    }
  }

  function bringToFront(ids: number[]) {
    let top = Math.max(0, ...items.map((i) => i.z));
    const moving = items.filter((i) => ids.includes(i.id)).sort((a, b) => a.z - b.z);
    // Already on top, in this order? Then there's nothing to change.
    const others = items.filter((i) => !ids.includes(i.id));
    if (moving.every((i) => others.every((o) => o.z < i.z))) return;
    remember();
    for (const item of moving) item.z = ++top;
    save(moving.map((i) => i.id));
  }

  function sendToBack(ids: number[]) {
    let bottom = Math.min(0, ...items.map((i) => i.z));
    const moving = items.filter((i) => ids.includes(i.id)).sort((a, b) => b.z - a.z);
    // Already behind everything? Then there's nothing to change.
    const others = items.filter((i) => !ids.includes(i.id));
    if (moving.every((i) => others.every((o) => o.z > i.z))) return;
    remember();
    for (const item of moving) item.z = --bottom;
    save(moving.map((i) => i.id));
  }

  // ---- Undo and redo ----
  // Each step keeps a copy of what was on the board before a change. Boards hold few items and
  // change one action at a time, so this stays small, and it's dropped when the board is closed.

  /** How many steps can be undone. */
  const MAX_UNDO = 100;
  let undoSteps: BoardItem[][] = [];
  let redoSteps: BoardItem[][] = [];
  /** Undoing and removing write to the database one at a time, so a quick Delete + Ctrl+Z stays in order. */
  let queue: Promise<unknown> = Promise.resolve();

  function snapshot(): BoardItem[] {
    return $state.snapshot(items) as BoardItem[];
  }

  /** Call before changing the board; `before` is what to go back to, if it isn't the board as it is now. */
  function remember(before: BoardItem[] = snapshot()) {
    undoSteps.push(before);
    if (undoSteps.length > MAX_UNDO) undoSteps.shift();
    redoSteps = [];
    showHistory();
  }

  function rememberIfChanged(before: BoardItem[]) {
    if (JSON.stringify(before) !== JSON.stringify(snapshot())) remember(before);
  }

  function showHistory() {
    canUndo = undoSteps.length > 0;
    canRedo = redoSteps.length > 0;
  }

  export function undo() {
    step(true);
  }

  export function redo() {
    step(false);
  }

  function step(backwards: boolean) {
    if (editingNote !== null || drag) return;
    cropping = null;
    queue = queue.then(async () => {
      // Looked up only now: the lists are replaced when items change or get new ids.
      const target = (backwards ? undoSteps : redoSteps).pop();
      if (!target) return;
      (backwards ? redoSteps : undoSteps).push(snapshot());
      try {
        await restore(target);
      } catch (e) {
        onerror(String(e));
      }
      showHistory();
    });
  }

  /** Makes the board (and the database) match an earlier copy of it. */
  async function restore(target: BoardItem[]) {
    const current = new Map(snapshot().map((item) => [item.id, item]));
    const wanted = new Set(target.map((item) => item.id));
    // An image renamed since then keeps its new path: the library has followed the file.
    const kept = target.map((item) => (current.has(item.id) ? { ...item, path: current.get(item.id)!.path } : item));

    const gone = [...current.keys()].filter((id) => !wanted.has(id));
    const back = kept.filter((item) => !current.has(item.id));
    const changed = kept.filter(
      (item) => current.has(item.id) && JSON.stringify(item) !== JSON.stringify(current.get(item.id)),
    );
    if (gone.length > 0) await removeBoardItems(boardId, gone);
    const restored = back.length > 0 ? await restoreBoardItems(boardId, back) : [];
    if (changed.length > 0) await updateBoardItems(boardId, changed);

    // An item that came back with a new id (see `restore_board_items`) must be found under it in
    // the rest of the history too.
    const newIds = new Map(back.map((item, i) => [item.id, restored[i].id]));
    const renumber = (list: BoardItem[]) => list.map((item) => ({ ...item, id: newIds.get(item.id) ?? item.id }));
    if ([...newIds].some(([before, after]) => before !== after)) {
      undoSteps = undoSteps.map(renumber);
      redoSteps = redoSteps.map(renumber);
    }
    items = renumber(kept);
    for (const id of [...selected]) if (!items.some((i) => i.id === id)) selected.delete(id);
    await loadFiles();
    report();
  }

  // ---- Cropping ----

  function startCrop(item: BoardItem) {
    if (item.path === null) return;
    selected.clear();
    selected.add(item.id);
    cropping = { id: item.id, full: uncroppedRect(item), crop: { ...(item.crop ?? FULL_CROP) } };
  }

  /** The crop after dragging one of its edges, corners, or the whole window. */
  function dragCrop(d: Extract<Drag, { kind: "crop" }>, p: { x: number; y: number }): Crop {
    const full = cropping!.full;
    const fx = (p.x - d.startX) / view.zoom / full.width;
    const fy = (p.y - d.startY) / view.zoom / full.height;
    let { x, y, w, h } = d.start;
    if (d.handle === "move") {
      x = Math.min(Math.max(0, x + fx), 1 - w);
      y = Math.min(Math.max(0, y + fy), 1 - h);
      return { x, y, w, h };
    }
    let left = x, top = y, right = x + w, bottom = y + h;
    if (d.handle.includes("w")) left = Math.min(Math.max(0, left + fx), right - MIN_CROP);
    if (d.handle.includes("e")) right = Math.max(Math.min(1, right + fx), left + MIN_CROP);
    if (d.handle.includes("n")) top = Math.min(Math.max(0, top + fy), bottom - MIN_CROP);
    if (d.handle.includes("s")) bottom = Math.max(Math.min(1, bottom + fy), top + MIN_CROP);
    return { x: left, y: top, w: right - left, h: bottom - top };
  }

  function applyCrop() {
    if (!cropping) return;
    const { id, full, crop } = cropping;
    const item = items.find((i) => i.id === id);
    cropping = null;
    if (!item) return;
    const before = snapshot();
    item.crop = isFullCrop(crop) ? null : crop;
    item.x = full.x + crop.x * full.width;
    item.y = full.y + crop.y * full.height;
    item.width = crop.w * full.width;
    item.height = crop.h * full.height;
    rememberIfChanged(before);
    save([id]);
  }

  function resetCrop(item: BoardItem) {
    remember();
    const full = uncroppedRect(item);
    Object.assign(item, { crop: null, ...full });
    save([item.id]);
  }

  // ---- Notes ----

  /** Adds a note at the middle of the view and starts editing it. */
  export function addNote() {
    const centre = toBoard(stageWidth / 2, stageHeight / 2);
    addNoteAt(centre.x, centre.y);
  }

  async function addNoteAt(x: number, y: number) {
    try {
      const note = await addBoardNote(boardId, "", x - NOTE_SIZE.width / 2, y - NOTE_SIZE.height / 2, NOTE_SIZE.width, NOTE_SIZE.height);
      remember();
      items.push(note);
      selected.clear();
      selected.add(note.id);
      freshNote = note.id;
      startEditing(note.id);
      report();
    } catch (e) {
      onerror(String(e));
    }
  }

  /** The board before the note being edited changed, to go back to with undo. */
  let beforeEditing: BoardItem[] = [];
  let noteSaveTimer: ReturnType<typeof setTimeout> | undefined;

  function startEditing(id: number) {
    beforeEditing = snapshot();
    editingNote = id;
  }

  /** Saves what's typed soon after each pause, so closing the app mid-sentence doesn't lose it. */
  function saveNoteSoon(id: number) {
    clearTimeout(noteSaveTimer);
    noteSaveTimer = setTimeout(() => save([id]), NOTE_SAVE_DELAY_MS);
  }

  function finishEditing() {
    const id = editingNote;
    editingNote = null;
    clearTimeout(noteSaveTimer);
    if (id === null) return;
    const note = items.find((i) => i.id === id);
    if (!note) return;
    if (id === freshNote) {
      // Adding the note was already remembered as one step, together with what's typed in it.
      if (!note.text?.trim()) {
        // An empty new note isn't kept, and there's nothing to undo either.
        undoSteps.pop();
        showHistory();
        remove([id], false);
      } else {
        save([id]);
      }
      freshNote = null;
      return;
    }
    if (beforeEditing.find((item) => item.id === id)?.text !== note.text) remember(beforeEditing);
    save([id]);
  }

  function focusEditor(textarea: HTMLTextAreaElement) {
    textarea.focus();
    textarea.select();
  }

  // ---- Arranging, removing, menus ----

  /** Tidies the selected items (or everything, if fewer than two are selected) into neat rows. */
  export function arrangeItems() {
    const targets = selectedItems.length >= 2 ? selectedItems : items;
    const before = snapshot();
    const placed = arrange(targets);
    for (const item of targets) Object.assign(item, placed.get(item.id));
    rememberIfChanged(before);
    save(placed.keys());
    if (targets === items) fitAll();
  }

  /** Takes items off the board. `undoable` is false for tidying up an empty new note. */
  function remove(ids: number[], undoable = true) {
    if (ids.length === 0) return;
    queue = queue.then(async () => {
      try {
        const before = snapshot();
        await removeBoardItems(boardId, ids);
        if (undoable) remember(before);
        items = items.filter((i) => !ids.includes(i.id));
        for (const id of ids) selected.delete(id);
        report();
      } catch (e) {
        onerror(String(e));
      }
    });
  }

  /** The board position under a point on screen (e.g. where something was dropped), or null if it's outside the board. */
  export function boardPointAt(clientX: number, clientY: number): { x: number; y: number } | null {
    const r = stage.getBoundingClientRect();
    if (clientX < r.left || clientX > r.right || clientY < r.top || clientY > r.bottom) return null;
    return toBoard(clientX - r.left, clientY - r.top);
  }

  /** Paths of the selected images (for tagging them). */
  export function selectedPaths(): string[] {
    return selectedItems.filter((i) => i.path !== null && !isMissing(i.path)).map((i) => i.path!);
  }

  /** The board's images in reading order (rows top to bottom, left to right), for paging through in the viewer. */
  function imagesInOrder() {
    const rowHeight = 50; // images whose tops are this close count as the same row
    return items
      .filter((i) => i.path !== null && !isMissing(i.path))
      .sort((a, b) => Math.round(a.y / rowHeight) - Math.round(b.y / rowHeight) || a.x - b.x)
      .map((i) => entryFor(i.path!));
  }

  function openInViewer(item: BoardItem) {
    const list = imagesInOrder();
    const index = list.findIndex((image) => image.path === item.path);
    if (index >= 0) onopen(list, index);
  }

  function itemAt(e: MouseEvent): BoardItem | undefined {
    // Pointer capture can make the event's own target the canvas, so look at what's under the pointer.
    const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-item]");
    return el ? items.find((i) => i.id === Number(el.dataset.item)) : undefined;
  }

  function ondblclick(e: MouseEvent) {
    if (cropping || !active) return;
    if ((e.target as HTMLElement).closest(".note-editor")) return;
    const item = itemAt(e);
    if (!item) {
      const p = stagePoint(e);
      const at = toBoard(p.x, p.y);
      addNoteAt(at.x, at.y);
    } else if (item.path === null) {
      startEditing(item.id);
    } else if (!isMissing(item.path)) {
      openInViewer(item);
    }
  }

  function oncontextmenu(e: MouseEvent) {
    e.preventDefault();
    if (!active || cropping) return;
    const item = itemAt(e);
    const at = toBoard(stagePoint(e).x, stagePoint(e).y);
    let menu: MenuItem[];
    if (!item) {
      menu = [
        { label: "Add a note here", action: () => addNoteAt(at.x, at.y) },
        { label: "Arrange everything", disabled: items.length < 2, action: () => (selected.clear(), arrangeItems()) },
        { label: "Fit everything", hint: "F", action: fitAll },
      ];
    } else {
      if (!selected.has(item.id)) {
        selected.clear();
        selected.add(item.id);
      }
      const ids = [...selected];
      const several = ids.length > 1;
      const removeLabel = several ? `Remove ${ids.length} from board` : "Remove from board";
      if (item.path === null) {
        menu = [
          { label: "Edit text", action: () => startEditing(item.id) },
          "separator",
          { label: "Bring to front", action: () => bringToFront(ids) },
          { label: "Send to back", action: () => sendToBack(ids) },
          "separator",
          { label: removeLabel, hint: "Del", danger: true, action: () => remove(ids) },
        ];
      } else {
        const missing = isMissing(item.path);
        const path = item.path;
        menu = [
          { label: "View full size", disabled: missing, action: () => openInViewer(item) },
          { label: "Crop", disabled: missing || several, action: () => startCrop(item) },
          ...(item.crop ? [{ label: "Show whole image", action: () => resetCrop(item) }] : []),
          "separator",
          { label: "Bring to front", action: () => bringToFront(ids) },
          { label: "Send to back", action: () => sendToBack(ids) },
          ...(several ? [{ label: "Arrange selected", action: arrangeItems }] : []),
          "separator",
          { label: `Show in ${fileManagerName}`, disabled: missing, action: () => revealItemInDir(path) },
          { label: removeLabel, hint: "Del", danger: true, action: () => remove(ids) },
        ];
      }
    }
    onmenu({ x: e.clientX, y: e.clientY, items: menu });
  }

  function onkeydown(e: KeyboardEvent) {
    if (!active || e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
    if (cropping) {
      if (e.key === "Enter") applyCrop();
      else if (e.key === "Escape") cropping = null;
      else return;
      e.preventDefault();
      return;
    }
    if (hasOtherModifier(e)) return;
    if (hasPrimaryModifier(e)) {
      const key = e.key.toLowerCase();
      if (key === "a") {
        for (const item of items) selected.add(item.id);
      } else if (key === "z") {
        step(!e.shiftKey); // with Shift: redo
      } else if (key === "y") {
        step(false);
      } else {
        return;
      }
      e.preventDefault();
      return;
    }
    switch (e.key) {
      case "Delete":
      case "Backspace":
        remove([...selected]);
        break;
      case "Escape":
        if (selected.size === 0) return;
        selected.clear();
        break;
      case "f":
      case "F":
        fitAll();
        break;
      default:
        return;
    }
    e.preventDefault();
  }

  const handles = ["nw", "ne", "sw", "se"];
  const cropHandles = ["nw", "n", "ne", "e", "se", "s", "sw", "w"];
</script>

<svelte:window {onkeydown} />

<div
  class="stage"
  class:panning
  bind:this={stage}
  bind:clientWidth={stageWidth}
  bind:clientHeight={stageHeight}
  role="application"
  aria-label="Mood board"
  style:background-size="{24 * view.zoom}px {24 * view.zoom}px"
  style:background-position="{view.x}px {view.y}px"
  {onpointerdown}
  {onpointermove}
  {onpointerup}
  onpointercancel={onpointerup}
  {ondblclick}
  {oncontextmenu}
>
  <div class="world" style:transform="translate({view.x}px, {view.y}px) scale({view.zoom})">
    {#each ordered as item (item.id)}
      {#if item.path !== null}
        {@const crop = cropping?.id === item.id ? FULL_CROP : (item.crop ?? FULL_CROP)}
        {@const rect = cropping?.id === item.id ? cropping.full : item}
        {@const entry = entryFor(item.path)}
        {@const sharper = cropping?.id === item.id ? null : sharperSource(item)}
        <div
          class="item image"
          class:cropping={cropping?.id === item.id}
          data-item={item.id}
          style:left="{rect.x}px"
          style:top="{rect.y}px"
          style:width="{rect.width}px"
          style:height="{rect.height}px"
          title={entry.name}
        >
          {#if isMissing(item.path) || broken.has(item.id)}
            <div class="missing">
              <span>{isMissing(item.path) ? "Image not found" : "Can't show this image"}</span>
              <span class="file">{entry.name}</span>
            </div>
          {:else}
            <!-- The whole image, shifted and sized so that only the cropped part shows. -->
            <div
              class="inner"
              style:width="{rect.width / crop.w}px"
              style:height="{rect.height / crop.h}px"
              style:transform="translate({(-crop.x * rect.width) / crop.w}px, {(-crop.y * rect.height) / crop.h}px)"
            >
              <img class="layer" src={thumbUrl(entry)} alt={entry.name} draggable="false" onerror={() => broken.add(item.id)} />
              {#if sharper}
                {#key sharper}
                  <img class="layer" src={sharper} alt="" draggable="false" />
                {/key}
              {/if}
            </div>
          {/if}
        </div>
      {:else if editingNote === item.id}
        <textarea
          class="item note note-editor"
          style:left="{item.x}px"
          style:top="{item.y}px"
          style:width="{item.width}px"
          style:height="{item.height}px"
          bind:value={item.text}
          placeholder="Type a note…"
          spellcheck="false"
          {@attach focusEditor}
          oninput={() => saveNoteSoon(item.id)}
          onblur={finishEditing}
          onkeydown={(e) => {
            e.stopPropagation();
            if (e.key === "Escape") finishEditing();
          }}
        ></textarea>
      {:else}
        <div
          class="item note"
          data-item={item.id}
          style:left="{item.x}px"
          style:top="{item.y}px"
          style:width="{item.width}px"
          style:height="{item.height}px"
        >
          {item.text}
        </div>
      {/if}
    {/each}
  </div>

  <!-- Drawn on top of the board at a fixed size, whatever the zoom. -->
  <div class="overlay">
    {#if cropping}
      {@const frame = toScreen({
        x: cropping.full.x + cropping.crop.x * cropping.full.width,
        y: cropping.full.y + cropping.crop.y * cropping.full.height,
        width: cropping.crop.w * cropping.full.width,
        height: cropping.crop.h * cropping.full.height,
      })}
      <div
        class="crop-window"
        data-crop-window
        style:left="{frame.x}px"
        style:top="{frame.y}px"
        style:width="{frame.width}px"
        style:height="{frame.height}px"
      >
        {#each cropHandles as h (h)}
          <span class="handle {h}" data-handle={h}></span>
        {/each}
      </div>
      <div class="crop-help">Drag the edges to crop · Enter to apply · Esc to cancel</div>
    {:else}
      {#each selectedItems as item (item.id)}
        {@const r = toScreen(item)}
        <div class="sel" style:left="{r.x}px" style:top="{r.y}px" style:width="{r.width}px" style:height="{r.height}px"></div>
      {/each}
      {#if selectionBox && editingNote === null}
        {@const box = toScreen(selectionBox)}
        {#if selectedItems.length > 1}
          <div class="group" style:left="{box.x}px" style:top="{box.y}px" style:width="{box.width}px" style:height="{box.height}px"></div>
        {/if}
        {#each handles as h (h)}
          <span
            class="handle {h}"
            data-handle={h}
            style:left="{h.includes('w') ? box.x : box.x + box.width}px"
            style:top="{h.includes('n') ? box.y : box.y + box.height}px"
          ></span>
        {/each}
      {/if}
    {/if}
    {#if marquee}
      <div class="marquee" style:left="{marquee.x}px" style:top="{marquee.y}px" style:width="{marquee.width}px" style:height="{marquee.height}px"></div>
    {/if}
  </div>

  {#if loaded && items.length === 0}
    <div class="empty">
      <p>This board is empty.</p>
      <p class="hint">
        Select images in the grid and right-click → <em>Add to board</em>, or drag them onto the board in the sidebar.
        Double-click here to add a note.
      </p>
    </div>
  {/if}
</div>

<style>
  .stage {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    cursor: grab;
    touch-action: none;
    background-color: var(--bg);
    /* A faint dot grid that moves and scales with the board, so you can see it moving. */
    background-image: radial-gradient(circle, #2a2a30 1px, transparent 1.2px);
  }
  .stage.panning {
    cursor: grabbing;
  }
  .world {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }
  .item {
    position: absolute;
    overflow: hidden;
    box-sizing: border-box;
    cursor: move;
  }
  .image {
    background: var(--tile-bg);
  }
  /* While cropping, the whole image is shown; the crop window darkens everything outside it. */
  .image.cropping {
    overflow: visible;
    cursor: default;
  }
  .inner {
    position: absolute;
    top: 0;
    left: 0;
    transform-origin: 0 0;
  }
  .layer {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: cover;
    user-select: none;
    pointer-events: none;
  }
  .missing {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    height: 100%;
    padding: 8px;
    font-size: 14px;
    text-align: center;
    color: var(--text-dim);
    border: 2px dashed var(--border);
  }
  .missing .file {
    font-size: 12px;
    word-break: break-all;
  }
  .note {
    padding: 12px 14px;
    font: inherit;
    font-size: 18px;
    line-height: 1.35;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: #f3e8c4;
    background: #3a331f;
    border: 1px solid #5a4e2c;
    border-radius: 6px;
  }
  textarea.note {
    resize: none;
    outline: 2px solid var(--accent);
    cursor: text;
    user-select: text;
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .sel {
    position: absolute;
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .group {
    position: absolute;
    outline: 1px dashed var(--accent);
    outline-offset: 5px;
  }
  .handle {
    position: absolute;
    width: 11px;
    height: 11px;
    margin: -5px 0 0 -5px;
    background: #fff;
    border: 2px solid var(--accent);
    border-radius: 2px;
    pointer-events: auto;
  }
  .handle.nw,
  .handle.se {
    cursor: nwse-resize;
  }
  .handle.ne,
  .handle.sw {
    cursor: nesw-resize;
  }
  .marquee {
    position: absolute;
    border: 1px dashed var(--accent);
    background: rgba(91, 157, 255, 0.12);
  }
  /* Cropping: the crop window shows the kept part at full brightness. */
  .crop-window {
    position: absolute;
    outline: 2px solid #fff;
    box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.35);
    pointer-events: auto;
    cursor: move;
  }
  .crop-window .handle {
    border-color: #fff;
    background: var(--accent);
  }
  .crop-window .nw { left: 0; top: 0; }
  .crop-window .n { left: 50%; top: 0; cursor: ns-resize; }
  .crop-window .ne { left: 100%; top: 0; }
  .crop-window .e { left: 100%; top: 50%; cursor: ew-resize; }
  .crop-window .se { left: 100%; top: 100%; }
  .crop-window .s { left: 50%; top: 100%; cursor: ns-resize; }
  .crop-window .sw { left: 0; top: 100%; }
  .crop-window .w { left: 0; top: 50%; cursor: ew-resize; }
  .crop-help {
    position: absolute;
    left: 50%;
    bottom: 14px;
    transform: translateX(-50%);
    padding: 5px 12px;
    font-size: 12px;
    color: var(--text);
    background: rgba(20, 20, 22, 0.85);
    border: 1px solid var(--border);
    border-radius: 6px;
  }
  .empty {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px;
    text-align: center;
    color: var(--text-dim);
    pointer-events: none;
  }
  .empty p {
    margin: 4px 0;
    max-width: 460px;
  }
  .empty em {
    color: var(--text);
    font-style: normal;
  }
  .hint {
    font-size: 13px;
  }
</style>
