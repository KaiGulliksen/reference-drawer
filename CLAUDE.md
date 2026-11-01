# Reference Library

A lightweight desktop app for collecting, browsing, tagging and arranging reference images
(art inspiration, interiors, …). Tauri 2 (Rust backend in `src-tauri/`) + Svelte 5 / SvelteKit as a
single-page app (interface in `src/`). README.md has the features, how to use them, and the full
file layout.

The owner has basic coding skills but a good grasp of concepts: explain changes and decisions in
plain language, say what was tested and how, and keep code commented the way it is now (comments
explain *why*, in plain English, at the level of the surrounding code).

## Goals that shape every change

- **Lightweight.** Low memory and CPU use matter more than features. Measure (Task Manager's
  "private working set", see Testing) when a change could affect it.
- **Cross-platform.** The app ships on Windows today but must stay buildable and correct on macOS
  and Linux. Every change has to work on all three (rules below).
- **The user's files are sacred.** The app never renames, edits, replaces or deletes image files
  or folders. Removing a workspace, tag, board or board item only touches the library database.
  It changes folders in exactly two ways, both asked for by the user:
  - images dragged onto the window are saved as new files (`import.rs`): never replacing one
    (`new_file`, " (2)" when the name is taken), and not at all if the same image is already in
    the workspace;
  - *Move to workspace* moves image files into another workspace's folder (`moving.rs`): the name
    is claimed with `new_file` first, so nothing is ever replaced; across drives it copies, checks
    the copy, and only then removes the original. Only images inside the source workspace's folder
    are ever moved.

## How it fits together

- **Workspaces** are folders, browsed separately. `scan_workspace` (Rust) walks the folder, fills in
  each image's pixel size from the size cache (`sizes.rs`), runs `Library::reconcile` (tags and board
  items follow renamed files), and allows the folder in the asset-protocol scope so the web view
  may load its files. Nothing outside a workspace folder is ever served. It also starts watching
  that folder (`watch.rs`, the `notify` crate; one folder at a time, the last one scanned): after
  a quiet moment the backend emits `folder-changed`, and the page rescans if it's the open
  workspace. So whatever the page shows is rescanned on its own; don't add polling.
- **Moving images between workspaces:** `move_images` (lib.rs) moves the files (`moving.rs`), then
  `Library::moved_to_workspace` brings the tags along by name (creating missing tags in the target)
  and drops the old records, which takes the images off the old workspace's boards; boards only
  hold images from their own workspace. `boards_left_by_move` lets the page ask first. If the
  target folder is inside the source folder, the image stays in the source workspace too.
- **Getting images on screen**, from cheapest to most expensive:
  - grid thumbnails: `thumb://…?s=400` (`thumbs.rs`), made once and cached on disk, generated on a
    newest-first worker pool (`pool.rs`) so on-screen tiles go first;
  - viewer/board previews: `thumb://…?s=1920|2560|3840` (screen-sized; small images and GIFs are
    served as the original);
  - originals: Tauri's asset protocol (`convertFileSrc`), only when zoomed in past the preview's detail.
  Build URLs with the helpers in `src/lib/api.ts`, never by hand.
- **Data lives in three places:**
  - the library database (`library.db`, app data folder): workspaces, tags, boards; real data, with
    migrations and tests. `backup.rs` keeps copies in `backups/` next to it (`VACUUM INTO`): one a
    day for 7 days, and one before each upgrade, kept for good;
  - caches (app cache folder): thumbnails and `sizes.db`; safe to delete, rebuilt on demand;
  - `localStorage` (`settings.svelte.ts`): per-device UI preferences only (layout, tile size, tag
    shortcut, last workspace). Never put library data there.
- **Drops from other apps** (browser images, files, folders) are handled by the web view itself:
  Tauri's own drop handling is off (`dragDropEnabled: false` in `tauri.conf.json`) because it only
  understands files on disk and ignores images dragged out of a browser. `drop.ts` reads the drop
  (the picture itself when the source hands it over, else its web address, which `import.rs`
  downloads with `ureq`); `+page.svelte` decides where it goes (open workspace, a workspace or
  board in the sidebar, or the open board at the drop point). Web pages aren't told where dropped
  folders are, so folder → workspace only works on Windows, via WebView2's
  `postMessageWithAdditionalObjects` (`platform.ts` / `platform.rs`).
