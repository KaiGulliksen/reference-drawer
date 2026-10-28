mod boards;
mod identity;
mod import;
mod library;
mod parallel;
mod platform;
mod pool;
mod scan;
mod sizes;
mod thumbs;

use boards::{Board, BoardData, BoardItem, BoardView, NewImage};
use library::{Library, Tag, TagData, Workspace};
use pool::WorkerPool;
use scan::ImageEntry;
use serde::Serialize;
use sizes::SizeCache;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, State, UriSchemeContext, UriSchemeResponder, Wry};

/// Set this to use a different folder for the library database and caches, e.g. a throwaway one for testing.
const DATA_DIR_ENV: &str = "REFERENCE_LIBRARY_DATA_DIR";

/// Where generated thumbnails are stored (inside %LOCALAPPDATA% on Windows).
struct ThumbCacheDir(PathBuf);

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
        .register_asynchronous_uri_scheme_protocol("thumb", thumb_protocol)
        .setup(|app| {
            // With DATA_DIR_ENV set (for testing), the caches go in that folder too, so a test
            // run touches nothing of the real app's.
            let (data_dir, cache_dir) = match std::env::var_os(DATA_DIR_ENV) {
                Some(dir) => (PathBuf::from(&dir), PathBuf::from(&dir).join("cache")),
                None => (app.path().app_data_dir()?, app.path().app_cache_dir()?),
            };
            app.manage(ThumbCacheDir(cache_dir.join("thumbs")));
            app.manage(SizeCache::open(&cache_dir.join("sizes.db"))?);
            app.manage(Library::open(&data_dir.join("library.db"))?);
            if let Some(window) = app.get_webview_window("main") {
                platform::watch_dropped_paths(&window)?;
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
            set_board_view,
            import_image,
            import_image_url,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
