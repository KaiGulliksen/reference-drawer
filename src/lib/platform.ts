// Differences between Windows, macOS and Linux that the interface needs to know about.
// Keep platform checks in this file so the rest of the app doesn't need to care.

import { listen } from "@tauri-apps/api/event";

const userAgent = typeof navigator === "undefined" ? "" : navigator.userAgent;

export const isMac = /Macintosh|Mac OS X/.test(userAgent);
export const isLinux = !isMac && /Linux/.test(userAgent);
export const isWindows = !isMac && !isLinux;

/**
 * Whether the main shortcut modifier is held: Cmd on macOS, Ctrl elsewhere.
 * (On a Mac, Ctrl+click means right-click, so it must not count there.)
 */
export function hasPrimaryModifier(e: KeyboardEvent | MouseEvent): boolean {
  return isMac ? e.metaKey : e.ctrlKey;
}

/** Whether a modifier other than the primary one (and Shift) is held. */
export function hasOtherModifier(e: KeyboardEvent | MouseEvent): boolean {
  return e.altKey || (isMac ? e.ctrlKey : e.metaKey);
}

/** How to write the primary modifier in hints: "Ctrl" or "⌘". */
export const primaryKeyLabel = isMac ? "⌘" : "Ctrl";

/** Joins the primary modifier and a key for display: "Ctrl+B" or "⌘B". */
export function shortcutLabel(key: string): string {
  return isMac ? `⌘${key}` : `Ctrl+${key}`;
}

/**
 * Shortcuts that belong to the web view rather than to an app: reload, find, print, save page, …
 * The app is a single page in a web view, so these would reload it (losing the current filter
 * and selection) or open browser tools that make no sense here. The list leaves out what people
 * rely on: copy and paste, and on a Mac the system's own Cmd+Q, Cmd+W and Cmd+H.
 */
export function isBrowserShortcut(e: KeyboardEvent): boolean {
  if (e.key === "F3" || e.key === "F5" || e.key === "F7") return true;
  if (e.altKey && (e.key === "ArrowLeft" || e.key === "ArrowRight")) return true; // history
  return hasPrimaryModifier(e) && !hasOtherModifier(e) && "rfgpsoujld".includes(e.key.toLowerCase());
}

/** F5 or Ctrl+R (⌘R): in this app, these rescan the folder instead of reloading the page. */
export function isRefreshShortcut(e: KeyboardEvent): boolean {
  return e.key === "F5" || (hasPrimaryModifier(e) && !hasOtherModifier(e) && e.key.toLowerCase() === "r");
}

/** What the system's file browser is called. */
export const fileManagerName = isMac ? "Finder" : isWindows ? "Explorer" : "file manager";

/**
 * Whether file names that differ only in capital letters are the same file.
 * True on Windows and macOS (by default); false on Linux.
 */
export const pathsIgnoreCase = !isLinux;

/** A version of a path that is equal for two paths pointing at the same file. */
export function pathKey(path: string): string {
  return pathsIgnoreCase ? path.toLowerCase() : path;
}

/** The last part of a path (a file or folder name), for either kind of slash. */
export function baseName(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).pop() ?? path;
}

// ---- Where dropped folders are (Windows only) ----
// Web pages aren't told where dropped files are on disk. WebView2 (Windows) can tell the app,
// though: posting a message with the dropped items attached makes the Rust side (platform.rs)
// send back their paths. macOS and Linux have no such feature, so folders are added with + there.

/** Must match DROPPED_PATHS_MESSAGE and DROPPED_PATHS_EVENT in platform.rs. */
const DROPPED_PATHS_MESSAGE = "reference-library:dropped-paths";
const DROPPED_PATHS_EVENT = "dropped-paths";

interface WebView2 {
  postMessageWithAdditionalObjects(message: string, objects: ArrayLike<File>): void;
}

const webView2 =
  typeof window === "undefined" ? undefined : (window as { chrome?: { webview?: WebView2 } }).chrome?.webview;

/** Whether dropping a folder onto the window can add it as a workspace. */
export const canDropFolders = isWindows && typeof webView2?.postMessageWithAdditionalObjects === "function";

/** The full paths of dropped folders (or files), or an empty list where that can't be found out. */
export async function droppedPaths(files: File[]): Promise<string[]> {
  if (!canDropFolders || files.length === 0) return [];
  return new Promise((resolve) => {
    let done = false;
    const finish = (paths: string[]) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      stop.then((unlisten) => unlisten());
      resolve(paths);
    };
    const timer = setTimeout(() => finish([]), 3000); // the answer normally takes milliseconds
    const stop = listen<string[]>(DROPPED_PATHS_EVENT, (event) => finish(event.payload));
    stop.then(() => webView2!.postMessageWithAdditionalObjects(DROPPED_PATHS_MESSAGE, files));
  });
}