- **Interface state:** `src/routes/+page.svelte` owns the page state (active workspace or board,
  selection, filter, which overlay is open) and wires the components together. Shared lists live in
  small stores (`workspaces/tags/boards.svelte.ts`) that call the backend and update locally.
- **Layering:** the grid or a board is underneath; the viewer, tag editor, Settings and menus open on
  top. Each underlying component gets an `active` prop that is false while something is on top, so
  keys go to the top layer only. `goBack()` (Esc-like, also the mouse back button) closes the top
  layer first: menu → tag editor → Settings → viewer → board.

## Keeping it lightweight (don't undo these)

- The grid is virtualized: only on-screen tiles exist (`Grid.svelte` + `layout.ts`), and tiles that
  only flash past during fast scrolling never request a thumbnail.
- Big lists are `$state.raw` and replaced, not edited in place. Derive per-image lookups with maps
  keyed by `pathKey`.
- Never put originals in the grid. In the viewer and on boards, pick the smallest source that's sharp
  enough and drop the bigger one again when it's no longer needed.
- Reading many files (headers, fingerprints) goes through `parallel::map`; don't hold the database
  lock while reading files (see `reconcile`: read, compute without the lock, then write).
- Prefer a few kilobytes of extra code over a new dependency; check bundle and memory impact before
  adding one.

## Cross-platform rules

- **Platform differences live in one place:** `src/lib/platform.ts` on the interface side, and
  `src-tauri/src/platform.rs` (or clearly named `cfg!(target_os = …)` constants) in Rust. Don't
  scatter OS checks around.
- **Shortcuts:** use `hasPrimaryModifier(e)` / `hasOtherModifier(e)` (Cmd on macOS, Ctrl elsewhere),
  never `e.ctrlKey` directly. On macOS Ctrl+click means right-click. Show shortcuts with
  `shortcutLabel()` / `primaryKeyLabel`, never a hard-coded "Ctrl".
- **Wording:** no Windows-only names in the interface; use `fileManagerName` ("Explorer" /
  "Finder" / "file manager").
