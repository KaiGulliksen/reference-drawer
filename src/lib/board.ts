// Geometry for mood boards: where new images go, tidying up ("Arrange"), and fitting
// everything on screen. All in board units (pixels at 100% zoom).

import type { BoardItem, Crop } from "./api";

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Height new images get when added to a board. */
export const NEW_IMAGE_HEIGHT = 300;
/** Space between images placed or arranged by the app. */
export const BOARD_GAP = 16;
export const NOTE_SIZE = { width: 240, height: 140 };

/** The smallest rectangle around all of `rects`, or null if there are none. */
export function bounds(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null;
  let left = Infinity, top = Infinity, right = -Infinity, bottom = -Infinity;
  for (const r of rects) {
    left = Math.min(left, r.x);
    top = Math.min(top, r.y);
    right = Math.max(right, r.x + r.width);
    bottom = Math.max(bottom, r.y + r.height);
  }
  return { x: left, y: top, width: right - left, height: bottom - top };
}

export function overlaps(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
}

/**
 * Places new images in rows below what's already on the board, each `NEW_IMAGE_HEIGHT` tall
 * and as wide as its shape needs. `ratios` are width ÷ height (1 when unknown).
 */
export function placeNewImages(existing: Rect[], ratios: number[]): Rect[] {
  const area = bounds(existing);
  const left = area?.x ?? 0;
  const rowWidth = Math.max(area?.width ?? 0, 1600);
  let x = left;
  let y = area ? area.y + area.height + BOARD_GAP * 3 : 0;
  return ratios.map((ratio) => {
    const width = NEW_IMAGE_HEIGHT * Math.min(Math.max(ratio || 1, 0.2), 5);
    if (x > left && x + width > left + rowWidth) {
      x = left;
      y += NEW_IMAGE_HEIGHT + BOARD_GAP;
    }
    const placed = { x, y, width, height: NEW_IMAGE_HEIGHT };
    x += width + BOARD_GAP;
    return placed;
  });
}

/**
 * Tidies items into neat rows, keeping their sizes, starting where the top-left one was.
 * Taller items go first, so each row is roughly even. Returns new positions by item id.
 */
export function arrange(items: (Rect & { id: number })[]): Map<number, { x: number; y: number }> {
  const result = new Map<number, { x: number; y: number }>();
  const area = bounds(items);
  if (!area) return result;
  const totalArea = items.reduce((sum, r) => sum + (r.width + BOARD_GAP) * (r.height + BOARD_GAP), 0);
  // Aim for a landscape block, about as wide as a screen is compared to its height.
  const widest = Math.max(...items.map((r) => r.width));
  const rowWidth = Math.max(widest, Math.sqrt(totalArea * 1.6));
  const order = [...items].sort((a, b) => b.height - a.height || a.y - b.y || a.x - b.x);
  let x = area.x;
  let y = area.y;
  let rowHeight = 0;
  for (const item of order) {
    if (x > area.x && x + item.width > area.x + rowWidth) {
      x = area.x;
      y += rowHeight + BOARD_GAP;
      rowHeight = 0;
    }
    result.set(item.id, { x, y });
    x += item.width + BOARD_GAP;
    rowHeight = Math.max(rowHeight, item.height);
  }
  return result;
}

/** The zoom and pan that show `area` in a `viewWidth` × `viewHeight` screen, with a margin. */
export function fitView(area: Rect, viewWidth: number, viewHeight: number, margin = 40) {
  const zoom = Math.min(
    (viewWidth - margin * 2) / Math.max(area.width, 1),
    (viewHeight - margin * 2) / Math.max(area.height, 1),
    2,
  );
  return {
    zoom,
    x: viewWidth / 2 - (area.x + area.width / 2) * zoom,
    y: viewHeight / 2 - (area.y + area.height / 2) * zoom,
  };
}

/** The whole image's rectangle for a (possibly cropped) item: the crop is a window onto it. */
export function uncroppedRect(item: BoardItem): Rect {
  const crop = item.crop ?? FULL_CROP;
  const width = item.width / crop.w;
  const height = item.height / crop.h;
  return { x: item.x - crop.x * width, y: item.y - crop.y * height, width, height };
}

export const FULL_CROP: Crop = { x: 0, y: 0, w: 1, h: 1 };

export function isFullCrop(crop: Crop | null): boolean {
  return !crop || (crop.x <= 0.001 && crop.y <= 0.001 && crop.w >= 0.999 && crop.h >= 0.999);
}
