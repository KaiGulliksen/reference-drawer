//! Finding the image files inside a folder.

use serde::Serialize;
use std::fs::Metadata;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::{DirEntry, WalkDir};

/// File types the app shows. All of these can be displayed by the system's web view
/// (WebView2 on Windows, WebKit on macOS and Linux; AVIF needs a recent one). All except
/// AVIF can also be turned into thumbnails by the `image` crate (AVIF thumbnails fall
/// back to the original file).
pub const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "jfif", "png", "webp", "gif", "bmp", "avif"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageEntry {
    pub path: String,
    pub name: String,
    /// File size in bytes.
    pub size: u64,
    /// Last modified time, in milliseconds since 1970 (what JavaScript's `Date` expects).
    pub modified: u64,
    /// Size in pixels as displayed (after EXIF rotation); 0 × 0 if it couldn't be read.
    /// Filled in from the size cache after scanning (see sizes.rs).
    pub width: u32,
    pub height: u32,
}

/// A file's size in bytes and last modified time in milliseconds, as stored in `ImageEntry`.
pub fn file_stamp(meta: &Metadata) -> (u64, u64) {
    let modified = meta
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64);
    (meta.len(), modified)
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

/// Names starting with "." are hidden on macOS and Linux (and include macOS's "._" copies
/// on shared drives); names starting with "$" are Windows system folders like $RECYCLE.BIN.
fn is_hidden(entry: &DirEntry) -> bool {
    entry.file_name().to_str().is_some_and(|name| name.starts_with('.') || name.starts_with('$'))
}

/// The details of one image file, or None if it isn't there (or isn't an image). For images that
/// a scan may not list, like those in subfolders with "Subfolders" off. Pixel size still needs filling in.
pub fn entry_for(path: &Path) -> Option<ImageEntry> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || !is_image(path) {
        return None;
    }
    let (size, modified) = file_stamp(&meta);
    Some(ImageEntry {
        path: path.to_string_lossy().into_owned(),
        name: path.file_name()?.to_string_lossy().into_owned(),
        size,
        modified,
        width: 0,
        height: 0,
    })
}

pub fn scan(folder: &Path, recursive: bool) -> Vec<ImageEntry> {
    let max_depth = if recursive { usize::MAX } else { 1 };
    WalkDir::new(folder)
        .min_depth(1)
        .max_depth(max_depth)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_hidden(entry))
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() && is_image(entry.path()))
        .filter_map(|entry| {
            let (size, modified) = file_stamp(&entry.metadata().ok()?);
            Some(ImageEntry {
                path: entry.path().to_string_lossy().into_owned(),
                name: entry.file_name().to_string_lossy().into_owned(),
                size,
                modified,
                width: 0,
                height: 0,
            })
        })
        .collect()
}
