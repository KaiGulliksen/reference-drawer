<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import ContextMenu from "$lib/ContextMenu.svelte";
  import Grid from "$lib/Grid.svelte";
  import SettingsDialog from "$lib/SettingsDialog.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import TagEditor from "$lib/TagEditor.svelte";
  import Viewer from "$lib/Viewer.svelte";
  import { scanWorkspace, type ImageEntry, type Workspace } from "$lib/api";
  import type { MenuRequest } from "$lib/menu";
  import { baseName, fileManagerName, hasOtherModifier, hasPrimaryModifier, primaryKeyLabel, shortcutLabel } from "$lib/platform";
  import { keyLabel, settings, type SortKey } from "$lib/settings.svelte";
  import { tags, type TagItem } from "$lib/tags.svelte";
  import { workspaces } from "$lib/workspaces.svelte";

  let ready = $state(false);
  let activeId = $state<number | null>(null);
  const active = $derived(workspaces.list.find((w) => w.id === activeId) ?? null);

  // `$state.raw` because the list can be huge and is only ever replaced, never edited in place.
  let images = $state.raw<ImageEntry[]>([]);
  let loading = $state(false);
  /** Why the active workspace can't be shown (usually: its folder was moved or its drive unplugged). */
  let scanError = $state<string | null>(null);
  /** Errors from other actions, shown in a dismissible bar. */
  let error = $state<string | null>(null);
  /** A short message in the status bar that goes away by itself. */
  let notice = $state<string | null>(null);
  let noticeTimer: ReturnType<typeof setTimeout> | undefined;

  /** Paths of the selected images. */
  const selection = new SvelteSet<string>();
  /** The image the keyboard is on (and that Enter opens). */
  let focusPath = $state<string | null>(null);
  let viewing = $state<number | null>(null);

  // Tag filter: show only images with all of these tags, or only untagged images.
  const filterTags = new SvelteSet<number>();
  let filterUntagged = $state(false);
  let tagSearch = $state("");

  let dragHover = $state(false);
  let menu = $state<MenuRequest | null>(null);
  /** The tag editor, and the images it is editing (fixed when it opens). */
  let editor = $state<{ paths: string[]; subject: string } | null>(null);
  let settingsOpen = $state(false);
  let renamingId = $state<number | null>(null);
  let renamingTagId = $state<number | null>(null);
  const missing = new SvelteSet<number>();
  let grid = $state<Grid | undefined>();

  // Remembered per workspace while the app is open, so switching back is instant
  // and returns you to the same place.
  const scanCache = new Map<number, ImageEntry[]>();
  const viewState = new Map<number, { topIndex: number; selected: string[]; focusPath: string | null }>();
  let scanToken = 0;

  const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

  const sorted = $derived.by(() => {
    const direction = settings.sortAscending ? 1 : -1;
    const key = settings.sortKey;
    return [...images].sort((a, b) => {
      const diff =
        key === "name" ? collator.compare(a.name, b.name) : key === "size" ? a.size - b.size : a.modified - b.modified;
      return direction * (diff || collator.compare(a.path, b.path));
    });
  });

  const filtering = $derived(filterUntagged || filterTags.size > 0);

  /** What the grid shows: the sorted images, narrowed down by the tag filter. */
  const filtered = $derived.by(() => {
    if (filterUntagged) return sorted.filter((i) => tags.idsOf(i.path).length === 0);
    if (filterTags.size === 0) return sorted;
    const wanted = [...filterTags];
    return sorted.filter((i) => {
      const ids = tags.idsOf(i.path);
      return wanted.every((id) => ids.includes(id));
    });
  });

  /** How many of the workspace's images (in view, ignoring the filter) have each tag. */
  const usage = $derived.by(() => {
    const counts = new Map<number, number>();
    for (const image of images) {
      for (const id of tags.idsOf(image.path)) counts.set(id, (counts.get(id) ?? 0) + 1);
    }
    return counts;
  });

  const tagItems = $derived<TagItem[]>(
    tags.list.filter((t) => usage.has(t.id)).map((t) => ({ id: t.id, name: t.name, count: usage.get(t.id) ?? 0 })),
  );
  const untaggedCount = $derived(images.filter((i) => tags.idsOf(i.path).length === 0).length);

  // The focus is kept by file path, so it survives re-sorting, filtering and refreshing.
  const focus = $derived(focusPath ? filtered.findIndex((i) => i.path === focusPath) : -1);

  // A fresh grid (starting from the remembered position) for each workspace, sort order and filter.
  const gridKey = $derived(
    `${activeId}|${settings.sortKey}|${settings.sortAscending}|${active?.recursive}|${filterUntagged}|${[...filterTags].sort()}`,
  );

  const tagKey = $derived(keyLabel(settings.tagHotkey));

  $effect(() => {
    getCurrentWindow().setTitle(active ? `${active.name} – Reference Library` : "Reference Library");
  });

  // Images that drop out of view (filtered out, deleted, …) also drop out of the selection.
  $effect(() => {
    const shown = new Set(filtered.map((i) => i.path));
    if (loading && images.length === 0) return; // still opening the workspace
    untrack(() => {
      for (const path of [...selection]) if (!shown.has(path)) selection.delete(path);
    });
  });

  // If the image open in the viewer drops out of view (say it was just tagged while showing
  // "Untagged"), move on to the next one; close the viewer when nothing is left.
  $effect(() => {
    const count = filtered.length;
    untrack(() => {
      if (viewing !== null && viewing >= count) viewing = count > 0 ? count - 1 : null;
    });
  });

  // A tag merged into another or deleted can't stay in the filter.
  $effect(() => {
    const known = new Set(tags.list.map((t) => t.id));
    untrack(() => {
      for (const id of [...filterTags]) if (!known.has(id)) filterTags.delete(id);
    });
  });

  function showNotice(text: string, ms = 2500) {
    notice = text;
    clearTimeout(noticeTimer);
    noticeTimer = setTimeout(() => (notice = null), ms);
  }

  // ---- Workspaces ----

  async function openWorkspace(id: number) {
    if (id === activeId) return;
    rememberSelection();
    activeId = id;
    settings.activeWorkspaceId = id;
    viewing = null;
    editor = null;
    scanError = null;
    clearFilter();
    tagSearch = "";
    images = scanCache.get(id) ?? [];
    const saved = viewState.get(id);
    selection.clear();
    for (const path of saved?.selected ?? []) selection.add(path);
    focusPath = saved?.focusPath ?? null;
    tags.load(id).catch((e) => (error = String(e)));
    await refresh();
  }

  /** Remembers where you were in the current workspace, for when you come back to it. */
  function rememberSelection() {
    if (activeId === null) return;
    viewState.set(activeId, {
      // A position inside a filtered view means nothing once the filter is gone.
      topIndex: filtering ? 0 : (grid?.topIndex() ?? 0),
      selected: [...selection],
      focusPath,
    });
  }

  function forgetPosition() {
    if (activeId !== null) viewState.set(activeId, { topIndex: 0, selected: [...selection], focusPath });
  }

  /** Rescans the active workspace's folder (which also keeps tags with renamed or moved images). */
  async function refresh() {
    const workspace = active;
    if (!workspace) return;
    const token = ++scanToken; // ignore the result if another scan started meanwhile
    loading = true;
    try {
      const result = await scanWorkspace(workspace.id);
      if (token !== scanToken) return;
      scanError = null;
      missing.delete(workspace.id);
      if (!sameImages(images, result.images)) images = result.images;
      scanCache.set(workspace.id, result.images);
      if (result.relinked > 0) {
        // Some tags moved to their images' new names or places.
        tags.load(workspace.id).catch((e) => (error = String(e)));
        showNotice(
          result.relinked === 1
            ? "Kept the tags of 1 renamed or moved image."
            : `Kept the tags of ${result.relinked} renamed or moved images.`,
          5000,
        );
      }
    } catch (e) {
      if (token !== scanToken) return;
      scanError = String(e);
      missing.add(workspace.id);
      images = [];
      scanCache.delete(workspace.id);
    } finally {
      if (token === scanToken) loading = false;
    }
  }

  function sameImages(a: ImageEntry[], b: ImageEntry[]) {
    if (a.length !== b.length) return false;
    return a.every((x, i) => x.path === b[i].path && x.modified === b[i].modified && x.size === b[i].size);
  }

  async function addWorkspaceFromDialog() {
    const path = await open({ directory: true, title: "Choose a folder for the new workspace" });
    if (typeof path === "string") await addWorkspaceFromPath(path);
  }

  /** Adds a folder as a workspace and opens it (or opens the existing workspace for it). */
  async function addWorkspaceFromPath(path: string) {
    try {
      const workspace = await workspaces.add(path);
      await openWorkspace(workspace.id);
    } catch (e) {
      error = String(e);
    }
  }

  async function renameWorkspace(id: number, name: string) {
    try {
      await workspaces.rename(id, name);
    } catch (e) {
      error = String(e);
    }
  }

  async function changeFolder(workspace: Workspace) {
    const path = await open({
      directory: true,
      title: `Choose the folder for “${workspace.name}”`,
      defaultPath: workspace.path,
    });
    if (typeof path !== "string") return;
    try {
      await workspaces.setFolder(workspace.id, path);
      scanCache.delete(workspace.id);
      viewState.delete(workspace.id);
      if (workspace.id === activeId) {
        selection.clear();
        focusPath = null;
        // Tags are stored relative to the folder, so reload them for the new location.
        tags.load(workspace.id).catch((e) => (error = String(e)));
        await refresh();
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function removeWorkspace(workspace: Workspace) {
    const confirmed = await ask(
      `Remove “${workspace.name}” from Reference Library?\n\nIts tags are removed too. The folder and its images are not touched.`,
      { title: "Remove workspace", kind: "warning", okLabel: "Remove", cancelLabel: "Cancel" },
    );
    if (!confirmed) return;
    try {
      await workspaces.remove(workspace.id);
      scanCache.delete(workspace.id);
      viewState.delete(workspace.id);
      missing.delete(workspace.id);
      if (workspace.id === activeId) {
        activeId = null;
        images = [];
        scanError = null;
        selection.clear();
        focusPath = null;
        tags.clear();
        const next = workspaces.list[0];
        if (next) await openWorkspace(next.id);
        else settings.activeWorkspaceId = null;
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function toggleRecursive() {
    const workspace = active;
    if (!workspace) return;
    try {
      await workspaces.setRecursive(workspace.id, !workspace.recursive);
      forgetPosition();
      scanCache.delete(workspace.id);
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  // ---- Tags ----

  /** The images the tag shortcut applies to: the one in the viewer, else the selection, else the focused one. */
  function tagTargets(): string[] {
    if (viewing !== null) return filtered[viewing] ? [filtered[viewing].path] : [];
    if (selection.size > 0) return filtered.filter((i) => selection.has(i.path)).map((i) => i.path);
    return focus >= 0 ? [filtered[focus].path] : [];
  }

  function openTagEditor(paths: string[]) {
    if (paths.length === 0) {
      showNotice("Select one or more images to tag first.");
      return;
    }
    const subject = paths.length === 1 ? baseName(paths[0]) : `${paths.length} images`;
    editor = { paths, subject };
  }

  /** For the open editor: how many of its images have each tag. */
  const editorApplied = $derived.by(() => {
    const counts = new Map<number, number>();
    for (const path of editor?.paths ?? []) {
      for (const id of tags.idsOf(path)) counts.set(id, (counts.get(id) ?? 0) + 1);
    }
    return counts;
  });

  function onTagClick(id: number | "untagged", add: boolean) {
    forgetPosition();
    if (id === "untagged") {
      const on = !filterUntagged;
      filterTags.clear();
      filterUntagged = on;
      return;
    }
    filterUntagged = false;
    if (add) {
      if (filterTags.has(id)) filterTags.delete(id);
      else filterTags.add(id);
    } else if (filterTags.size === 1 && filterTags.has(id)) {
      filterTags.clear();
    } else {
      filterTags.clear();
      filterTags.add(id);
    }
  }

  function clearFilter() {
    filterTags.clear();
    filterUntagged = false;
  }

  async function renameTag(id: number, name: string) {
    try {
      await tags.rename(id, name);
    } catch (e) {
      error = String(e);
    }
  }

  async function deleteTag(tag: TagItem) {
    const confirmed = await ask(
      `Delete the tag “${tag.name}”?\n\nIt will be removed from ${tag.count === 1 ? "1 image" : `${tag.count} images`}. The images themselves are not touched.`,
      { title: "Delete tag", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" },
    );
    if (!confirmed) return;
    try {
      await tags.delete(tag.id);
    } catch (e) {
      error = String(e);
    }
  }

  // ---- Menus ----

  function showWorkspaceMenu(workspace: Workspace, x: number, y: number) {
    menu = {
      x,
      y,
      items: [
        { label: "Rename", action: () => (renamingId = workspace.id) },
        { label: "Change folder…", action: () => changeFolder(workspace) },
        { label: `Show folder in ${fileManagerName}`, action: () => revealItemInDir(workspace.path) },
        "separator",
        { label: "Remove workspace…", danger: true, action: () => removeWorkspace(workspace) },
      ],
    };
  }

  function showImageMenu(index: number, x: number, y: number) {
    const image = filtered[index];
    const targets = tagTargets();
    menu = {
      x,
      y,
      items: [
        { label: "Open", hint: "Enter", action: () => (viewing = index) },
        {
          label: targets.length > 1 ? `Tags for ${targets.length} images…` : "Tags…",
          hint: tagKey,
          action: () => openTagEditor(targets),
        },
        "separator",
        { label: `Show in ${fileManagerName}`, action: () => revealItemInDir(image.path) },
      ],
    };
  }

  function showTagMenu(tag: TagItem, x: number, y: number) {
    const targets = selection.size > 0 ? tagTargets() : [];
    menu = {
      x,
      y,
      items: [
        { label: "Show images with this tag", action: () => onTagClick(tag.id, false) },
        {
          label: targets.length > 1 ? `Add to the ${targets.length} selected images` : "Add to the selected image",
          disabled: targets.length === 0,
          action: () => tags.add(targets, tag.name).catch((e) => (error = String(e))),
        },
        "separator",
        { label: "Rename", action: () => (renamingTagId = tag.id) },
        { label: "Delete tag…", danger: true, action: () => deleteTag(tag) },
      ],
    };
  }

  // ---- Sorting and viewing ----

  function setSort(key: SortKey) {
    settings.sortKey = key;
    forgetPosition();
  }

  function toggleSortDirection() {
    settings.sortAscending = !settings.sortAscending;
    forgetPosition();
  }

  function closeViewer() {
    const path = viewing !== null ? filtered[viewing]?.path : undefined;
    if (path) {
      selection.clear();
      selection.add(path);
      focusPath = path;
    }
    viewing = null;
  }

  /** Closes whatever is on top (a menu, the tag editor, Settings, or the viewer), like Esc. */
  function goBack() {
    if (menu) menu = null;
    else if (editor) editor = null;
    else if (settingsOpen) settingsOpen = false;
    else if (viewing !== null) closeViewer();
  }

  // The mouse's back button (button 3) goes back; the forward button (4) does nothing. Both are
  // blocked here so the web view doesn't try to navigate the page itself.
  const BACK_BUTTON = 3;
  const FORWARD_BUTTON = 4;

  function onmousedown(e: MouseEvent) {
    if (e.button === BACK_BUTTON || e.button === FORWARD_BUTTON) e.preventDefault();
  }

  function onmouseup(e: MouseEvent) {
    if (e.button === BACK_BUTTON) {
      e.preventDefault();
      goBack();
    } else if (e.button === FORWARD_BUTTON) {
      e.preventDefault();
    }
  }

  function onkeydown(e: KeyboardEvent) {
    if (editor || settingsOpen || menu) return;
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;

    if (!e.ctrlKey && !e.altKey && !e.metaKey && e.key.toLowerCase() === settings.tagHotkey) {
      if (!active) return;
      e.preventDefault(); // also stops the key's letter from being typed into the editor
      openTagEditor(tagTargets());
      return;
    }

    // The rest are Ctrl shortcuts (Cmd on a Mac).
    if (viewing !== null || !hasPrimaryModifier(e) || hasOtherModifier(e)) return;
    if (e.key >= "1" && e.key <= "9") {
      // Ctrl+1 … Ctrl+9 switch to the workspace at that position.
      const workspace = workspaces.list[Number(e.key) - 1];
      if (workspace) openWorkspace(workspace.id);
      e.preventDefault();
    } else if (e.key === "b" || e.key === "B") {
      settings.sidebarCollapsed = !settings.sidebarCollapsed;
      e.preventDefault();
    }
  }

  // ---- Start-up ----

  async function init() {
    try {
      await workspaces.load();
      // Before workspaces existed, the app opened a single folder. That becomes the first workspace.
      if (workspaces.list.length === 0 && settings.lastFolder) {
        const workspace = await workspaces.add(settings.lastFolder).catch(() => null);
        if (workspace && settings.recursive) await workspaces.setRecursive(workspace.id, true);
      }
      settings.lastFolder = null;

      const start = workspaces.list.find((w) => w.id === settings.activeWorkspaceId) ?? workspaces.list[0];
      if (start) await openWorkspace(start.id);
    } catch (e) {
      error = String(e);
    } finally {
      ready = true;
    }
  }

  onMount(() => {
    init();

    // No browser right-click menu (Back, Reload, …), except in text fields where copy/paste is handy.
    const blockBrowserMenu = (e: MouseEvent) => {
      if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
    };
    document.addEventListener("contextmenu", blockBrowserMenu);

    // Dropping a folder (or an image) onto the window adds it as a workspace.
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const { type } = event.payload;
      if (type === "enter" || type === "over") dragHover = true;
      else if (type === "leave") dragHover = false;
      else if (type === "drop") {
        dragHover = false;
        if (event.payload.paths.length > 0) addWorkspaceFromPath(event.payload.paths[0]);
      }
    });

    return () => {
      document.removeEventListener("contextmenu", blockBrowserMenu);
      unlisten.then((fn) => fn());
    };
  });
</script>

<svelte:window {onkeydown} {onmousedown} {onmouseup} />

<div class="app">
  {#if !settings.sidebarCollapsed}
    <Sidebar
      workspaces={workspaces.list}
      {activeId}
      {missing}
      bind:renamingId
      onselect={openWorkspace}
      onadd={addWorkspaceFromDialog}
      onrename={renameWorkspace}
      onmenu={showWorkspaceMenu}
      {tagItems}
      {untaggedCount}
      {filterTags}
      {filterUntagged}
      bind:tagSearch
      bind:renamingTagId
      ontagclick={onTagClick}
      ontagrename={renameTag}
      ontagmenu={showTagMenu}
    />
  {/if}

  <main>
    <header class="toolbar">
      <button
        class="icon"
        class:on={!settings.sidebarCollapsed}
        onclick={() => (settings.sidebarCollapsed = !settings.sidebarCollapsed)}
        title="Show or hide the sidebar ({shortcutLabel('B')})"
        aria-label="Toggle sidebar"
      >
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <rect x="1.5" y="2.5" width="13" height="11" rx="1.5" fill="none" stroke="currentColor" stroke-width="1.2" />
          <path d="M6 2.5v11" stroke="currentColor" stroke-width="1.2" />
        </svg>
      </button>

      {#if active}
        <div class="title" title={active.path}>
          <span class="name">{active.name}</span>
          <span class="path">{active.path}</span>
        </div>
        <button onclick={refresh} title="Rescan the folder for new or removed images">Refresh</button>
      {/if}

      <label class="check" title="Also show images inside subfolders of this workspace's folder">
        <input type="checkbox" checked={active?.recursive ?? false} disabled={!active} onchange={toggleRecursive} />
        Subfolders
      </label>

      <div class="group">
        <select value={settings.sortKey} onchange={(e) => setSort(e.currentTarget.value as SortKey)} title="Sort by">
          <option value="name">Name</option>
          <option value="modified">Date modified</option>
          <option value="size">File size</option>
        </select>
        <button
          onclick={toggleSortDirection}
          title={settings.sortAscending ? "Ascending" : "Descending"}
          aria-label="Toggle sort direction">{settings.sortAscending ? "↑" : "↓"}</button
        >
      </div>

      <div class="group" role="radiogroup" aria-label="Layout">
        <button
          class="icon"
          class:on={settings.layout === "masonry"}
          onclick={() => (settings.layout = "masonry")}
          title="Masonry: tiles shaped like their images"
          aria-label="Masonry layout"
          role="radio"
          aria-checked={settings.layout === "masonry"}
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <rect x="1.5" y="1.5" width="5.5" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="1.5" y="10.5" width="5.5" height="4" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="9" y="1.5" width="5.5" height="4" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="9" y="7.5" width="5.5" height="7" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
        </button>
        <button
          class="icon"
          class:on={settings.layout === "grid"}
          onclick={() => (settings.layout = "grid")}
          title="Grid: square tiles in rows"
          aria-label="Grid layout"
          role="radio"
          aria-checked={settings.layout === "grid"}
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <rect x="1.5" y="1.5" width="5.5" height="5.5" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="9" y="1.5" width="5.5" height="5.5" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="1.5" y="9" width="5.5" height="5.5" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
            <rect x="9" y="9" width="5.5" height="5.5" rx="1" fill="none" stroke="currentColor" stroke-width="1.2" />
          </svg>
        </button>
      </div>

      {#if settings.layout === "grid"}
        <div class="group">
          <button class:on={settings.tileFit === "cover"} onclick={() => (settings.tileFit = "cover")} title="Crop thumbnails to fill the tiles">Fill</button>
          <button class:on={settings.tileFit === "contain"} onclick={() => (settings.tileFit = "contain")} title="Show whole images">Whole</button>
        </div>
      {/if}

      <input
        class="size"
        type="range"
        min="80"
        max="400"
        step="10"
        bind:value={settings.tileSize}
        title="Thumbnail size"
        aria-label="Thumbnail size"
      />

      <button class="icon" onclick={() => (settingsOpen = true)} title="Settings" aria-label="Settings">
        <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
          <circle cx="8" cy="8" r="2.2" fill="none" stroke="currentColor" stroke-width="1.2" />
          <path
            d="M8 1.5v2M8 12.5v2M1.5 8h2M12.5 8h2M3.4 3.4l1.4 1.4M11.2 11.2l1.4 1.4M3.4 12.6l1.4-1.4M11.2 4.8l1.4-1.4"
            stroke="currentColor"
            stroke-width="1.2"
            stroke-linecap="round"
          />
        </svg>
      </button>
    </header>

    {#if error}
      <div class="notice error">
        {error}
        <button onclick={() => (error = null)}>Dismiss</button>
      </div>
    {/if}

    {#if active && filtering}
      <div class="filter-bar">
        <span>
          Showing {filtered.length.toLocaleString()} of {images.length.toLocaleString()} images
          {#if filterUntagged}without tags{:else}tagged{/if}
        </span>
        {#each [...filterTags] as id (id)}
          <button class="filter-chip" onclick={() => onTagClick(id, true)} title="Remove from the filter">
            #{tags.names.get(id) ?? "?"} <span aria-hidden="true">✕</span>
          </button>
        {/each}
        <button class="link" onclick={clearFilter}>Clear filter</button>
      </div>
    {/if}

    {#if active && filtered.length > 0}
      {#key gridKey}
        <Grid
          bind:this={grid}
          images={filtered}
          mode={settings.layout}
          tileSize={settings.tileSize}
          tileFit={settings.tileFit}
          showTags={settings.showTileTags}
          showNames={settings.showTileNames}
          {selection}
          bind:focus={() => focus, (i) => (focusPath = filtered[i]?.path ?? null)}
          active={viewing === null && menu === null && editor === null && !settingsOpen}
          initialTopIndex={viewState.get(active.id)?.topIndex ?? 0}
          tagsOf={(path) => tags.namesOf(path)}
          onopen={(i) => (viewing = i)}
          onitemmenu={showImageMenu}
        />
      {/key}
    {:else}
      <div class="empty">
        {#if !ready}
          <!-- starting up -->
        {:else if !active}
          <h1>Reference Library</h1>
          <p>Make a workspace for each collection of references, like <em>Art inspiration</em> or <em>Home interiors</em>.</p>
          <p>Each workspace shows the images from its own folder.</p>
          <button class="primary big" onclick={addWorkspaceFromDialog}>Add a workspace…</button>
          <p class="hint">…or drag a folder onto this window.</p>
        {:else if scanError}
          <h2>Can't open “{active.name}”</h2>
          <p>The folder may have been moved or renamed, or it's on a drive that isn't connected.</p>
          <p class="hint path-line">{active.path}</p>
          <div class="actions">
            <button class="primary" onclick={() => changeFolder(active)}>Choose its folder…</button>
            <button onclick={refresh}>Try again</button>
            <button onclick={() => removeWorkspace(active)}>Remove workspace…</button>
          </div>
        {:else if loading && images.length === 0}
          <p>Scanning folder…</p>
        {:else if filtering && images.length > 0}
          <p>{filterUntagged ? "Every image here has tags." : "No images have all of these tags."}</p>
          <button onclick={clearFilter}>Clear filter</button>
        {:else}
          <p>No images found in <strong>{active.name}</strong>.</p>
          {#if !active.recursive}
            <p class="hint">Tip: tick “Subfolders” to include images in folders inside it.</p>
          {/if}
        {/if}
      </div>
    {/if}

    <footer class="status">
      {#if active && !scanError}
        <span>{filtered.length.toLocaleString()} images{loading ? " · scanning…" : ""}</span>
        {#if notice}
          <span class="notice-text">{notice}</span>
        {:else if selection.size > 1}
          <span>{selection.size.toLocaleString()} selected</span>
        {:else if focus >= 0}
          <span class="dim">{filtered[focus].name}</span>
          {#if tags.idsOf(filtered[focus].path).length}
            <span class="status-tags">{tags.namesOf(filtered[focus].path).join(" · ")}</span>
          {/if}
        {/if}
        <span class="dim right">{tagKey} to tag · {primaryKeyLabel}/Shift+click to select several · double-click to view</span>
      {/if}
    </footer>
  </main>
</div>

{#if viewing !== null && filtered[viewing]}
  <Viewer
    images={filtered}
    bind:index={viewing}
    active={editor === null && !settingsOpen}
    tagsOf={(path) => tags.namesOf(path)}
    onclose={closeViewer}
  />
{/if}

{#if editor}
  <TagEditor
    allTags={tags.list}
    {usage}
    targetCount={editor.paths.length}
    applied={editorApplied}
    subject={editor.subject}
    onadd={(name) => tags.add(editor!.paths, name)}
    onremove={(id) => tags.remove(editor!.paths, id)}
    onclose={() => (editor = null)}
  />
{/if}

{#if settingsOpen}
  <SettingsDialog onclose={() => (settingsOpen = false)} />
{/if}

{#if menu}
  <ContextMenu {...menu} onclose={() => (menu = null)} />
{/if}

{#if dragHover}
  <div class="drop">Drop a folder to add it as a workspace</div>
{/if}

<style>
  .app {
    display: flex;
    height: 100vh;
  }
  main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .toolbar {
    display: flex;
    align-items: center;
    /* In a narrow window the controls continue on a second row instead of being cut off. */
    flex-wrap: wrap;
    gap: 6px 10px;
    padding: 8px 12px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
  }
  .toolbar > :global(button),
  .toolbar > .group {
    flex-shrink: 0;
    white-space: nowrap;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 30px;
    height: 28px;
    padding: 0;
    color: var(--text-dim);
  }
  .icon.on {
    border-color: var(--border);
    color: var(--text);
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: 10px;
    /* Takes whatever room is left (shortening the path if needed), so only the buttons
       themselves can push the toolbar onto a second row. */
    flex: 1 1 0;
    min-width: 120px;
    overflow: hidden;
    white-space: nowrap;
  }
  .title .name {
    font-weight: 600;
  }
  .title .path {
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 12px;
    color: var(--text-dim);
  }
  .group {
    display: flex;
    gap: 2px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
    /* Right-aligned (also when no workspace is open and there's no title taking up the space). */
    margin-left: auto;
    white-space: nowrap;
    cursor: pointer;
  }
  .check:has(input:disabled) {
    color: var(--text-dim);
    cursor: default;
  }
  .size {
    width: 120px;
    accent-color: var(--accent);
  }
  .notice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 8px 12px;
  }
  .error {
    background: #3a1d1f;
    color: #ffb4b4;
  }
  .filter-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 6px 12px;
    font-size: 13px;
    color: var(--text-dim);
    background: #1c2433;
    border-bottom: 1px solid var(--border);
  }
  .filter-chip {
    padding: 1px 10px;
    font-size: 12px;
    color: var(--text);
    background: #24406e;
    border: 1px solid #3a63a8;
    border-radius: 999px;
  }
  .filter-chip:hover {
    background: #2c4d84;
  }
  .filter-chip span {
    margin-left: 4px;
    font-size: 10px;
    color: var(--text-dim);
  }
  .link {
    padding: 0;
    color: var(--accent);
    background: none;
    border: none;
  }
  .link:hover {
    text-decoration: underline;
    background: none;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 4px;
    padding: 24px;
    text-align: center;
    color: var(--text-dim);
  }
  .empty p {
    margin: 4px 0;
  }
  .empty h1 {
    margin: 0 0 8px;
    font-size: 24px;
    font-weight: 600;
    color: var(--text);
  }
  .empty h2 {
    margin: 0 0 6px;
    font-size: 18px;
    font-weight: 600;
    color: var(--text);
  }
  .empty em {
    color: var(--text);
    font-style: normal;
  }
  .big {
    margin: 14px 0 4px;
    padding: 8px 20px;
    font-size: 15px;
  }
  .hint {
    font-size: 13px;
  }
  .path-line {
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .actions {
    display: flex;
    gap: 8px;
    margin-top: 12px;
  }
  .status {
    display: flex;
    gap: 16px;
    min-height: 28px;
    padding: 5px 12px;
    font-size: 12px;
    background: var(--panel);
    border-top: 1px solid var(--border);
  }
  .status span {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-tags {
    color: #c8d8ff;
  }
  .notice-text {
    color: #ffc58f;
  }
  .dim {
    color: var(--text-dim);
  }
  .right {
    margin-left: auto;
  }
  .drop {
    position: fixed;
    inset: 12px;
    z-index: 20;
    display: grid;
    place-items: center;
    font-size: 20px;
    border: 2px dashed var(--accent);
    border-radius: 10px;
    background: rgba(22, 22, 24, 0.85);
    pointer-events: none;
  }
</style>
