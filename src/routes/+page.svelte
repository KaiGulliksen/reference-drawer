<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import ContextMenu from "$lib/ContextMenu.svelte";
  import Grid from "$lib/Grid.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import Viewer from "$lib/Viewer.svelte";
  import { scanFolder, type ImageEntry, type Workspace } from "$lib/api";
  import type { MenuRequest } from "$lib/menu";
  import { settings, type SortKey } from "$lib/settings.svelte";
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
  let selectedPath = $state<string | null>(null);
  let viewing = $state<number | null>(null);
  let dragHover = $state(false);
  let menu = $state<MenuRequest | null>(null);
  let renamingId = $state<number | null>(null);
  const missing = new SvelteSet<number>();
  let grid = $state<Grid | undefined>();

  // Remembered per workspace while the app is open, so switching back is instant
  // and returns you to the same place.
  const scanCache = new Map<number, ImageEntry[]>();
  const viewState = new Map<number, { topIndex: number; selectedPath: string | null }>();
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

  // The selection is kept by file path, so it survives re-sorting and refreshing.
  const selected = $derived(selectedPath ? sorted.findIndex((i) => i.path === selectedPath) : -1);

  // A fresh grid (starting from the remembered position) for each workspace and sort order.
  const gridKey = $derived(`${activeId}|${settings.sortKey}|${settings.sortAscending}|${active?.recursive}`);

  $effect(() => {
    getCurrentWindow().setTitle(active ? `${active.name} – Reference Library` : "Reference Library");
  });

  // ---- Workspaces ----

  async function openWorkspace(id: number) {
    if (id === activeId) return;
    rememberSelection();
    activeId = id;
    settings.activeWorkspaceId = id;
    viewing = null;
    scanError = null;
    images = scanCache.get(id) ?? [];
    selectedPath = viewState.get(id)?.selectedPath ?? null;
    await refresh();
  }

  /** Remembers where you were in the current workspace, for when you come back to it. */
  function rememberSelection() {
    if (activeId === null) return;
    viewState.set(activeId, { topIndex: grid?.topIndex() ?? 0, selectedPath });
  }

  function forgetPosition(id: number | null) {
    if (id !== null) viewState.set(id, { topIndex: 0, selectedPath });
  }

  /** Rescans the active workspace's folder. */
  async function refresh() {
    const workspace = active;
    if (!workspace) return;
    const token = ++scanToken; // ignore the result if another scan started meanwhile
    loading = true;
    try {
      const result = await scanFolder(workspace.path, workspace.recursive);
      if (token !== scanToken) return;
      scanError = null;
      missing.delete(workspace.id);
      if (!sameImages(images, result.images)) images = result.images;
      scanCache.set(workspace.id, result.images);
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
        selectedPath = null;
        await refresh();
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function removeWorkspace(workspace: Workspace) {
    const confirmed = await ask(
      `Remove “${workspace.name}” from Reference Library?\n\nOnly the workspace is removed. The folder and its images are not touched.`,
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
        selectedPath = null;
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
      forgetPosition(workspace.id);
      scanCache.delete(workspace.id);
      await refresh();
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
        { label: "Show folder in Explorer", action: () => revealItemInDir(workspace.path) },
        "separator",
        { label: "Remove workspace…", danger: true, action: () => removeWorkspace(workspace) },
      ],
    };
  }

  function showImageMenu(index: number, x: number, y: number) {
    const image = sorted[index];
    menu = {
      x,
      y,
      items: [
        { label: "Open", action: () => (viewing = index) },
        { label: "Show in Explorer", action: () => revealItemInDir(image.path) },
      ],
    };
  }

  // ---- Sorting and viewing ----

  function setSort(key: SortKey) {
    settings.sortKey = key;
    forgetPosition(activeId);
  }

  function toggleSortDirection() {
    settings.sortAscending = !settings.sortAscending;
    forgetPosition(activeId);
  }

  function closeViewer() {
    if (viewing !== null) selectedPath = sorted[viewing]?.path ?? selectedPath;
    viewing = null;
  }

  function onkeydown(e: KeyboardEvent) {
    if (viewing !== null || menu || e.target instanceof HTMLInputElement) return;
    if (!e.ctrlKey || e.altKey || e.metaKey) return;
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

<svelte:window {onkeydown} />

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
    />
  {/if}

  <main>
    <header class="toolbar">
      <button
        class="icon"
        class:on={!settings.sidebarCollapsed}
        onclick={() => (settings.sidebarCollapsed = !settings.sidebarCollapsed)}
        title="Show or hide the sidebar (Ctrl+B)"
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

      <div class="spacer"></div>

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

      <div class="group">
        <button class:on={settings.tileFit === "cover"} onclick={() => (settings.tileFit = "cover")} title="Crop thumbnails to fill the tiles">Fill</button>
        <button class:on={settings.tileFit === "contain"} onclick={() => (settings.tileFit = "contain")} title="Show whole images">Whole</button>
      </div>

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
    </header>

    {#if error}
      <div class="notice error">
        {error}
        <button onclick={() => (error = null)}>Dismiss</button>
      </div>
    {/if}

    {#if active && sorted.length > 0}
      {#key gridKey}
        <Grid
          bind:this={grid}
          images={sorted}
          tileSize={settings.tileSize}
          tileFit={settings.tileFit}
          bind:selected={() => selected, (i) => (selectedPath = sorted[i]?.path ?? null)}
          active={viewing === null && menu === null}
          initialTopIndex={viewState.get(active.id)?.topIndex ?? 0}
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
        {:else if loading}
          <p>Scanning folder…</p>
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
        <span>{sorted.length.toLocaleString()} images{loading ? " · scanning…" : ""}</span>
        {#if selected >= 0}
          <span class="dim">{sorted[selected].name}</span>
        {/if}
        <span class="dim right">Double-click or press Enter to view · right-click for options</span>
      {/if}
    </footer>
  </main>
</div>

{#if viewing !== null && sorted[viewing]}
  <Viewer images={sorted} bind:index={viewing} onclose={closeViewer} />
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
    gap: 10px;
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
    min-width: 0;
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
  .spacer {
    flex: 1;
  }
  .group {
    display: flex;
    gap: 2px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
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
