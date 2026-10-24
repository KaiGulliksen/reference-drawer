mod library;
mod pool;
mod scan;
mod thumbs;

use library::{Library, Workspace};
use pool::WorkerPool;
use scan::ImageEntry;
use serde::Serialize;
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{AppHandle, Manager, State, UriSchemeContext, UriSchemeResponder, Wry};

/// Set this to use a different folder for the library database, e.g. a throwaway one for testing.
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
}

#[derive(Serialize)]
struct ImageSize {
    width: u32,
    height: u32,
}

/// Lists the images in a folder. If `path` is a file, its parent folder is used.
#[tauri::command]
async fn scan_folder(app: AppHandle, path: String, recursive: bool) -> Result<ScanResult, String> {
    let mut folder = PathBuf::from(&path);
    if folder.is_file() {
        folder = folder.parent().map(PathBuf::from).unwrap_or(folder);
    }
    if !folder.is_dir() {
        return Err(format!("Folder not found: {path}"));
    }

    // Allow the webview to load full-size images (and thumbnails) from this folder only.
    app.asset_protocol_scope()
        .allow_directory(&folder, recursive)
        .map_err(|e| e.to_string())?;

    let to_scan = folder.clone();
    let images = tauri::async_runtime::spawn_blocking(move || scan::scan(&to_scan, recursive))
        .await
        .map_err(|e| e.to_string())?;

    Ok(ScanResult { folder: folder.to_string_lossy().into_owned(), images })
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
            let cache_dir = app.path().app_cache_dir()?.join("thumbs");
            app.manage(ThumbCacheDir(cache_dir));

            let data_dir = match std::env::var_os(DATA_DIR_ENV) {
                Some(dir) => PathBuf::from(dir),
                None => app.path().app_data_dir()?,
            };
            app.manage(Library::open(&data_dir.join("library.db"))?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            scan_folder,
            image_size,
            list_workspaces,
            add_workspace,
            rename_workspace,
            set_workspace_recursive,
            set_workspace_folder,
            remove_workspace,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
