// Reading what is dropped onto the window from outside the app: images (from a web browser,
// another app or a file manager), images that come as just a web address, and folders.
//
// Drops are handled by the web view itself (not Tauri), because that's the only way to receive
// images dragged out of a browser: those aren't files on disk, so Tauri's own drop handling
// ignores them. Dragging tiles inside the app uses pointer events instead (see Grid.svelte).

import { importImage, importImageUrl, type Imported } from "./api";

/** One dropped image: its contents (`file`), or only where it is on the web (`url`). */
export type DroppedImage = { file: File } | { url: string };

export interface Dropped {
  images: DroppedImage[];
  /** Dropped folders, to add as workspaces. */
  folders: File[];
}

/**
 * Whether a drag (by the kinds of data it carries) may hold something the app can take.
 * What exactly is in it is only known once it's dropped.
 */
export function canTake(types: readonly string[]): boolean {
  return types.includes("Files") || types.includes("text/uri-list");
}

/** Whether a drag only carries files (from a file manager), so it could be a folder. */
export const onlyFiles = (types: readonly string[]) => types.length > 0 && types.every((t) => t === "Files");

/** Shortcut files that browsers attach when a link is dragged: not images. */
const LINK_FILE = /\.(url|lnk|webloc|desktop)$/i;

/** What was dropped. Must be called during the drop event (the data is gone afterwards). */
export function readDrop(data: DataTransfer): Dropped {
  const files: File[] = [];
  const folders: File[] = [];
  for (const item of data.items) {
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (!file) continue;
    if (item.webkitGetAsEntry()?.isDirectory) folders.push(file);
    else if (!LINK_FILE.test(file.name)) files.push(file);
  }
  if (files.length > 0 || folders.length > 0) return { images: files.map((file) => ({ file })), folders };

  // Only a web address: the browser didn't hand over the picture itself.
  const url = imageUrl(data.getData("text/html"), data.getData("text/uri-list"));
  return { images: url ? [{ url }] : [], folders: [] };
}

/** Saves a dropped image in a workspace's folder (downloading it first if it's only an address). */
export async function saveDropped(workspaceId: number, image: DroppedImage): Promise<Imported> {
  if ("file" in image) {
    return importImage(workspaceId, image.file.name, new Uint8Array(await image.file.arrayBuffer()));
  }
  if (image.url.startsWith("data:")) {
    // The picture is written into the address itself (some sites do that for small images).
    const bytes = await (await fetch(image.url)).arrayBuffer();
    return importImage(workspaceId, "image", new Uint8Array(bytes));
  }
  return importImageUrl(workspaceId, image.url);
}

const USABLE_URL = /^(https?|data):/i;

/**
 * The address of the dragged image. Browsers describe a dragged image as a bit of web page
 * (`<img src=…>`) and as a list of addresses. The page bit is the better guide: for an image
 * inside a link, the address list holds the link (often a web page), not the image.
 */
function imageUrl(html: string, uriList: string): string | null {
  if (html) {
    // A parsed document that isn't shown loads nothing and runs no scripts.
    const img = new DOMParser().parseFromString(html, "text/html").querySelector("img");
    const src = img && (largestInSrcset(img.getAttribute("srcset")) ?? img.getAttribute("src"));
    if (src && USABLE_URL.test(src)) return src;
  }
  const first = uriList
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line && !line.startsWith("#"));
  return first && USABLE_URL.test(first) ? first : null;
}

/**
 * The biggest version of an image offered in a `srcset` ("small.jpg 480w, big.jpg 1600w" or
 * "a.jpg 1x, b.jpg 2x"): sites often show a small one, and references are better sharp.
 */
function largestInSrcset(srcset: string | null): string | null {
  let best: string | null = null;
  let bestSize = 0;
  for (const candidate of (srcset ?? "").split(/,\s+/)) {
    const [url, descriptor = "1x"] = candidate.trim().split(/\s+/);
    const size = parseFloat(descriptor);
    if (url && USABLE_URL.test(url) && size > bestSize) {
      best = url;
      bestSize = size;
    }
  }
  return best;
}
