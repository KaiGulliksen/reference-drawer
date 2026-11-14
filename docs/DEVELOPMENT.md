# Building and working on Reference Drawer

Built with [Tauri 2](https://tauri.app) (a Rust backend, and the system's own web view for the
window) and [Svelte 5](https://svelte.dev) for the interface.

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

## Building for every platform

Windows is the main platform, but the app is written to work on **macOS** and **Linux** too:
shortcuts use Cmd on a Mac, the library database is the same on every system, and file names
follow each system's rules about capital letters.

To build for all three, open the repository on GitHub → **Actions** → **Build for all platforms** →
**Run workflow**. When it finishes, the installers (Windows `.exe`, macOS `.dmg`, Linux `.deb` /
`.rpm` / `.AppImage`) can be downloaded from the run's page. It uses GitHub Actions minutes, and only
runs when started by hand. The macOS build isn't signed, so the first time you open it, right-click
the app → Open.

## How it's put together

```
src/                      the interface (Svelte)
  routes/+page.svelte     main window: switching workspaces, toolbar, empty states, menus
  lib/Sidebar.svelte      the workspace list and the tag list with search
  lib/TagEditor.svelte    adding/removing tags on the selected images
  lib/SettingsDialog.svelte  settings (thumbnails, the tag shortcut, the backups folder) and the About page
  lib/Board.svelte        a mood board: the free canvas, notes and cropping
  lib/board.ts            board geometry: placing new images, Arrange, Fit
  lib/drop.ts             reading images and folders dragged onto the window from other apps
  lib/boards.svelte.ts    the open workspace's boards, kept in sync with the database
  lib/ContextMenu.svelte  right-click menus (items defined in lib/menu.ts), including the color row
  lib/colors.ts           the colors workspaces, boards and tags can have, and how each looks
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
  src/backup.rs           safety copies of the library database (daily, and before upgrades)
  src/boards.rs           mood boards and what's on them, with tests
  src/library.rs          the library database (workspaces, tags, their colors), with its tests;
                          `reconcile` keeps tags with renamed and moved images
  src/identity.rs         fingerprints of image contents (to recognise renamed files)
  src/import.rs           saves images dragged in from other apps (and downloads web addresses)
  src/moving.rs           moves image files to another workspace's folder (never replacing anything)
  src/watch.rs            watches the open workspace's folder, so changes show up by themselves
  src/parallel.rs         spreads file work over several threads
  src/scan.rs             finds image files in a folder
  src/sizes.rs            remembers each image's pixel size (for masonry) in a cache
  src/platform.rs         differences between Windows, macOS and Linux (e.g. where dropped folders are)
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
- **Boards load only as much detail as they show.** An image on a board steps from the thumbnail
  to a 1024 px preview, then the screen-sized one, then the original, as you zoom in on it.
- **Thumbnails pause while you scroll fast**, so tiles that only flash past aren't generated.
- **Memory is handed back while the window is minimised** (Windows): the web view is asked to let
  go of what it isn't using (`save_memory_while_minimized` in `platform.rs`), which takes the app
  from around 190 MB to around 40 MB after a minute, and it takes it back when the window returns.
- **Huge images are decoded one at a time** (`BIG_DECODE_BYTES` in `thumbs.rs`), so a handful of
  very large PNGs can't each take a hundred megabytes at the same moment.

- **The folder is watched, not polled.** The system reports changes to the open workspace's folder,
  so watching costs nothing while it stays the same, and a burst of changes means one rescan.

- **Image sizes are read from file headers once**, then cached (`sizes.db` in the cache folder),
  so the masonry layout costs almost nothing after the first scan of a folder.

Deleting the thumbs folder or `sizes.db` is safe; they're recreated as needed.
