# Reference Library

A lightweight desktop app for browsing folders of art reference images.
Built with [Tauri 2](https://tauri.app) (Rust backend + the WebView2 browser engine that ships with Windows)
and [Svelte 5](https://svelte.dev) for the interface.

## Using it

- **Workspaces** keep separate collections apart, e.g. *Art inspiration* and *Home interiors*.
  Each one shows the images from its own folder. Add one with **+** in the sidebar (or drag a folder
  onto the window), switch with a click or **Ctrl+1…9**, and hide the sidebar with **Ctrl+B**.
  - Double-click a workspace to rename it; right-click (or **⋯**) to rename, change its folder,
    show it in Explorer or remove it. Removing a workspace never touches the folder or its images.
  - Each workspace remembers its own *Subfolders* setting, and where you were scrolled to.
- **Click** a thumbnail to select it, **double-click** or press **Enter** to view it full size.
  Right-click an image for more options.
- In the grid: arrow keys move the selection, Page Up/Down and Home/End jump.
- In the viewer:
  - scroll to zoom (towards the cursor) · drag to pan · double-click to toggle fit / actual size
  - ← / → (or A / D) previous / next · F or 0 fit · 1 actual size · + / − zoom · Esc close
- **Subfolders** includes images in folders inside the workspace's folder.
- **Fill / Whole** switches between cropped square thumbnails and whole images.

Workspaces are stored in `%APPDATA%\com.kaigulliksen.reflibrary\library.db` (an SQLite database).

Supported formats: JPG, PNG, WebP, GIF, BMP, AVIF.

## Running and building

You need [Node.js](https://nodejs.org), [Rust](https://rustup.rs) and the Microsoft C++ Build Tools.

```bash
npm install          # first time only
npm run tauri dev    # run the app with live reloading while you edit
npm run tauri build  # make the optimised app + installer
```

`tauri build` produces:
- `src-tauri/target/release/reference-library.exe`, the app itself, which runs on its own
- `src-tauri/target/release/bundle/nsis/…-setup.exe`, an installer that adds a Start-menu entry

## How it's put together

```
src/                      the interface (Svelte)
  routes/+page.svelte     main window: switching workspaces, toolbar, empty states, menus
  lib/Sidebar.svelte      the workspace list (room for a tags section later)
  lib/ContextMenu.svelte  right-click menus (items defined in lib/menu.ts)
  lib/Grid.svelte         virtualized thumbnail grid (only on-screen tiles exist)
  lib/Viewer.svelte       full-size viewer with zoom/pan
  lib/api.ts              calls into the Rust side, builds image URLs
  lib/workspaces.svelte.ts  the workspace list, kept in sync with the database
  lib/settings.svelte.ts  remembered preferences
src-tauri/                the backend (Rust)
  src/lib.rs              app setup, commands, the thumb:// protocol
  src/library.rs          the library database (workspaces; tags later)
  src/scan.rs             finds image files in a folder
  src/thumbs.rs           makes thumbnails and viewer previews, keeps the on-disk cache
  src/pool.rs             background workers (newest request first)
  tauri.conf.json         window and app settings
  capabilities/           what the interface is allowed to ask the system for
```

**Why it stays light**

- **Thumbnails, not originals, in the grid.** Each image is shrunk to 400 px once and cached in
  `%LOCALAPPDATA%\com.kaigulliksen.reflibrary\thumbs` (about 15 KB each). JPEGs are decoded directly
  at reduced size, which is much faster than decoding the full image.
- **Virtualized grid.** Only visible tiles exist, so big folders cost about the same as small ones.
- **Images go straight from disk to the webview** via custom URLs (`thumb://` for thumbnails,
  the asset protocol for originals) instead of being copied through JavaScript.
- **Screen-sized previews in the viewer.** A fitted 6000×4000 photo is shown from a preview the size
  of your screen (~10 MB of memory instead of ~100 MB). The original only loads when you zoom in
  past the preview's detail, and is released again when you zoom back out.
- **Thumbnails pause while you scroll fast**, so tiles that only flash past aren't generated.

Deleting the thumbs folder is safe; thumbnails are recreated as needed.
