<script lang="ts">
  import { onMount } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import Grid from "$lib/Grid.svelte";
  import Viewer from "$lib/Viewer.svelte";
  import { folderName, scanFolder, type ImageEntry } from "$lib/api";
  import { settings, type SortKey } from "$lib/settings.svelte";

  let folder = $state<string | null>(null);
  // `$state.raw` because the list can be huge and is only ever replaced, never edited in place.
  let images = $state.raw<ImageEntry[]>([]);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let selected = $state(-1);
  let viewing = $state<number | null>(null);
  let dragHover = $state(false);

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

  async function load(path: string) {
    loading = true;
    error = null;
    try {
      const result = await scanFolder(path, settings.recursive);
      folder = result.folder;
      images = result.images;
      settings.lastFolder = result.folder;
      selected = -1;
      viewing = null;
      getCurrentWindow().setTitle(`${folderName(result.folder)} – Reference Library`);
    } catch (e) {
      error = String(e);
    } finally {
      loading = false;
    }
  }

  async function chooseFolder() {
    const path = await open({ directory: true, title: "Choose a folder of images", defaultPath: folder ?? undefined });
    if (typeof path === "string") load(path);
  }

  function toggleRecursive() {
    settings.recursive = !settings.recursive;
    if (folder) load(folder);
  }

  function setSort(key: SortKey) {
    settings.sortKey = key;
    selected = -1;
  }

  function closeViewer() {
    if (viewing !== null) selected = viewing;
    viewing = null;
  }

  onMount(() => {
    if (settings.lastFolder) load(settings.lastFolder);

    // Dropping a folder (or an image) onto the window opens that folder.
    const unlisten = getCurrentWebview().onDragDropEvent((event) => {
      const { type } = event.payload;
      if (type === "enter" || type === "over") dragHover = true;
      else if (type === "leave") dragHover = false;
      else if (type === "drop") {
        dragHover = false;
        if (event.payload.paths.length > 0) load(event.payload.paths[0]);
      }
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  });
</script>

<main>
  <header class="toolbar">
    <button class="primary" onclick={chooseFolder}>Open folder…</button>
    {#if folder}
      <span class="path" title={folder}>{folder}</span>
      <button onclick={() => folder && load(folder)} title="Rescan the folder for new or removed images">Refresh</button>
    {/if}

    <div class="spacer"></div>

    <label class="check" title="Also show images inside subfolders">
      <input type="checkbox" checked={settings.recursive} onchange={toggleRecursive} />
      Subfolders
    </label>

    <div class="group">
      <select value={settings.sortKey} onchange={(e) => setSort(e.currentTarget.value as SortKey)} title="Sort by">
        <option value="name">Name</option>
        <option value="modified">Date modified</option>
        <option value="size">File size</option>
      </select>
      <button
        onclick={() => (settings.sortAscending = !settings.sortAscending)}
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

  {#if folder && sorted.length > 0}
    <Grid
      images={sorted}
      tileSize={settings.tileSize}
      tileFit={settings.tileFit}
      bind:selected
      active={viewing === null}
      onopen={(i) => (viewing = i)}
    />
  {:else}
    <div class="empty">
      {#if loading}
        <p>Scanning folder…</p>
      {:else if folder}
        <p>No images found in <strong>{folderName(folder)}</strong>.</p>
        {#if !settings.recursive}
          <p class="hint">Tip: tick “Subfolders” to include images in folders inside it.</p>
        {/if}
      {:else}
        <h1>Reference Library</h1>
        <p>Open a folder to browse the images inside it.</p>
        <button class="primary big" onclick={chooseFolder}>Open folder…</button>
        <p class="hint">…or drag a folder onto this window.</p>
      {/if}
    </div>
  {/if}

  <footer class="status">
    {#if folder}
      <span>{sorted.length.toLocaleString()} images{loading ? " · scanning…" : ""}</span>
      {#if selected >= 0 && sorted[selected]}
        <span class="dim">{sorted[selected].name}</span>
      {/if}
      <span class="dim right">Double-click or press Enter to view · arrow keys to move</span>
    {/if}
  </footer>
</main>

{#if viewing !== null && sorted[viewing]}
  <Viewer images={sorted} bind:index={viewing} onclose={closeViewer} />
{/if}

{#if dragHover}
  <div class="drop">Drop a folder to open it</div>
{/if}

<style>
  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
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
  .path {
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    text-overflow: ellipsis;
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
    color: var(--text-dim);
  }
  .empty h1 {
    margin: 0 0 4px;
    font-size: 24px;
    font-weight: 600;
    color: var(--text);
  }
  .big {
    margin: 12px 0 4px;
    padding: 8px 20px;
    font-size: 15px;
  }
  .hint {
    font-size: 13px;
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
