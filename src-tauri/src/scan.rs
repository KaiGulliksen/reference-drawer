//! Finding the image files inside a folder.

use serde::Serialize;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::{DirEntry, WalkDir};

/// File types the app shows. All of these can be displayed by WebView2; all
/// except AVIF can also be turned into thumbnails by the `image` crate (AVIF
/// thumbnails fall back to the original file).
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
}

pub fn is_image(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| IMAGE_EXTENSIONS.iter().any(|known| ext.eq_ignore_ascii_case(known)))
}

fn is_hidden(entry: &DirEntry) -> bool {
    entry.file_name().to_str().is_some_and(|name| name.starts_with('.') || name.starts_with('$'))
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
            let meta = entry.metadata().ok()?;
            let modified = meta
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            Some(ImageEntry {
                path: entry.path().to_string_lossy().into_owned(),
                name: entry.file_name().to_string_lossy().into_owned(),
                size: meta.len(),
                modified,
            })
        })
        .collect()
}
