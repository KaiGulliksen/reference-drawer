# Reference Library

A lightweight desktop app for browsing folders of art reference images.
Built with [Tauri 2](https://tauri.app) (Rust backend + the WebView2 browser engine that ships with Windows)
and [Svelte 5](https://svelte.dev) for the interface.

## Using it

Shortcuts are written for Windows and Linux; on a Mac use **⌘ (Cmd)** wherever it says Ctrl,
and the app says *Finder* where it says *Explorer* here.

- **Workspaces** keep separate collections apart, e.g. *Art inspiration* and *Home interiors*.
  Each one shows the images from its own folder. Add one with **+** in the sidebar (or drag a folder
  onto the window), switch with a click or **Ctrl+1…9**, and hide the sidebar with **Ctrl+B**.
  - Double-click a workspace to rename it; right-click (or **⋯**) to rename, change its folder,
    show it in Explorer or remove it. Removing a workspace never touches the folder or its images.
  - Each workspace remembers its own *Subfolders* setting, and where you were scrolled to.
- **Click** a thumbnail to select it, **double-click** or press **Enter** to view it full size.
  Right-click an image for more options.
- **Select several images** like in Explorer: Ctrl+click to add or remove one, Shift+click (or
  Shift+arrow keys) for a range, **Ctrl+A** for all, **Esc** to clear.
- In the grid: arrow keys move the selection, Page Up/Down and Home/End jump.
- **Tags:** press **T** (or right-click → *Tags…*) to tag the selected images, or the image open in
  the viewer. Type a tag and press Enter; existing tags are suggested as you type (Tab completes).
  With several images selected, tags only some of them have are shown with a count; click one to
  add it to all. Enter on an empty box or Esc closes the editor. The shortcut can be changed in
  **Settings** (the gear button).
  - The sidebar lists the tags used in the workspace. Click one to show only those images,
    Ctrl+click to combine tags (images must have all of them), or pick *Untagged*.
    The search box narrows the list; Enter shows the best match.
  - Double-click a tag to rename it (renaming onto an existing name merges the two), or
    right-click it to delete it. Tags belong to their workspace.
- In the viewer:
  - scroll to zoom (towards the cursor) · drag to pan · double-click to toggle fit / actual size
  - ← / → (or A / D) previous / next · F or 0 fit · 1 actual size · + / − zoom
  - **Esc** or the **mouse back button** closes it (the back button also closes menus, the tag
    editor and Settings, one at a time)
- **Subfolders** includes images in folders inside the workspace's folder.
- **Mood boards** gather the references for one piece in a single place (in the sidebar, under the
  workspaces; each board belongs to its workspace).
  - Add images: right-click → *Add to board…*, drag selected tiles onto a board in the sidebar, or
    *Add to board* in the full-size viewer. Boards point at your images; nothing is copied.
  - On the board: drag empty space (or with the middle button) to move around, scroll to zoom,
    drag images to move them, drag the corner handles to resize, Shift+drag to select with a box.
    **Arrange** tidies the selection (or everything) into rows; **Fit** (or F) shows everything.
  - Double-click an image to view it full size, empty space to add a note, a note to edit it.
  - Right-click an image to **crop** it (only what's shown on the board changes, never the file),
    bring it to the front, or remove it from the board (Delete). Removing never touches the file.
  - *← Images*, clicking the workspace, or the mouse back button returns to the grid.
- **Layout** (toolbar): *Masonry* shows tiles shaped like their images, in columns; *Grid* shows
  square tiles in rows, where **Fill / Whole** switches between cropped and whole images.
- **Settings** (the gear button) has switches for showing tags and file names on thumbnails.
  Tags are shown and names hidden by default; the focused image's name is always in the status bar.

Workspaces and tags are stored in an SQLite database, `library.db`, in the app's data folder:
`%APPDATA%\com.kaigulliksen.reflibrary` on Windows, `~/Library/Application Support/com.kaigulliksen.reflibrary`
on macOS, `~/.local/share/com.kaigulliksen.reflibrary` on Linux. **Tags stay with their images:**
- Rename an image, or move it to another place inside the workspace's folder (in Explorer, Finder
  or any other app), and its tags follow it the next time the folder is scanned (opening the
  workspace, Refresh, or starting the app). The status bar says how many were kept.
- Editing an image in place keeps its tags too.
- A *copy* of a tagged image starts untagged, and a deleted image's tags are kept in case the file comes back.
- Moving the whole folder works too (use *Change folder…*).

How it works: each tagged image gets a "fingerprint" of its contents (its size plus its first and last
16 KB). When a tagged file disappears and an untagged file with the same fingerprint turns up,
it's the same image under a new name. Only files of exactly the same size are ever compared, so this
stays fast. A file that's both renamed *and* edited before the next scan can't be recognised.

Supported formats: JPG, PNG, WebP, GIF, BMP, AVIF.

## Platforms

Windows is the main platform, but the app is written to work on **macOS** and **Linux** too:
shortcuts use Cmd on a Mac, the library database is the same on every system, and file names
follow each system's rules about capital letters.

To build for all three, open the repository on GitHub → **Actions** → **Build for all platforms** →
**Run workflow**. When it finishes, the installers (Windows `.exe`, macOS `.dmg`, Linux `.deb` /
`.rpm` / `.AppImage`) can be downloaded from the run's page. It uses GitHub Actions minutes, and only
runs when started by hand. The macOS build isn't signed, so the first time you open it, right-click
the app → Open.

## Running and building

You need [Node.js](https://nodejs.org) and [Rust](https://rustup.rs), plus the
[system requirements for Tauri](https://tauri.app/start/prerequisites/): the Microsoft C++ Build Tools
on Windows, Xcode Command Line Tools on macOS, or WebKitGTK development packages on Linux.

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
  lib/Sidebar.svelte      the workspace list and the tag list with search
  lib/TagEditor.svelte    adding/removing tags on the selected images
  lib/SettingsDialog.svelte  settings (the tag shortcut)
  lib/Board.svelte        a mood board: the free canvas, notes and cropping
  lib/board.ts            board geometry: placing new images, Arrange, Fit
  lib/boards.svelte.ts    the open workspace's boards, kept in sync with the database
  lib/ContextMenu.svelte  right-click menus (items defined in lib/menu.ts)
  lib/tags.svelte.ts      the open workspace's tags, kept in sync with the database
  lib/platform.ts         differences between Windows, macOS and Linux (shortcuts, names, paths)
  lib/Grid.svelte         virtualized thumbnail grid (only on-screen tiles exist)
  lib/layout.ts           where each tile goes, for the grid and masonry layouts
  lib/Viewer.svelte       full-size viewer with zoom/pan
  lib/api.ts              calls into the Rust side, builds image URLs
  lib/workspaces.svelte.ts  the workspace list, kept in sync with the database
  lib/settings.svelte.ts  remembered preferences
src-tauri/                the backend (Rust)
  src/lib.rs              app setup, commands, the thumb:// protocol
  src/boards.rs           mood boards and what's on them, with tests
  src/library.rs          the library database (workspaces and tags), with its tests;
                          `reconcile` keeps tags with renamed and moved images
  src/identity.rs         fingerprints of image contents (to recognise renamed files)
  src/parallel.rs         spreads file work over several threads
  src/scan.rs             finds image files in a folder
  src/sizes.rs            remembers each image's pixel size (for masonry) in a cache
  src/platform.rs         differences between Windows, macOS and Linux
  src/thumbs.rs           makes thumbnails and viewer previews, keeps the on-disk cache
  src/pool.rs             background workers (newest request first)
  tauri.conf.json         window and app settings
  capabilities/           what the interface is allowed to ask the system for
```

**Why it stays light**

- **Thumbnails, not originals, in the grid.** Each image is shrunk to 400 px once and cached in
  the app's cache folder (`%LOCALAPPDATA%\com.kaigulliksen.reflibrary\thumbs` on Windows; about 15 KB each). JPEGs are decoded directly
  at reduced size, which is much faster than decoding the full image.
- **Virtualized grid.** Only visible tiles exist, so big folders cost about the same as small ones.
- **Images go straight from disk to the webview** via custom URLs (`thumb://` for thumbnails,
  the asset protocol for originals) instead of being copied through JavaScript.
- **Screen-sized previews in the viewer.** A fitted 6000×4000 photo is shown from a preview the size
  of your screen (~10 MB of memory instead of ~100 MB). The original only loads when you zoom in
  past the preview's detail, and is released again when you zoom back out.
- **Thumbnails pause while you scroll fast**, so tiles that only flash past aren't generated.

- **Image sizes are read from file headers once**, then cached (`sizes.db` in the cache folder),
  so the masonry layout costs almost nothing after the first scan of a folder.

Deleting the thumbs folder or `sizes.db` is safe; they're recreated as needed.
