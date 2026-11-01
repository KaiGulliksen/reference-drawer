mod backup;
mod boards;
mod identity;
mod import;
mod library;
mod moving;
mod parallel;
mod platform;
mod pool;
mod scan;
mod sizes;
mod thumbs;
mod watch;

use boards::{Board, BoardData, BoardItem, BoardView, NewImage};
use library::{Library, Tag, TagData, Workspace, relative_path};
use pool::WorkerPool;
use scan::ImageEntry;
use serde::Serialize;
use sizes::SizeCache;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, State, UriSchemeContext, UriSchemeResponder, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use watch::FolderWatch;

/// Set this to use a different folder for the library database and caches, e.g. a throwaway one for testing.
const DATA_DIR_ENV: &str = "REFERENCE_LIBRARY_DATA_DIR";

/// Where generated thumbnails are stored (inside %LOCALAPPDATA% on Windows).
struct ThumbCacheDir(PathBuf);

/// Where the library's safety copies are kept (see backup.rs).
struct BackupFolder(PathBuf);

/// When a grid thumbnail can't be generated, the original file is shown instead,
/// but only if it is small enough not to slow the grid down.
const MAX_FALLBACK_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Serialize)]
struct ScanResult {
    folder: String,
    images: Vec<ImageEntry>,
    /// How many tagged images were found again under a new name or in a new place.
    relinked: usize,
}

/// Where an image dropped onto the window was saved.
#[derive(Serialize)]
struct Imported {
    path: String,
    /// True when the same image was already in the workspace, so nothing new was saved.
    already: bool,
}

#[derive(Serialize)]
struct ImageSize {
    width: u32,
    height: u32,
}

/// Lists the images in a workspace's folder, and brings its tags up to date with what's there
/// (so tags follow images that were renamed or moved; see `Library::reconcile`).
#[tauri::command]
async fn scan_workspace(app: AppHandle, library: State<'_, Library>, id: i64) -> Result<ScanResult, String> {
    let workspace = library.workspace(id)?;
    let folder = PathBuf::from(&workspace.path);
    if !folder.is_dir() {
        return Err(format!("Folder not found: {}", workspace.path));
    }

    // Allow the webview to load full-size images (and thumbnails) from this folder only.
    app.asset_protocol_scope()
        .allow_directory(&folder, workspace.recursive)
        .map_err(|e| e.to_string())?;

    let to_scan = folder.clone();
    let (images, relinked) = tauri::async_runtime::spawn_blocking(move || {
        // Watch the folder from now on, so later changes show up by themselves (see watch.rs).
        // Started before scanning, so nothing that changes during the scan is missed.
        app.state::<FolderWatch>().watch(&app, id, &to_scan, workspace.recursive);
        let mut images = scan::scan(&to_scan, workspace.recursive);
        app.state::<SizeCache>().fill(&mut images);
        // A problem here shouldn't stop the images from showing; the tags just stay where they were.
        let relinked = app.state::<Library>().reconcile(id, &images).unwrap_or_else(|e| {
            eprintln!("Couldn't update tags after scanning: {e}");
            0
        });
        (images, relinked)
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok(ScanResult { folder: folder.to_string_lossy().into_owned(), images, relinked })
}

#[tauri::command]
async fn list_workspaces(library: State<'_, Library>) -> Result<Vec<Workspace>, String> {
    library.workspaces()
}

#[tauri::command]
async fn add_workspace(library: State<'_, Library>, path: String) -> Result<Workspace, String> {
    library.add_workspace(&path)
}

#[tauri::command]
async fn rename_workspace(library: State<'_, Library>, id: i64, name: String) -> Result<Workspace, String> {
    library.rename_workspace(id, &name)
}

/// Gives a workspace one of the library's colours (`COLORS`), or none.
#[tauri::command]
async fn set_workspace_color(library: State<'_, Library>, id: i64, color: Option<String>) -> Result<Workspace, String> {
    library.set_workspace_color(id, color.as_deref())
}

#[tauri::command]
async fn set_workspace_recursive(library: State<'_, Library>, id: i64, recursive: bool) -> Result<Workspace, String> {
    library.set_workspace_recursive(id, recursive)
}

#[tauri::command]
async fn set_workspace_folder(library: State<'_, Library>, id: i64, path: String) -> Result<Workspace, String> {
    library.set_workspace_folder(id, &path)
}

#[tauri::command]
async fn remove_workspace(library: State<'_, Library>, id: i64) -> Result<(), String> {
    library.remove_workspace(id)
}

#[tauri::command]
async fn get_tags(library: State<'_, Library>, workspace_id: i64) -> Result<TagData, String> {
    library.tag_data(workspace_id)
}

#[tauri::command]
async fn add_tag(
    app: AppHandle,
    library: State<'_, Library>,
    workspace_id: i64,
    paths: Vec<String>,
    name: String,
) -> Result<Tag, String> {
    let tag = library.add_tag(workspace_id, &paths, &name)?;
    // Fingerprint the newly tagged images in the background (it reads a little of each file),
    // so they're recognised if they're renamed or moved even before the next scan.
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = app.state::<Library>().fingerprint_new(workspace_id, &paths) {
            eprintln!("Couldn't fingerprint newly tagged images: {e}");
        }
    });
    Ok(tag)
}

