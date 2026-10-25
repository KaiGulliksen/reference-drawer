# Reference Library

A lightweight desktop app for browsing and tagging folders of reference images.
Tauri 2 (Rust backend in `src-tauri/`) + Svelte 5 / SvelteKit (interface in `src/`). See README.md
for features and the code layout.

The owner has basic coding skills but a good grasp of concepts: explain changes in plain language,
and keep code commented the way it is now.

## Goals that shape every change

- **Lightweight.** Low memory and CPU use matter more than features. Measure (Task Manager's
  "private working set") when a change could affect it.
- **Cross-platform.** The app ships on Windows today but must stay buildable and correct on macOS
  and Linux. Every change has to work on all three (rules below).

## Cross-platform rules

- **Platform differences live in one place:** `src/lib/platform.ts` on the interface side, and
  clearly named constants / `cfg!(target_os = …)` in Rust. Don't scatter OS checks around.
- **Shortcuts:** use `hasPrimaryModifier(e)` (Cmd on macOS, Ctrl elsewhere), never `e.ctrlKey`
  directly. On macOS Ctrl+click means right-click. Show shortcuts with `shortcutLabel()` /
  `primaryKeyLabel`, never a hard-coded "Ctrl".
- **Wording:** no Windows-only names in the interface; use `fileManagerName` ("Explorer" /
  "Finder" / "file manager").
- **Paths:** never build or split paths by hand with `\` or `/`. In Rust use `Path`/`PathBuf`.
  Paths stored in the database are relative to the workspace folder and always use `/`
  (`relative_path` / `full_path` in `library.rs`). Paths sent to the interface are written the way
  the system writes them.
- **Capital letters in file names:** the same file on Windows and macOS, different files on Linux.
  Compare paths with `same_path()` in SQL and `pathKey()` in the interface; never lower-case paths
  directly. (Tag names are case-insensitive everywhere; that's a design choice, not a platform one.)
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
  (macOS minutes count 10×), so ask before starting it.

## Database

`src-tauri/src/library.rs`. Schema changes are numbered entries in `MIGRATIONS`, only ever added
at the end; never edit an existing one, since installed copies have already run it. Foreign keys are
switched off while migrating (rebuilding a table would otherwise delete everything linked to it)
and each step is checked with `pragma_foreign_key_check` before it is saved. Add a test for every
migration that starts from the previous version's data.

Tagged images are identified by location *and* content: `scan_workspace` calls
`Library::reconcile`, which keeps tags on files still at their location (even if edited) and moves
the tags of a missing file to an untagged file with the same fingerprint (`identity.rs`). Never
treat a file as missing just because it wasn't in the scan (e.g. "Subfolders" is off): check that
it's really gone. Any change to this logic needs tests in `library.rs` (they use real temp files).

## Commands

```bash
npm run check                                            # type-check the interface
cargo test --manifest-path src-tauri/Cargo.toml --lib    # database tests
npm run tauri dev                                        # run with live reload
npm run tauri build                                      # release build + installer
```

Rust is installed through rustup in `%USERPROFILE%\.cargo\bin`; add it to `PATH` in new shells.

## Testing in the real app without touching the owner's library

The owner uses the installed app, so their real library (`library.db` in the app data folder) and
settings must never be used by tests.

- Point the app at a throwaway database: `REFERENCE_LIBRARY_DATA_DIR=<scratch folder>` (the
  thumbnail and image-size caches then go in `<scratch folder>/cache` too).
- For release builds also set `WEBVIEW2_USER_DATA_FOLDER=<scratch folder>`: the installed app keeps
  its settings in the same web view storage, and the one-time "last folder → first workspace"
  conversion would otherwise be used up.
- `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222` lets a script drive the
  window over the Chrome DevTools Protocol (click, type, screenshot, evaluate).
- The Vite dev server sometimes serves a component's old CSS after edits; reload, or check the
  release build, before assuming a styling bug.

## Releases

Bump the version in `package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json`
together (then `npm install --package-lock-only`). The owner reviews before anything is committed
or pushed to GitHub (`KaiGulliksen/reference-library`, private).
