// Where each thumbnail goes, for the two grid layouts:
//   "grid":    square tiles in rows, left to right.
//   "masonry": columns of tiles shaped like their images; each image goes into whichever
//              column is currently shortest, so the columns stay roughly even.
// Everything else (scrolling, keyboard movement, remembering your place) works from the
// rectangles worked out here, so it doesn't need to know which layout is in use.

import type { ImageEntry } from "./api";
import type { GridLayout } from "./settings.svelte";

export const GAP = 6;
export const PADDING = 12;

/**
 * Masonry tiles keep their height between these multiples of their width, so very wide
 * panoramas don't become slivers and very tall images don't fill the whole screen.
 */
const MIN_RATIO = 0.3;
const MAX_RATIO = 3;

export interface Layout {
  /** Position and size of each tile, four numbers each: tile i is x, y, width, height at [4i … 4i+3]. */
  rects: Float64Array;
  columns: number;
  /** Width of one column. */
  cell: number;
  totalHeight: number;
  /** Which column each tile is in, and where in that column. */
  column: Int32Array;
  rowInColumn: Int32Array;
  /** The tiles in each column, top to bottom. */
  byColumn: number[][];
}

export function computeLayout(images: ImageEntry[], width: number, tileSize: number, mode: GridLayout): Layout {
  const columns = Math.max(1, Math.floor((width - PADDING * 2 + GAP) / (tileSize + GAP)));
  // Tiles stretch slightly so the columns fill the full width.
  const cell = Math.max(1, (width - PADDING * 2 - GAP * (columns - 1)) / columns);
  const count = images.length;
  const rects = new Float64Array(count * 4);
  const column = new Int32Array(count);
  const rowInColumn = new Int32Array(count);
  const byColumn: number[][] = Array.from({ length: columns }, () => []);
  const bottoms = new Float64Array(columns).fill(PADDING);

  for (let i = 0; i < count; i++) {
    let col: number;
    let tileHeight: number;
    if (mode === "masonry") {
      col = 0;
      for (let c = 1; c < columns; c++) if (bottoms[c] < bottoms[col]) col = c;
      tileHeight = cell * ratioOf(images[i]);
    } else {
      col = i % columns;
      tileHeight = cell;
    }
    const x = PADDING + col * (cell + GAP);
    const y = bottoms[col];
    // Written one by one: a small list per tile would be thrown away again 20,000 times over.
    rects[i * 4] = x;
    rects[i * 4 + 1] = y;
    rects[i * 4 + 2] = cell;
    rects[i * 4 + 3] = tileHeight;
    column[i] = col;
    rowInColumn[i] = byColumn[col].length;
    byColumn[col].push(i);
    bottoms[col] = y + tileHeight + GAP;
  }

  const tallest = count > 0 ? Math.max(...bottoms) - GAP : PADDING;
  return { rects, columns, cell, totalHeight: tallest + PADDING, column, rowInColumn, byColumn };
}

/** Height ÷ width for a masonry tile (square when the image's size is unknown). */
function ratioOf(image: ImageEntry): number {
  if (!image.width || !image.height) return 1;
  return Math.min(MAX_RATIO, Math.max(MIN_RATIO, image.height / image.width));
}

export const tileTop = (layout: Layout, i: number) => layout.rects[i * 4 + 1];
export const tileHeight = (layout: Layout, i: number) => layout.rects[i * 4 + 3];

/** The tiles that overlap the band from `top` to `bottom` (in page pixels). */
export function tilesBetween(layout: Layout, top: number, bottom: number): number[] {
  const found: number[] = [];
  const { rects } = layout;
  for (let i = 0; i < rects.length / 4; i++) {
    const y = rects[i * 4 + 1];
    if (y <= bottom && y + rects[i * 4 + 3] >= top) found.push(i);
  }
  return found;
}

/** The first tile (in image order) that is still visible when scrolled to `top`. */
export function firstTileFrom(layout: Layout, top: number): number {
  const { rects } = layout;
  for (let i = 0; i < rects.length / 4; i++) {
    if (rects[i * 4 + 1] + rects[i * 4 + 3] > top) return i;
  }
  return 0;
}

/**
 * The tile the keyboard moves to from tile `from`.
 * ← → go to the previous / next image. ↑ ↓ and Page Up / Down stay in the same column,
 * like moving up and down in a list. `pageHeight` is how far Page Up / Down jump.
 */
export function neighbour(layout: Layout, from: number, key: string, pageHeight: number): number {
  const count = layout.rects.length / 4;
  if (count === 0) return -1;
  if (from < 0) return 0;
  const col = layout.byColumn[layout.column[from]];
  const row = layout.rowInColumn[from];
  switch (key) {
    case "ArrowRight":
      return Math.min(count - 1, from + 1);
    case "ArrowLeft":
      return Math.max(0, from - 1);
    case "ArrowDown":
      // Below the last tile in a column, go to the very last image (like Explorer does).
      return row + 1 < col.length ? col[row + 1] : count - 1;
    case "ArrowUp":
      return row > 0 ? col[row - 1] : from;
    case "PageDown":
    case "PageUp": {
      const target = tileTop(layout, from) + (key === "PageDown" ? pageHeight : -pageHeight);
      // The tile in this column whose top is closest to one screen further on.
      let best = from;
      for (const i of col) {
        if (Math.abs(tileTop(layout, i) - target) < Math.abs(tileTop(layout, best) - target)) best = i;
      }
      return best;
    }
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return from;
  }
}