#[tauri::command]
async fn remove_tag(
    library: State<'_, Library>,
    workspace_id: i64,
    paths: Vec<String>,
    tag_id: i64,
) -> Result<bool, String> {
    library.remove_tag(workspace_id, &paths, tag_id)
}

#[tauri::command]
async fn rename_tag(
    library: State<'_, Library>,
    workspace_id: i64,
    tag_id: i64,
    name: String,
) -> Result<TagData, String> {
    library.rename_tag(workspace_id, tag_id, &name)
}

#[tauri::command]
async fn set_tag_color(
    library: State<'_, Library>,
    workspace_id: i64,
    tag_id: i64,
    color: Option<String>,
) -> Result<(), String> {
    library.set_tag_color(workspace_id, tag_id, color.as_deref())
}

#[tauri::command]
async fn delete_tag(library: State<'_, Library>, workspace_id: i64, tag_id: i64) -> Result<(), String> {
    library.delete_tag(workspace_id, tag_id)
}

#[tauri::command]
async fn list_boards(library: State<'_, Library>, workspace_id: i64) -> Result<Vec<Board>, String> {
    library.boards(workspace_id)
}

#[tauri::command]
async fn create_board(library: State<'_, Library>, workspace_id: i64, name: String) -> Result<Board, String> {
    library.create_board(workspace_id, &name)
}

#[tauri::command]
async fn rename_board(library: State<'_, Library>, id: i64, name: String) -> Result<Board, String> {
    library.rename_board(id, &name)
}

#[tauri::command]
async fn set_board_color(library: State<'_, Library>, id: i64, color: Option<String>) -> Result<Board, String> {
    library.set_board_color(id, color.as_deref())
}

#[tauri::command]
async fn delete_board(library: State<'_, Library>, id: i64) -> Result<(), String> {
    library.delete_board(id)
}

#[tauri::command]
async fn get_board(library: State<'_, Library>, id: i64) -> Result<BoardData, String> {
    library.board_data(id)
}

#[tauri::command]
async fn add_board_images(
    app: AppHandle,
    library: State<'_, Library>,
    board_id: i64,
    images: Vec<NewImage>,
) -> Result<Vec<BoardItem>, String> {
    let added = library.add_board_images(board_id, &images)?;
    // Like tagging: fingerprint them in the background, so they follow renames and moves.
    let workspace_id = library.board_data(board_id)?.board.workspace_id;
    let paths: Vec<String> = images.into_iter().map(|image| image.path).collect();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = app.state::<Library>().fingerprint_new(workspace_id, &paths) {
            eprintln!("Couldn't fingerprint images added to a board: {e}");
        }
    });
    Ok(added)
}