- **Paths:** never build or split paths by hand with `\` or `/`. In Rust use `Path`/`PathBuf`.
  Paths stored in the database are relative to the workspace folder and always use `/`
  (`relative_path` / `full_path` in `library.rs`). Paths sent to the interface are written the way
  the system writes them; `baseName()` in `platform.ts` handles either slash.
- **Capital letters in file names:** the same file on Windows and macOS, different files on Linux.
  Compare paths with `same_path()` in SQL, `path_key()` in Rust and `pathKey()` in the interface;
  never lower-case paths directly. (Tag names are case-insensitive everywhere; that's a design
  choice, not a platform one.)
- **Locations:** use Tauri's path API (`app_data_dir`, `app_cache_dir`), never hard-coded folders
  like `%APPDATA%`.
- **Web view:** Windows uses WebView2 (Chromium); macOS and Linux use WebKit (Safari's engine).
  Only use web features Safari supports too. Minimum macOS version is 12 (`tauri.conf.json`).
- **Installers:** settings for every platform go in `tauri.conf.json`; Windows-only ones in
  `tauri.windows.conf.json` (add `tauri.macos.conf.json` / `tauri.linux.conf.json` if needed).
- **Tests:** Rust tests must pass on every platform; write paths with `full_path(folder, "a/b.jpg")`
  in tests rather than Windows-style strings.
- **Checking other platforms:** `.github/workflows/build.yml` builds and tests on Windows, macOS
  and Linux. It only runs when started by hand and uses the owner's GitHub Actions minutes
  (macOS minutes count 10×), so ask before starting it. It has never been run yet.

## Database

`src-tauri/src/library.rs` (workspaces, tags, the shared helpers) and `boards.rs` (mood boards).
Schema changes are numbered entries in `MIGRATIONS` (6 so far), only ever added at the end; never
edit an existing one, since installed copies have already run it. Foreign keys are switched off while
migrating (rebuilding a table would otherwise delete everything linked to it) and each step is
checked with `pragma_foreign_key_check` before it is saved. Add a test for every migration that
starts from the previous version's data. A library with steps this version doesn't know (made by a
newer version) is refused and left alone. If the app can't start (`start_up` in `lib.rs`), it says
why in a message box and quits.

Images get a record in the `images` table when they're tagged or put on a board (`image_record`);
tags and board items point at that record, so both follow renamed files. `reconcile` only forgets
records that are neither tagged nor on a board.

Images are identified by location *and* content: `reconcile` keeps records on files still at their
location (even if edited; the fingerprint is refreshed) and moves the record of a missing file to an
untracked file with the same fingerprint (`identity.rs`: size + first/last 16 KB). Never treat a file
as missing just because it wasn't in the scan (e.g. "Subfolders" is off): check that it's really gone.
Any change to this logic needs tests in `library.rs` (they use real temp files).

## Backend conventions (Rust / Tauri)

- Commands are `async fn` returning `Result<_, String>` with user-readable error text, registered in
  `generate_handler!` in `lib.rs`. Blocking file work goes in `tauri::async_runtime::spawn_blocking`.
- Send file contents to a command as a raw body (`invoke(cmd, bytes, { headers })`, read with
  `tauri::ipc::Request`), not as a JSON array of numbers; see `import_image`.
- Anything the interface calls that isn't a plain command needs a permission in
  `src-tauri/capabilities/default.json` (currently: window title, open/ask dialogs, reveal in folder).
  Grant the narrowest permission that works.
- A broken image file must never crash the app: catch panics around decoding (`catch_unwind`) and
  fall back gracefully, as `thumb_protocol` and `sizes.rs` do.
- `cargo build` should produce no warnings.

## Interface conventions (Svelte 5)

- Runes only (`$state`, `$derived`, `$effect`, `$props`, `{@attach}`); use `untrack` inside effects
  that write state, and function bindings (`bind:x={() => get, (v) => set}`) where a child edits
  state the page keeps by path.
- `npm run check` must report 0 errors and 0 warnings.
- Colours and sizes come from the CSS variables in `src/app.css` (dark theme).
- Right-click menus: build `MenuItem[]` (`src/lib/menu.ts`) and set the page's `menu`; one shared
  `ContextMenu` renders it. A `{ current, pick }` item is a row of colour swatches.
- Colours of workspaces, boards and tags: the database stores only a name from `COLORS`
  (`library.rs`, checked by `check_color`); `src/lib/colors.ts` lists the same names and decides
  how each looks (`colorHex` / `colorTint`). Add a colour to both lists. Menus open only on left/right clicks; backdrops ignore mouse buttons 3/4
  so the back button is handled once, by the page.
- Destructive actions confirm first with the dialog plugin's `ask()`, and the text says what is and
  isn't affected ("The images themselves are not touched").
- Keyboard shortcuts: check `RESERVED_KEYS` in `settings.svelte.ts` before adding a key, and add new
  single-key shortcuts there, so the user can't pick it as the tag shortcut. The web view's own
  shortcuts (reload, find, print, …) are blocked by `isBrowserShortcut` in `platform.ts`; to use one
  of those keys, handle it in the page's `onkeydowncapture`.
- Toolbar controls that keep the keyboard (slider, checkbox, list) call `letGoOfKeyboard` after being
  used with the mouse, so arrow keys and the tag shortcut go back to the grid.
- Board changes are undoable: call `remember()` (or `rememberIfChanged(before)`) in `Board.svelte`
  before changing items. The history lasts until the board is closed.
- A board's images come from `board_files` (looked up directly), not the workspace's scan, which
  doesn't list images in subfolders with "Subfolders" off.
- Dragging inside the app (tiles onto a board) uses plain pointer events, not HTML5 drag-and-drop:
  HTML5 drag events on the window mean "something dropped in from another app" (see Drops above),
  and the page ignores HTML5 drags that start inside it. Find the drop target with
  `document.elementFromPoint` (`data-workspace-drop` / `data-board-drop` mark sidebar targets).
- With pointer capture, `click`/`dblclick` targets become the capturing element; find what's under
  the pointer with `elementFromPoint` instead (see `itemAt` in `Board.svelte`).

## Commands

```bash
npm run check                                            # type-check the interface
cargo test --manifest-path src-tauri/Cargo.toml --lib    # database, size cache and board tests
npm run tauri dev                                        # run with live reload
npm run tauri build                                      # release build + installer (NSIS on Windows)
```

Rust is installed through rustup in `%USERPROFILE%\.cargo\bin`; add it to `PATH` in new shells.
The project path contains a space ("Reference Library"): quote paths in shell commands.

## Testing

Every feature is checked at three levels before it's called done:

1. **Rust tests** for anything touching the database or files (`cargo test --lib`).
2. **The real app in dev mode, isolated from the owner's data** (below), driven with real mouse and
   keyboard input over the Chrome DevTools Protocol, with screenshots looked at, not just DOM checks.
3. **A smoke test of the release build** (also isolated), including a memory reading.

### Never touch the owner's library

The owner uses the installed app, so their real library, caches and settings must never be used by
tests.

- `REFERENCE_LIBRARY_DATA_DIR=<scratch folder>` points the app at a throwaway database; the thumbnail
  and image-size caches then go in `<scratch folder>/cache` too.
- Also set `WEBVIEW2_USER_DATA_FOLDER=<scratch folder>/webview`, for dev and release builds: the
  installed app keeps its settings in the same web view storage (the one-time "last folder → first
  workspace" conversion would otherwise be used up), and while the installed app is running, a
  test build with different browser arguments fails to start at all (WebView2 error 0x8007139F).
- Generated test images and test databases belong in the scratchpad, never in the project.

### Driving the app

- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` lets a Node script drive the
  window over CDP (Node 24 has a global `WebSocket`): `Input.dispatchMouseEvent` /
  `Input.dispatchKeyEvent` for real input, `Page.captureScreenshot`, `Runtime.evaluate`.
