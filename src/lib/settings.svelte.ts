// User preferences, remembered between launches.

export type SortKey = "name" | "modified" | "size";
export type TileFit = "cover" | "contain";

interface Settings {
  lastFolder: string | null;
  recursive: boolean;
  sortKey: SortKey;
  sortAscending: boolean;
  /** Target thumbnail width in the grid, in CSS pixels. */
  tileSize: number;
  /** "cover" crops thumbnails to fill square tiles, "contain" shows the whole image. */
  tileFit: TileFit;
}

const STORAGE_KEY = "reference-library.settings";

const defaults: Settings = {
  lastFolder: null,
  recursive: false,
  sortKey: "name",
  sortAscending: true,
  tileSize: 180,
  tileFit: "cover",
};

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