#[tauri::command]
async fn add_board_note(
    library: State<'_, Library>,
    board_id: i64,
    text: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<BoardItem, String> {
    library.add_board_note(board_id, &text, x, y, width, height)
}

#[tauri::command]
async fn update_board_items(library: State<'_, Library>, board_id: i64, items: Vec<BoardItem>) -> Result<(), String> {
    library.update_board_items(board_id, &items)
}

#[tauri::command]
async fn remove_board_items(library: State<'_, Library>, board_id: i64, ids: Vec<i64>) -> Result<(), String> {
    library.remove_board_items(board_id, &ids)
}

/// Puts items back on a board (undo). Like adding images: fingerprinted in the background,
/// as their records may have been forgotten while they were off the board.
#[tauri::command]
async fn restore_board_items(
    app: AppHandle,
    library: State<'_, Library>,
    board_id: i64,
    items: Vec<BoardItem>,
) -> Result<Vec<BoardItem>, String> {
    let restored = library.restore_board_items(board_id, &items)?;
    let workspace_id = library.board_data(board_id)?.board.workspace_id;
    let paths: Vec<String> = items.into_iter().filter_map(|item| item.path).collect();
    tauri::async_runtime::spawn_blocking(move || {
        if let Err(e) = app.state::<Library>().fingerprint_new(workspace_id, &paths) {
            eprintln!("Couldn't fingerprint images put back on a board: {e}");
        }
    });
    Ok(restored)
}

/// The image files on a board that are really there, looked up directly: a board can hold images
/// the workspace's scan doesn't list, like those in subfolders with "Subfolders" off. Their paths
/// come from the library (always inside the workspace's folder), and the web view is allowed to
/// load just these files, not the subfolders around them.
#[tauri::command]
async fn board_files(app: AppHandle, library: State<'_, Library>, board_id: i64) -> Result<Vec<ImageEntry>, String> {
    let paths: Vec<String> = library.board_data(board_id)?.items.into_iter().filter_map(|item| item.path).collect();
    tauri::async_runtime::spawn_blocking(move || {
        let mut files: Vec<ImageEntry> =
            parallel::map(&paths, |path| scan::entry_for(Path::new(path))).into_iter().flatten().collect();
        app.state::<SizeCache>().fill(&mut files);
        for file in &files {
            app.asset_protocol_scope().allow_file(&file.path).map_err(|e| e.to_string())?;
        }
        Ok(files)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The folder the library's safety copies are in (see backup.rs), made if it isn't there yet so
/// it can be shown in the file manager.
#[tauri::command]
async fn backups_folder(folder: State<'_, BackupFolder>) -> Result<String, String> {
    fs::create_dir_all(&folder.0).map_err(|e| e.to_string())?;
    Ok(folder.0.to_string_lossy().into_owned())
}

#[tauri::command]
async fn set_board_view(library: State<'_, Library>, board_id: i64, view: BoardView) -> Result<(), String> {
    library.set_board_view(board_id, view)
}

/// Saves an image dropped onto the window into a workspace's folder. The image arrives as the
/// raw request body (much smaller and faster than a list of numbers in JSON); which workspace,
/// and the name the image had, come in the request's headers.
#[tauri::command]
async fn import_image(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<Imported, String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err("No image was received.".into());
    };
    let header = |name: &str| request.headers().get(name).and_then(|value| value.to_str().ok());
    let workspace_id = header("workspace-id").and_then(|id| id.parse().ok()).ok_or("No workspace was given.")?;
    // Headers can only hold plain ASCII, so the name is percent-encoded.
    let name = percent_encoding::percent_decode_str(header("file-name").unwrap_or("")).decode_utf8_lossy().into_owned();
    save_dropped(app, workspace_id, name, bytes.clone()).await
}

/// Downloads an image that was dropped onto the window as just a web address, and saves it into
/// a workspace's folder.
#[tauri::command]
async fn import_image_url(app: AppHandle, workspace_id: i64, url: String) -> Result<Imported, String> {
    app.state::<Library>().workspace(workspace_id)?; // don't download for a workspace that's gone
    let (name, bytes) = tauri::async_runtime::spawn_blocking(move || import::download(&url))
        .await
        .map_err(|e| e.to_string())??;
    save_dropped(app, workspace_id, name, bytes).await
}

async fn save_dropped(app: AppHandle, workspace_id: i64, name: String, bytes: Vec<u8>) -> Result<Imported, String> {
    let workspace = app.state::<Library>().workspace(workspace_id)?;
    let folder = PathBuf::from(&workspace.path);
    if !folder.is_dir() {
        return Err(format!("Folder not found: {}", workspace.path));
    }
    let saved = tauri::async_runtime::spawn_blocking(move || import::save(&folder, workspace.recursive, &name, &bytes))
        .await
        .map_err(|e| e.to_string())??;
    Ok(Imported { path: saved.path.to_string_lossy().into_owned(), already: saved.already })
}

/// What happened when images were moved to another workspace.
#[derive(Serialize)]
struct MoveResult {
    /// Where the moved images are now.
    moved: Vec<String>,
    /// How many were already in the other workspace's folder, so stayed where they were.
    already: usize,
    /// What went wrong with the ones that couldn't be moved (they're still where they were).
    failed: Vec<String>,
}

/// The boards moving these images to another workspace would take them off (see
/// `Library::boards_left_by_move`), so the interface can ask first.
#[tauri::command]
async fn boards_left_by_move(
    library: State<'_, Library>,
    from_workspace_id: i64,
    to_workspace_id: i64,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    library.boards_left_by_move(from_workspace_id, to_workspace_id, &paths)
}

/// Moves image files from one workspace's folder into another's (at its top level), and takes
/// their tags along (see moving.rs and `Library::moved_to_workspace`).
#[tauri::command]
async fn move_images(
    app: AppHandle,
    from_workspace_id: i64,
    to_workspace_id: i64,
    paths: Vec<String>,
) -> Result<MoveResult, String> {
    if from_workspace_id == to_workspace_id {
        return Err("Those images are already in that workspace.".into());
    }
    let library = app.state::<Library>();
    let from = PathBuf::from(library.workspace(from_workspace_id)?.path);
    let to = library.workspace(to_workspace_id)?;
    let to_folder = PathBuf::from(&to.path);
    if !to_folder.is_dir() {
        return Err(format!("Folder not found: {}", to.path));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let library = app.state::<Library>();
        let avoid = library.known_paths(to_workspace_id)?;
        let mut moves = Vec::new();
        let mut already = 0;
        let mut failed = Vec::new();
        for path in paths {
            let name = file_name_of(&path);
            // Only ever images from the workspace being moved from, never any other file.
            if relative_path(&from, &path).is_err() {
                failed.push(format!("{name}: it isn't in this workspace's folder."));
                continue;
            }
            match moving::move_file(Path::new(&path), &to_folder, |p| avoid.contains(&platform::path_key(&p.to_string_lossy()))) {
                Ok(Some(new)) => moves.push((path, new.to_string_lossy().into_owned())),
                Ok(None) => already += 1,
                Err(e) => failed.push(format!("{name}: {e}")),
            }
        }
        // The files have moved by now; if the library can't be updated, say so, but the images
        // are still fine (their tags just stay behind).
        if let Err(e) = library.moved_to_workspace(from_workspace_id, to_workspace_id, &moves) {
            failed.push(format!("The images were moved, but their tags couldn't go with them: {e}"));
        }
        Ok(MoveResult { moved: moves.into_iter().map(|(_, new)| new).collect(), already, failed })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The file name at the end of a path, for messages.
fn file_name_of(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_owned(), |name| name.to_string_lossy().into_owned())
}

/// The real width and height of an image (after EXIF rotation), without decoding it.
#[tauri::command]
async fn image_size(app: AppHandle, path: String) -> Result<ImageSize, String> {
    let path = PathBuf::from(path);
    if !app.asset_protocol_scope().is_allowed(&path) {
        return Err("not allowed".into());
    }
    let (width, height) =
        tauri::async_runtime::spawn_blocking(move || thumbs::dimensions(&path).map_err(|e| e.to_string()))
            .await
            .map_err(|e| e.to_string())??;
    Ok(ImageSize { width, height })
}

/// Handles `thumb://` requests, e.g. `http://thumb.localhost/C%3A%5Cart%5Ccat.jpg?s=400`.
/// `s` is the longest side wanted: the grid thumbnail size, or one of the viewer's preview sizes.
/// The work happens on the worker pool so the UI never waits on image decoding.
fn thumb_protocol(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let app = ctx.app_handle().clone();
    let encoded = request.uri().path().trim_start_matches('/');
    let path = PathBuf::from(percent_encoding::percent_decode_str(encoded).decode_utf8_lossy().into_owned());
    let size = request
        .uri()
        .query()
        .and_then(|query| query.split('&').find_map(|pair| pair.strip_prefix("s=")))
        .and_then(|s| s.parse().ok())
        .unwrap_or(thumbs::THUMB_SIZE);

    ctx.app_handle().state::<WorkerPool>().submit(move || {
        let response = if !scan::is_image(&path) || !app.asset_protocol_scope().is_allowed(&path) {
            status(StatusCode::FORBIDDEN)
        } else if size != thumbs::THUMB_SIZE && !thumbs::PREVIEW_SIZES.contains(&size) {
            status(StatusCode::BAD_REQUEST)
        } else {
            let cache_dir = &app.state::<ThumbCacheDir>().0;
            // In the grid, only fall back to originals that are small enough not to slow it down;
            // the viewer needs to show the image whatever its size.
            let fallback_limit = if size == thumbs::THUMB_SIZE { MAX_FALLBACK_BYTES } else { u64::MAX };
            // A broken image file must never take the app down, so catch panics too.
            match catch_unwind(AssertUnwindSafe(|| thumbs::render(cache_dir, &path, size))) {
                Ok(Ok(thumbs::Rendered::Image(thumb))) => image_response(thumb.bytes, thumb.mime),
                Ok(Ok(thumbs::Rendered::UseOriginal)) => original(&path, u64::MAX),
                _ => original(&path, fallback_limit),
            }
        };
        responder.respond(response);
    });
}

fn original(path: &PathBuf, max_bytes: u64) -> Response<Vec<u8>> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() <= max_bytes => match fs::read(path) {
            Ok(bytes) => image_response(bytes, thumbs::mime_for(path)),
            Err(_) => status(StatusCode::NOT_FOUND),
        },
        Ok(_) => status(StatusCode::UNSUPPORTED_MEDIA_TYPE),
        Err(_) => status(StatusCode::NOT_FOUND),
    }
}

fn image_response(bytes: Vec<u8>, mime: &str) -> Response<Vec<u8>> {
    Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
        .body(bytes)
        .unwrap()
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(code).body(Vec::new()).unwrap()
}

/// The folders the library database and the caches are in.
fn data_folders(app: &tauri::App) -> tauri::Result<(PathBuf, PathBuf)> {
    // With DATA_DIR_ENV set (for testing), the caches go in that folder too, so a test
    // run touches nothing of the real app's.
    Ok(match std::env::var_os(DATA_DIR_ENV) {
        Some(dir) => (PathBuf::from(&dir), PathBuf::from(&dir).join("cache")),
        None => (app.path().app_data_dir()?, app.path().app_cache_dir()?),
    })
}

/// Opens the library and caches, and gets everything the commands rely on ready.
fn start_up(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let (data_dir, cache_dir) = data_folders(app)?;
    let database = data_dir.join("library.db");
    app.manage(ThumbCacheDir(cache_dir.join("thumbs")));
    app.manage(SizeCache::open(&cache_dir.join("sizes.db"))?);
    app.manage(BackupFolder(backup::folder(&database)));
    app.manage(Library::open(&database)?);
    // Today's safety copy, in the background so it never holds up opening the window. If it
    // fails the app still works; the copies from earlier days are still there.
    std::thread::spawn(move || {
        if let Err(e) = backup::daily(&database) {
            eprintln!("Couldn't back up the library: {e}");
        }
    });
    if let Some(window) = app.get_webview_window("main") {
        platform::watch_dropped_paths(&window)?;
    }
    Ok(())
}

/// What to tell the user when the app can't start, including where the safety copies are.
fn startup_message(app: &tauri::App, problem: &str) -> String {
    let backups = data_folders(app)
        .map(|(data_dir, _)| format!("\n\nCopies of your library are kept in:\n{}", backup::folder(&data_dir.join("library.db")).display()))
        .unwrap_or_default();
    format!("{problem}\n\nYour images aren't affected.{backups}")
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Leave a core free for the rest of the system, and cap it so decoding many
    // huge images at once can't use an excessive amount of memory.
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    let workers = cores.saturating_sub(1).clamp(2, 6);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(WorkerPool::new(workers))
        .manage(FolderWatch::default())
        .register_asynchronous_uri_scheme_protocol("thumb", thumb_protocol)
        .setup(|app| {
            // A failure here used to end the app without a word (the window never appeared), so
            // say what's wrong instead, then quit. The message is shown once the app's event loop
            // is running, which is why setup itself still finishes normally.
            if let Err(problem) = start_up(app) {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.hide();
                }
                let handle = app.handle().clone();
                app.dialog()
                    .message(startup_message(app, &problem.to_string()))
                    .title("Reference Library can't start")
                    .kind(MessageDialogKind::Error)
                    .show(move |_| handle.exit(1));
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_workspace,
            image_size,
            list_workspaces,
            add_workspace,
            rename_workspace,
            set_workspace_color,
            set_workspace_recursive,
            set_workspace_folder,
            remove_workspace,
            get_tags,
            add_tag,
            remove_tag,
            rename_tag,
            set_tag_color,
            delete_tag,
            list_boards,
            create_board,
            rename_board,
            set_board_color,
            delete_board,
            get_board,
            add_board_images,
            add_board_note,
            update_board_items,
            remove_board_items,
            restore_board_items,
            board_files,
            backups_folder,
            set_board_view,
            import_image,
            import_image_url,
            boards_left_by_move,
            move_images,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