- CDP input coordinates are CSS pixels; screenshots are device pixels (×1.25 on this machine).
  Measure positions with `getBoundingClientRect()`, not from screenshots.
- A synthetic Enter must include `text: "\r"`, or focused buttons won't activate.
- The app's native confirm dialogs can be answered with PowerShell `WScript.Shell`
  (`AppActivate("<dialog title>")` + `SendKeys`). A real mouse back button can be sent with
  user32 `mouse_event` (XBUTTON1).
- `Emulation.setDeviceMetricsOverride` doesn't resize WebView2; to test narrow layouts, set the width
  of `.app` from script instead.
- `Runtime.evaluate` doesn't allow top-level `await` in object literals; wrap in an async function.
- Drops: CDP `Input.dispatchDragEvent` with `items` (e.g. `text/uri-list`, `text/html`) tests the
  app's drop logic; its `files` option gives files the page can't read. For what browsers and
  Explorer really hand over, do real drags with user32 `SetCursorPos` + `mouse_event` (run
  inline in PowerShell; `-ExecutionPolicy Bypass` gets blocked). Use Edge with `--inprivate`:
  a fresh profile signs in to the owner's Microsoft account and blocks the page with a sync notice.
  Find Explorer items with UI Automation (`BoundingRectangle`). Serve test pages from a local Node
  server in the scratchpad.

### Gotchas

- The Vite dev server sometimes serves a component's old CSS after edits; reload, restart dev, or
  check the release build before assuming a styling bug.
- Memory: report Task Manager's figure (per-process "private working set" from
  `Win32_PerfFormattedData_PerfProc_Process`), not `PrivateMemorySize64`, which overstates the GPU
  process badly. The GPU process cache varies a lot between runs (≈60–250 MB); the Rust process is
  typically under 15 MB. A release build with a large workspace open sits around 125–160 MB.
- `python` on this machine is the Windows Store stub and waits for input; use Node for scripts.

## Releases and Git

- Each feature becomes a minor version. Bump `package.json`, `src-tauri/Cargo.toml` and
  `src-tauri/tauri.conf.json` together, then `npm install --package-lock-only` to update the lock file.
- Update README.md (how to use it, file layout) and this file when something they describe changes.
- The owner approves every commit and push to GitHub (`KaiGulliksen/reference-library`, private,
  branch `main`). Commit messages: a short summary line with the version, then a plain-language
  list of what changed.
