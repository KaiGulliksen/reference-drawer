<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";
  import { ask, open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import BoardCanvas from "$lib/Board.svelte";
  import ContextMenu from "$lib/ContextMenu.svelte";
  import Grid from "$lib/Grid.svelte";
  import SettingsDialog from "$lib/SettingsDialog.svelte";
  import Sidebar from "$lib/Sidebar.svelte";
  import TagEditor from "$lib/TagEditor.svelte";
  import Viewer from "$lib/Viewer.svelte";
  import {
    addBoardImages,
    boardsLeftByMove,
    getBoard,
    moveImages,
    onFolderChanged,
    scanWorkspace,
    type Board,
    type ImageEntry,
    type Imported,
    type Workspace,
  } from "$lib/api";
  import { placeAt, placeNewImages } from "$lib/board";
  import { canTake, onlyFiles, readDrop, saveDropped, type Dropped } from "$lib/drop";
  import { boards } from "$lib/boards.svelte";
  import { colorHex, colorTint } from "$lib/colors";
  import type { MenuItem, MenuRequest } from "$lib/menu";
  import {
    baseName,
    canDropFolders,
    droppedPaths,
    fileManagerName,
    hasOtherModifier,
    hasPrimaryModifier,
    isBrowserShortcut,
    isRefreshShortcut,
    pathKey,
    primaryKeyLabel,
    shortcutLabel,
  } from "$lib/platform";
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
  /** When the viewer was opened from a board: the board's images (otherwise it shows the grid's). */
  let viewerList = $state.raw<ImageEntry[] | null>(null);

  // The mood board being shown instead of the image grid, if any.
  let activeBoardId = $state<number | null>(null);
  const activeBoard = $derived(boards.list.find((b) => b.id === activeBoardId) ?? null);
  let boardCanvas = $state<BoardCanvas | undefined>();
  let boardZoom = $state(1);
  let boardCanUndo = $state(false);
  let boardCanRedo = $state(false);
  let boardCounts = $state({ images: 0, notes: 0 });
  let renamingBoardId = $state<number | null>(null);
  /** The board in the sidebar that images are being dragged over. */
  let dropBoardId = $state<number | null>(null);
  /** The other workspace in the sidebar that images from the grid are being dragged over (to move them there). */
  let moveTargetId = $state<number | null>(null);

  // Tag filter: show only images with all of these tags, or only untagged images.
  const filterTags = new SvelteSet<number>();
  let filterUntagged = $state(false);
  let tagSearch = $state("");

  /**
   * While something is dragged onto the window from outside the app: where it would go.
   * "main" is the open workspace (or the open board). `onlyFiles`: it could be a folder.
   */
  let dropTarget = $state<
    { kind: "main" | "workspace" | "board"; id: number | null; onlyFiles: boolean } | null
  >(null);
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
    tags.list.filter((t) => usage.has(t.id)).map((t) => ({ ...t, count: usage.get(t.id) ?? 0 })),
  );
  const untaggedCount = $derived(images.filter((i) => tags.idsOf(i.path).length === 0).length);

  // The focus is kept by file path, so it survives re-sorting, filtering and refreshing.
  const focus = $derived(focusPath ? filtered.findIndex((i) => i.path === focusPath) : -1);

  // A fresh grid (starting from the remembered position) for each workspace, sort order and filter.
  const gridKey = $derived(
    `${activeId}|${settings.sortKey}|${settings.sortAscending}|${active?.recursive}|${filterUntagged}|${[...filterTags].sort()}`,
  );

  const tagKey = $derived(keyLabel(settings.tagHotkey));

  /** The workspace's images by `pathKey`, to find an image's details from its path. */
  const imagesByPath = $derived(new Map(images.map((image) => [pathKey(image.path), image])));

  /** What the viewer pages through: a board's images, or the grid's. */
  const viewerImages = $derived(viewerList ?? filtered);

  $effect(() => {
    const parts = [activeBoard?.name, active?.name, "Reference Library"].filter(Boolean);
    getCurrentWindow().setTitle(parts.join(" – "));
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
    const count = viewerImages.length;
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
    if (id === activeId) {
      closeBoard(); // clicking the open workspace goes back to its images
      return;
    }
    rememberSelection();
    activeId = id;
    settings.activeWorkspaceId = id;
    activeBoardId = null;
    viewing = null;
    viewerList = null;
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
    boards.load(id).catch((e) => (error = String(e)));
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
      if (!sameImages(images, result.images)) {
        // The viewer goes by position in the grid's list; keep it on the same image when images
        // are added or removed around it (e.g. the folder changed while you're looking).
        const viewed = viewing !== null && viewerList === null ? filtered[viewing]?.path : undefined;
        images = result.images;
        const index = viewed ? filtered.findIndex((i) => i.path === viewed) : -1;
        if (index >= 0) viewing = index;
      }
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
      `Remove “${workspace.name}” from Reference Library?\n\nIts tags and boards are removed too. The folder and its images are not touched.`,
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
        boards.clear();
        activeBoardId = null;
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

  /**
   * The images the tag shortcut applies to: the one in the viewer, else the images selected on
   * the open board, else the grid's selection, else the grid's focused image.
   */
  function tagTargets(): string[] {
    if (viewing !== null) return viewerImages[viewing] ? [viewerImages[viewing].path] : [];
    if (activeBoardId !== null) return boardCanvas?.selectedPaths() ?? [];
    if (selection.size > 0) return filtered.filter((i) => selection.has(i.path)).map((i) => i.path);
    return focus >= 0 ? [filtered[focus].path] : [];
  }

  // ---- Boards ----

  function openBoard(id: number) {
    activeBoardId = id;
    viewing = null;
    viewerList = null;
    boardCounts = { images: 0, notes: 0 };
  }

  function closeBoard() {
    activeBoardId = null;
  }

  async function newBoard() {
    try {
      const board = await boards.create();
      openBoard(board.id);
      renamingBoardId = board.id; // straight into naming it
    } catch (e) {
      error = String(e);
    }
  }

  async function renameBoard(id: number, name: string) {
    try {
      await boards.rename(id, name);
    } catch (e) {
      error = String(e);
    }
  }

  async function deleteBoard(board: Board) {
    const confirmed = await ask(
      `Delete the board “${board.name}”?\n\nThe images on it stay in your library; only the board goes.`,
      { title: "Delete board", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" },
    );
    if (!confirmed) return;
    try {
      await boards.remove(board.id);
      if (board.id === activeBoardId) closeBoard();
    } catch (e) {
      error = String(e);
    }
  }

  /**
   * Puts images on a board, in rows below what's already there, or around `at` (a board position).
   * Images already on it are skipped. Resolves to how many were added.
   */
  async function addImagesToBoard(boardId: number, paths: string[], at: { x: number; y: number } | null = null) {
    const board = boards.list.find((b) => b.id === boardId);
    if (!board || paths.length === 0) return 0;
    try {
      const data = await getBoard(boardId);
      const onBoard = new Set(data.items.filter((i) => i.path !== null).map((i) => pathKey(i.path!)));
      const fresh = paths
        .map((path) => imagesByPath.get(pathKey(path)))
        .filter((image): image is ImageEntry => !!image && !onBoard.has(pathKey(image.path)));
      if (fresh.length === 0) {
        showNotice(paths.length === 1 ? `That image is already on “${board.name}”.` : `Those images are already on “${board.name}”.`);
        return 0;
      }
      const ratios = fresh.map((image) => (image.width && image.height ? image.width / image.height : 1));
      const spots = at ? placeAt(at, ratios) : placeNewImages(data.items, ratios);
      const added = await addBoardImages(
        boardId,
        fresh.map((image, i) => ({ path: image.path, ...spots[i] })),
      );
      boards.setCount(boardId, data.board.imageCount + added.length);
      if (boardId === activeBoardId) await boardCanvas?.reload();
      showNotice(
        added.length === 1 ? `Added 1 image to “${board.name}”.` : `Added ${added.length} images to “${board.name}”.`,
        4000,
      );
      return added.length;
    } catch (e) {
      error = String(e);
      return 0;
    }
  }

  /** The list of boards to add images to (plus "New board"), as a menu at `x`, `y`. */
  function showBoardPicker(paths: string[], x: number, y: number) {
    const items: MenuItem[] = boards.list.map((board) => ({
      label: board.name,
      hint: String(board.imageCount),
      color: board.color,
      action: () => addImagesToBoard(board.id, paths),
    }));
    if (items.length > 0) items.push("separator");
    items.push({
      label: "New board",
      action: async () => {
        try {
          const board = await boards.create();
          await addImagesToBoard(board.id, paths);
          renamingBoardId = board.id;
        } catch (e) {
          error = String(e);
        }
      },
    });
    menu = { x, y, items };
  }

  function showBoardMenu(board: Board, x: number, y: number) {
    menu = {
      x,
      y,
      items: [
        { label: "Open", action: () => openBoard(board.id) },
        { label: "Rename", action: () => (renamingBoardId = board.id) },
        { current: board.color, pick: (color) => boards.setColor(board.id, color).catch((e) => (error = String(e))) },
        "separator",
        { label: "Delete board…", danger: true, action: () => deleteBoard(board) },
      ],
    };
  }

  /** The board (in the sidebar) under the pointer while dragging images, if any. */
  function boardUnder(x: number, y: number): number | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-board-drop]");
    return el ? Number(el.dataset.boardDrop) : null;
  }

  function openViewerOnBoard(boardImages: ImageEntry[], index: number) {
    viewerList = boardImages;
    viewing = index;
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

  // ---- Moving images to another workspace ----
  // The files themselves move into the other workspace's folder; their tags go with them.

  async function moveToWorkspace(paths: string[], to: Workspace) {
    const from = active;
    if (!from || paths.length === 0 || to.id === from.id) return;
    const them = paths.length === 1 ? "it" : "them";
    try {
      // Boards only hold images from their own workspace, so ask before taking images off any.
      const onBoards = await boardsLeftByMove(from.id, to.id, paths);
      if (onBoards.length > 0) {
        const names = onBoards.map((name) => `“${name}”`).join(", ");
        const confirmed = await ask(
          `${paths.length === 1 ? "This image is" : "Some of these images are"} on ${onBoards.length === 1 ? "the board" : "the boards"} ${names} in “${from.name}”. Boards only hold images from their own workspace, so moving ${them} to “${to.name}” takes ${them} off.\n\nThe ${paths.length === 1 ? "image and its tags move" : "images and their tags move"} either way; nothing is deleted.`,
          { title: "Move to another workspace", kind: "warning", okLabel: "Move", cancelLabel: "Cancel" },
        );
        if (!confirmed) return;
      }

      showNotice(paths.length === 1 ? "Moving the image…" : `Moving ${paths.length} images…`, 120_000);
      const result = await moveImages(from.id, to.id, paths);
      scanCache.delete(to.id); // rescanned when it's opened next
      if (from.id === activeId) {
        await refresh();
        // Tags and board counts changed with the images that left.
        tags.load(from.id).catch((e) => (error = String(e)));
        boards.refresh().catch((e) => (error = String(e)));
      }

      const parts: string[] = [];
      if (result.moved.length > 0) parts.push(`Moved ${plural(result.moved.length, "image")} to “${to.name}”`);
      if (result.already > 0) {
        parts.push(`${plural(result.already, "image")} ${result.already === 1 ? "was" : "were"} already in its folder`);
      }
      if (parts.length > 0) showNotice(`${parts.join(" · ")}.`, 6000);
      else notice = null;
      if (result.failed.length > 0) {
        error =
          result.failed.length === 1
            ? `Couldn't move ${result.failed[0]}`
            : `${result.failed.length} images couldn't be moved. ${result.failed[0]}`;
      }
    } catch (e) {
      notice = null;
      error = String(e);
    }
  }

  /** The other workspaces to move images to, as a menu at `x`, `y`. */
  function showWorkspacePicker(paths: string[], x: number, y: number) {
    const others = workspaces.list.filter((w) => w.id !== activeId);
    const items: MenuItem[] = others.map((workspace) => ({
      label: workspace.name,
      color: workspace.color,
      disabled: missing.has(workspace.id),
      action: () => moveToWorkspace(paths, workspace),
    }));
    if (items.length === 0) items.push({ label: "No other workspaces yet (add one with +)", disabled: true, action: () => {} });
    menu = { x, y, items };
  }

  /** The workspace (in the sidebar) under the pointer while dragging images, other than the open one. */
  function otherWorkspaceUnder(x: number, y: number): number | null {
    const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-workspace-drop]");
    const id = el ? Number(el.dataset.workspaceDrop) : null;
    return id === activeId ? null : id;
  }

  // ---- Images and folders dropped onto the window from outside the app ----
  // Images are saved as new files in a workspace's folder: the open one, or the one in the
  // sidebar they're dropped on. Dropped onto a board, they're put on it too. Folders become
  // workspaces (Windows only, see platform.ts).

  /** Where a drop at this point on screen would go. */
  function dropTargetAt(x: number, y: number, files: boolean): NonNullable<typeof dropTarget> {
    const el = document.elementFromPoint(x, y);
    const workspace = el?.closest<HTMLElement>("[data-workspace-drop]");
    if (workspace) return { kind: "workspace", id: Number(workspace.dataset.workspaceDrop), onlyFiles: files };
    const board = el?.closest<HTMLElement>("[data-board-drop]");
    if (board) return { kind: "board", id: Number(board.dataset.boardDrop), onlyFiles: files };
    return { kind: "main", id: null, onlyFiles: files };
  }

  /** What the drop overlay says. */
  const dropLabel = $derived.by(() => {
    if (!dropTarget) return "";
    const folders = dropTarget.onlyFiles && canDropFolders ? " · a folder becomes a workspace" : "";
    if (dropTarget.kind === "workspace") {
      const workspace = workspaces.list.find((w) => w.id === dropTarget!.id);
      return `Save into “${workspace?.name}”${folders}`;
    }
    if (dropTarget.kind === "board") {
      return `Add to “${boards.list.find((b) => b.id === dropTarget!.id)?.name}” (saved in “${active?.name}”)`;
    }
    if (!active) return canDropFolders ? "Drop a folder to add it as a workspace" : "Add a workspace first (+ in the sidebar)";
    if (activeBoard) return `Add to this board (saved in “${active.name}”)`;
    return `Save into “${active.name}”${folders}`;
  });

  async function receiveDrop(dropped: Dropped, target: NonNullable<typeof dropTarget>, x: number, y: number) {
    if (dropped.folders.length > 0) {
      await addDroppedFolders(dropped.folders);
      return; // images dropped together with folders are left alone: which workspace would they go to?
    }
    if (dropped.images.length === 0) {
      showNotice("There's no image in what was dropped.", 4000);
      return;
    }

    const workspace = target.kind === "workspace" ? workspaces.list.find((w) => w.id === target.id) : active;
    if (!workspace) {
      showNotice(
        canDropFolders
          ? "Add a workspace first (drop a folder here, or use +): dropped images are saved in its folder."
          : "Add a workspace first with + in the sidebar: dropped images are saved in its folder.",
        6000,
      );
      return;
    }
    // The board to put them on, and where (decided now: the view may move while they're saved).
    const boardId = target.kind === "board" ? target.id : target.kind === "main" ? activeBoardId : null;
    const at = boardId !== null && target.kind === "main" ? (boardCanvas?.boardPointAt(x, y) ?? null) : null;

    const count = dropped.images.length;
    showNotice(count === 1 ? "Saving the image…" : `Saving ${count} images…`, 120_000);
    const saved: Imported[] = [];
    const failures: string[] = [];
    // One at a time, in order: a big drop shouldn't start dozens of downloads at once.
    for (const image of dropped.images) {
      try {
        saved.push(await saveDropped(workspace.id, image));
      } catch (e) {
        failures.push(String(e));
      }
    }
    const paths = saved.map((s) => s.path);
    const fresh = saved.filter((s) => !s.already).map((s) => s.path);

    const parts: string[] = [];
    if (fresh.length > 0) parts.push(`Saved ${plural(fresh.length, "image")} in “${workspace.name}”`);
    if (fresh.length < saved.length) {
      const already = saved.length - fresh.length;
      parts.push(fresh.length > 0 ? `${already} ${already === 1 ? "was" : "were"} already there` : `${already === 1 ? "It's" : "They're"} already in “${workspace.name}”`);
    }

    if (workspace.id === activeId) {
      await refresh();
      // New images get the tags being filtered on, so they don't vanish from view straight away.
      if (fresh.length > 0 && filterTags.size > 0) {
        const names = [...filterTags].map((id) => tags.names.get(id)).filter((name): name is string => !!name);
        for (const name of names) await tags.add(fresh, name).catch((e) => (error = String(e)));
        parts.push(`tagged ${names.map((name) => `#${name}`).join(" ")} to match the filter`);
      }
      if (boardId !== null) {
        const added = await addImagesToBoard(boardId, paths, at);
        const board = boards.list.find((b) => b.id === boardId);
        if (added > 0 && board) parts.push(`added to “${board.name}”`);
      } else if (paths.length > 0) {
        // Select them, so it's easy to see where they went (and to tag them right away).
        selection.clear();
        for (const path of paths) if (filtered.some((i) => i.path === path)) selection.add(path);
        focusPath = paths.find((path) => selection.has(path)) ?? focusPath;
      }
    } else {
      scanCache.delete(workspace.id); // rescanned when it's opened next
    }

    if (parts.length > 0) showNotice(`${parts.join(" · ")}.`, 6000);
    else notice = null;
    if (failures.length > 0) {
      error = failures.length === 1 ? failures[0] : `${failures.length} of the dropped images couldn't be saved. ${failures[0]}`;
    }
  }

  /** Adds dropped folders as workspaces and opens the first, where the system lets the app see where they are. */
  async function addDroppedFolders(folders: File[]) {
    if (!canDropFolders) {
      showNotice("To add a folder as a workspace, use + in the sidebar.", 5000);
      return;
    }
    const paths = await droppedPaths(folders);
    if (paths.length === 0) {
      error = "Couldn't tell where the dropped folder is. Add it with + in the sidebar instead.";
      return;
    }
    try {
      const added = [];
      for (const path of paths) added.push(await workspaces.add(path));
      await openWorkspace(added[0].id);
    } catch (e) {
      error = String(e);
    }
  }

  const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

  // ---- Menus ----

  function showWorkspaceMenu(workspace: Workspace, x: number, y: number) {
    menu = {
      x,
      y,
      items: [
        { label: "Rename", action: () => (renamingId = workspace.id) },
        { label: "Change folder…", action: () => changeFolder(workspace) },
        { label: `Show folder in ${fileManagerName}`, action: () => revealItemInDir(workspace.path) },
        { current: workspace.color, pick: (color) => workspaces.setColor(workspace.id, color).catch((e) => (error = String(e))) },
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
        {
          label: targets.length > 1 ? `Add ${targets.length} to board…` : "Add to board…",
          action: () => showBoardPicker(targets, x, y),
        },
        {
          label: targets.length > 1 ? `Move ${targets.length} to workspace…` : "Move to workspace…",
          action: () => showWorkspacePicker(targets, x, y),
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
        { current: tag.color, pick: (color) => tags.setColor(tag.id, color).catch((e) => (error = String(e))) },
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
    // Back in the grid, the image you ended on is selected (not when the viewer came from a board).
    const path = viewing !== null && viewerList === null ? filtered[viewing]?.path : undefined;
    if (path) {
      selection.clear();
      selection.add(path);
      focusPath = path;
    }
    viewing = null;
    viewerList = null;
  }

  /** Closes whatever is on top: a menu, the tag editor, Settings, the viewer, then a board. */
  function goBack() {
    if (menu) menu = null;
    else if (editor) editor = null;
    else if (settingsOpen) settingsOpen = false;
    else if (viewing !== null) closeViewer();
    else if (activeBoardId !== null) closeBoard();
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

  // Clicking the size slider, Subfolders or the sort list leaves the keyboard on that control, so
  // the arrow keys would go on changing it, and the tag shortcut wouldn't work, until you clicked
  // an image. So they let go of the keyboard after being used with the mouse. Someone who reached
  // them with Tab is using the keyboard on purpose, so they keep it.
  let usingPointer = true;

  function letGoOfKeyboard(e: Event) {
    if (usingPointer) (e.currentTarget as HTMLElement).blur();
  }

  // Seen first, before anything else gets the key: even with a dialog open, the web view
  // shouldn't reload the page or open its find bar (see isBrowserShortcut).
  function onkeydowncapture(e: KeyboardEvent) {
    if (e.key === "Tab") usingPointer = false;
    if (!isBrowserShortcut(e)) return;
    e.preventDefault();
    if (isRefreshShortcut(e) && !e.repeat && viewing === null && !editor && !settingsOpen && !menu) refresh();
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

    // The open workspace's folder is watched (watch.rs): rescan when images there are added,
    // removed or renamed, so they show up without pressing Refresh.
    const stopWatching = onFolderChanged((workspaceId) => {
      if (workspaceId === activeId) refresh();
      else scanCache.delete(workspaceId);
    });

    // No browser right-click menu (Back, Reload, …), except in text fields where copy/paste is handy.
    const blockBrowserMenu = (e: MouseEvent) => {
      if (!(e.target instanceof HTMLInputElement)) e.preventDefault();
    };
    document.addEventListener("contextmenu", blockBrowserMenu);

    // Things dragged onto the window from outside the app (see "Images and folders dropped").
    // Drags that start inside the app (selected text, say) aren't imports.
    let dragFromApp = false;
    let dragWatchdog: ReturnType<typeof setTimeout> | undefined;
    const isTextField = (target: EventTarget | null) =>
      target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement;
    const ondragstart = () => (dragFromApp = true);
    const ondragend = () => (dragFromApp = false);
    const ondragover = (e: DragEvent) => {
      const types = e.dataTransfer?.types ?? [];
      // Text dragged into a text field is typed there, as usual.
      if (dragFromApp || !canTake(types) || (isTextField(e.target) && !types.includes("Files"))) return;
      e.preventDefault(); // accept the drop (and stop the web view from opening the file itself)
      e.dataTransfer!.dropEffect = "copy";
      dropTarget = dropTargetAt(e.clientX, e.clientY, onlyFiles(types));
      // The web view keeps sending "dragover" while something is held over the window; if it
      // stops without a "dragleave" (that can happen), stop showing the overlay.
      clearTimeout(dragWatchdog);
      dragWatchdog = setTimeout(() => (dropTarget = null), 1000);
    };
    const ondragleave = (e: DragEvent) => {
      if (e.relatedTarget === null) dropTarget = null; // left the window (not just moved to another element)
    };
    const ondrop = (e: DragEvent) => {
      const types = e.dataTransfer?.types ?? [];
      if (dragFromApp || (isTextField(e.target) && !types.includes("Files"))) return;
      e.preventDefault(); // never let the web view open the dropped file or link itself
      clearTimeout(dragWatchdog);
      const target = dropTargetAt(e.clientX, e.clientY, onlyFiles(types));
      dropTarget = null;
      if (e.dataTransfer) receiveDrop(readDrop(e.dataTransfer), target, e.clientX, e.clientY);
    };
    window.addEventListener("dragstart", ondragstart);
    window.addEventListener("dragend", ondragend);
    window.addEventListener("dragenter", ondragover);
    window.addEventListener("dragover", ondragover);
    window.addEventListener("dragleave", ondragleave);
    window.addEventListener("drop", ondrop);

    return () => {
      stopWatching.then((stop) => stop());
      document.removeEventListener("contextmenu", blockBrowserMenu);
      window.removeEventListener("dragstart", ondragstart);
      window.removeEventListener("dragend", ondragend);
      window.removeEventListener("dragenter", ondragover);
      window.removeEventListener("dragover", ondragover);
      window.removeEventListener("dragleave", ondragleave);
      window.removeEventListener("drop", ondrop);
      clearTimeout(dragWatchdog);
    };
  });
</script>

<svelte:window {onkeydown} {onkeydowncapture} {onmousedown} {onmouseup} onpointerdowncapture={() => (usingPointer = true)} />

<div class="app">
  <!-- The toolbar spans the whole window; the sidebar and the images sit side by side below it. -->
  <header class="toolbar">
    <!-- Above the sidebar (and as wide as it, so the rest of the toolbar starts over the images). -->
    <div class="corner" class:wide={!settings.sidebarCollapsed}>
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
    </div>

    {#if active && activeBoard}
      <button onclick={closeBoard} title="Back to the workspace's images (mouse back button)">← Images</button>
      <div class="title" title="A board in {active.name}">
        <span class="name">{activeBoard.name}</span>
        <span class="path">board in {active.name}</span>
      </div>
      <div class="group board-actions">
        <button disabled={!boardCanUndo} onclick={() => boardCanvas?.undo()} title="Undo ({shortcutLabel('Z')})">Undo</button>
        <button disabled={!boardCanRedo} onclick={() => boardCanvas?.redo()} title="Redo ({shortcutLabel('Y')})">Redo</button>
      </div>
      <div class="group">
        <button onclick={() => boardCanvas?.addNote()} title="Add a text note (or double-click empty space)">Add note</button>
        <button onclick={() => boardCanvas?.arrangeItems()} title="Tidy the selected items (or everything) into neat rows">Arrange</button>
        <button onclick={() => boardCanvas?.fit()} title="Zoom to show everything (F)">Fit</button>
      </div>
      <span class="zoom">{Math.round(boardZoom * 100)}%</span>
    {:else}
    {#if active}
      <div class="title" title={active.path}>
        <span class="name">{active.name}</span>
        <span class="path">{active.path}</span>
      </div>
      <button onclick={refresh} title="Rescan the folder for new or removed images (F5)">Refresh</button>
    {/if}

    <label class="check" title="Also show images inside subfolders of this workspace's folder">
      <input
        type="checkbox"
        checked={active?.recursive ?? false}
        disabled={!active}
        onchange={(e) => {
          letGoOfKeyboard(e);
          toggleRecursive();
        }}
      />
      Subfolders
    </label>

    <div class="group">
      <select
        value={settings.sortKey}
        onchange={(e) => {
          setSort(e.currentTarget.value as SortKey);
          letGoOfKeyboard(e);
        }}
        oncancel={letGoOfKeyboard}
        title="Sort by"
      >
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
      onpointerup={letGoOfKeyboard}
      title="Thumbnail size"
      aria-label="Thumbnail size"
    />
    {/if}
  </header>

  <div class="body">
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
        boards={boards.list}
        {activeBoardId}
        bind:renamingBoardId
        dropBoardId={dropBoardId ?? (dropTarget?.kind === "board" ? dropTarget.id : null)}
        dropWorkspaceId={moveTargetId ?? (dropTarget?.kind === "workspace" ? dropTarget.id : null)}
        onboardselect={openBoard}
        onboardadd={newBoard}
        onboardrename={renameBoard}
        onboardmenu={showBoardMenu}
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
      {#if error}
        <div class="notice error">
          {error}
          <button onclick={() => (error = null)}>Dismiss</button>
        </div>
      {/if}

      {#if active && filtering && !activeBoard}
        <div class="filter-bar">
          <span>
            Showing {filtered.length.toLocaleString()} of {images.length.toLocaleString()} images
            {#if filterUntagged}without tags{:else}tagged{/if}
          </span>
          {#each [...filterTags] as id (id)}
            {@const tag = tags.byId.get(id)}
            <button
              class="filter-chip"
              style:--tag={colorHex(tag?.color)}
              style:--tag-bg={colorTint(tag?.color, 0.3)}
              onclick={() => onTagClick(id, true)}
              title="Remove from the filter"
            >
              #{tag?.name ?? "?"} <span aria-hidden="true">✕</span>
            </button>
          {/each}
          <button class="link" onclick={clearFilter}>Clear filter</button>
        </div>
      {/if}

      {#if active && activeBoard}
        {#key activeBoard.id}
          <BoardCanvas
            bind:this={boardCanvas}
            bind:zoom={boardZoom}
            bind:canUndo={boardCanUndo}
            bind:canRedo={boardCanRedo}
            boardId={activeBoard.id}
            {images}
            active={viewing === null && menu === null && editor === null && !settingsOpen}
            onopen={openViewerOnBoard}
            onmenu={(request) => (menu = request)}
            oncount={(imageCount, notes) => {
              boardCounts = { images: imageCount, notes };
              if (activeBoardId !== null) boards.setCount(activeBoardId, imageCount);
            }}
            onerror={(message) => (error = message)}
          />
        {/key}
      {:else if active && filtered.length > 0}
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
            tagsOf={(path) => tags.of(path)}
            onopen={(i) => (viewing = i)}
            onitemmenu={showImageMenu}
            ondragmove={(x, y) => {
              dropBoardId = boardUnder(x, y);
              moveTargetId = otherWorkspaceUnder(x, y);
            }}
            ondragend={(x, y, paths) => {
              const board = boardUnder(x, y);
              const workspace = workspaces.list.find((w) => w.id === otherWorkspaceUnder(x, y));
              dropBoardId = null;
              moveTargetId = null;
              if (paths && board !== null) addImagesToBoard(board, paths);
              else if (paths && workspace) moveToWorkspace(paths, workspace);
            }}
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
            {#if canDropFolders}<p class="hint">…or drag a folder onto this window.</p>{/if}
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
        {#if active && activeBoard}
          <span>
            {boardCounts.images === 1 ? "1 image" : `${boardCounts.images} images`}{boardCounts.notes
              ? ` · ${boardCounts.notes === 1 ? "1 note" : `${boardCounts.notes} notes`}`
              : ""}
          </span>
          {#if notice}<span class="notice-text">{notice}</span>{/if}
          <span class="dim right">Drag empty space to move · scroll to zoom · Shift+drag to select · double-click for a note</span>
        {:else if active && !scanError}
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

      {#if dropTarget}
        <!-- Over the images (or board) it would go to; for a workspace or board in the sidebar,
             that item lights up instead and only the label shows. -->
        <div class="drop" class:whole={dropTarget.kind === "main"}><span>{dropLabel}</span></div>
      {/if}
    </main>
  </div>
</div>

{#if viewing !== null && viewerImages[viewing]}
  <Viewer
    images={viewerImages}
    bind:index={viewing}
    active={editor === null && !settingsOpen && menu === null}
    tagsOf={(path) => tags.of(path)}
    onaddtoboard={(path, x, y) => showBoardPicker([path], x, y)}
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


<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100vh;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  main {
    position: relative;
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
  .corner {
    display: flex;
    flex-shrink: 0;
    gap: 2px;
  }
  /* With the sidebar showing, the corner plus the toolbar's gap ends 12px past the sidebar's edge,
     so the workspace name lines up with the images below (which are 12px in from it as well). */
  .corner.wide {
    width: calc(var(--sidebar-width) - 10px);
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
  .board-actions {
    margin-left: auto;
  }
  .board-actions button:disabled {
    color: var(--text-dim);
    background: transparent;
    cursor: default;
    opacity: 0.6;
  }
  .zoom {
    min-width: 44px;
    text-align: right;
    font-size: 13px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
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
    /* A tag's own colour, if it has one. */
    background: var(--tag-bg, #24406e);
    border: 1px solid var(--tag, #3a63a8);
    border-radius: 999px;
  }
  .filter-chip:hover {
    background: var(--tag-bg, #2c4d84);
    filter: brightness(1.15);
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
  /* pointer-events: none, so the page can still see what's under the pointer (see dropTargetAt). */
  .drop {
    position: absolute;
    inset: 10px;
    z-index: 20;
    display: grid;
    place-items: center;
    border-radius: 10px;
    pointer-events: none;
  }
  .drop.whole {
    border: 2px dashed var(--accent);
    background: rgba(22, 22, 24, 0.7);
  }
  .drop span {
    max-width: 90%;
    padding: 10px 18px;
    font-size: 17px;
    text-align: center;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 8px;
  }
</style>
