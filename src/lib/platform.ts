// Differences between Windows, macOS and Linux that the interface needs to know about.
// Keep platform checks in this file so the rest of the app doesn't need to care.

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
