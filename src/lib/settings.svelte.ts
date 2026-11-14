// User preferences, remembered between launches.

export type SortKey = "name" | "modified" | "size";
export type TileFit = "cover" | "contain";
/** "grid": square tiles in rows. "masonry": columns of tiles shaped like their images. */
export type GridLayout = "grid" | "masonry";

interface Settings {
  /** The workspace shown at launch. */
  activeWorkspaceId: number | null;
  sidebarCollapsed: boolean;
  /** From before workspaces existed; turned into the first workspace, then cleared. */
  lastFolder: string | null;
  /** From before workspaces existed (now a per-workspace setting). */
  recursive: boolean;
  sortKey: SortKey;
  sortAscending: boolean;
  /** Target thumbnail width in the grid, in CSS pixels. */
  tileSize: number;
  /** "cover" crops thumbnails to fill square tiles, "contain" shows the whole image (grid layout only). */
  tileFit: TileFit;
  layout: GridLayout;
  /** Show each image's tags on its thumbnail. */
  showTileTags: boolean;
  /** Show each image's file name on its thumbnail. */
  showTileNames: boolean;
  /** The key that opens the tag editor (a `KeyboardEvent.key` value, lower-cased). */
  tagHotkey: string;
}

const STORAGE_KEY = "reference-library.settings";

const defaults: Settings = {
  activeWorkspaceId: null,
  sidebarCollapsed: false,
  lastFolder: null,
  recursive: false,
  sortKey: "name",
  sortAscending: true,
  tileSize: 180,
  tileFit: "cover",
  layout: "masonry",
  showTileTags: true,
  showTileNames: false,
  tagHotkey: "t",
};

/** Keys the app already uses for something else, so they can't be the tag shortcut. */
export const RESERVED_KEYS: Record<string, string> = {
  arrowleft: "moving between images",
  arrowright: "moving between images",
  arrowup: "moving between images",
  arrowdown: "moving between images",
  pageup: "jumping through the grid",
  pagedown: "jumping through the grid",
  home: "going to the first image",
  end: "going to the last image",
  enter: "opening images",
  escape: "closing things",
  delete: "taking things off a board",
  backspace: "taking things off a board",
  tab: "moving between controls",
  " ": "scrolling",
  a: "previous image in the viewer",
  d: "next image in the viewer",
  f: "fitting the image in the viewer",
  "0": "fitting the image in the viewer",
  "1": "actual size in the viewer",
  "+": "zooming in",
  "=": "zooming in",
  "-": "zooming out",
  f5: "refreshing the folder",
};

/** How to show a key to people: "T", "F2", "Space". */
export function keyLabel(key: string): string {
  if (key === " ") return "Space";
  return key.length === 1 ? key.toUpperCase() : key.charAt(0).toUpperCase() + key.slice(1);
}

function load(): Settings {
  try {
    return { ...defaults, ...JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") };
  } catch {
    return { ...defaults };
  }
}

export const settings = $state<Settings>(load());

// Save whenever any setting changes.
$effect.root(() => {
  $effect(() => {
    const json = JSON.stringify(settings);
    try {
      localStorage.setItem(STORAGE_KEY, json);
    } catch {
      // Storage unavailable; settings just won't persist.
    }
  });
});
