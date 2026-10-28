//! Saving images that are dragged into the app from elsewhere (a web browser, another app, a
//! file manager) into a workspace's folder.
//!
//! This is the only place the app creates files (moving.rs moves them, using the same naming),
//! and it only ever adds new ones: a name that's taken gets " (2)", " (3)", … added, and an image
//! that's already in the folder isn't saved a second time. Nothing that's already there is ever
//! replaced.

use crate::scan;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Downloads bigger than this are refused (no reference image is this big; a web page that
/// keeps sending data is more likely).
const MAX_DOWNLOAD_BYTES: u64 = 200 * 1024 * 1024;
/// How long a download may take before giving up.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(60);
/// Longest file name (without the extension) the app makes. Very long names come from web
/// addresses and are awkward in every file manager.
const MAX_NAME_CHARS: usize = 100;

/// Where a dropped image ended up.
pub struct Saved {
    pub path: PathBuf,
    /// True when the very same image was already in the workspace, so nothing new was saved.
    pub already: bool,
}

/// Saves `bytes` as an image file in `folder`, named after `wanted_name` where possible.
/// If the same image is already in the folder (or a subfolder, when `recursive`), that file
/// is returned instead.
pub fn save(folder: &Path, recursive: bool, wanted_name: &str, bytes: &[u8]) -> Result<Saved, String> {
    let extension = image_extension(bytes)
        .ok_or("That isn't an image the app can show (JPG, PNG, WebP, GIF, BMP or AVIF).")?;
    if let Some(path) = find_copy(folder, recursive, bytes) {
        return Ok(Saved { path, already: true });
    }
    let (stem, extension) = file_name(wanted_name, extension);
    let path = create_new(folder, &stem, &extension, bytes)
        .map_err(|e| format!("Couldn't save the image in {}: {e}", folder.display()))?;
    Ok(Saved { path, already: false })
}

/// Downloads an image from a web address. Returns a file name for it (from the address)
/// and its contents.
pub fn download(url: &str) -> Result<(String, Vec<u8>), String> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err("The app can only download images from web addresses (http or https).".into());
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        // Some image hosts turn away requests that don't say what they come from.
        .user_agent(concat!("Mozilla/5.0 (compatible; ReferenceLibrary/", env!("CARGO_PKG_VERSION"), ")"))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("Accept", "image/avif,image/webp,image/png,image/*;q=0.9,*/*;q=0.5")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(code) => format!("The website refused to send the image (error {code})."),
            e => format!("Couldn't download the image: {e}"),
        })?;
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .read_to_vec()
        .map_err(|e| format!("Couldn't download the image: {e}"))?;
    Ok((name_from_url(url), bytes))
}

/// The file extension for an image, judged by its first bytes rather than by its name or web
/// address (websites often send a WebP for an address ending in ".jpg"). None if it isn't one
/// of the formats the app shows (see `scan::IMAGE_EXTENSIONS`).
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    let starts = |prefix: &[u8]| bytes.starts_with(prefix);
    if starts(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if starts(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if starts(b"GIF87a") || starts(b"GIF89a") {
        Some("gif")
    } else if starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("webp")
    } else if starts(b"BM") && bytes.len() > 26 {
        Some("bmp")
    } else if bytes.get(4..8) == Some(b"ftyp") && matches!(bytes.get(8..12), Some(b"avif" | b"avis")) {
        Some("avif")
    } else {
        None
    }
}

/// Looks for a file in the folder with exactly these contents. Only files of the same size
/// are read, which is usually none of them.
fn find_copy(folder: &Path, recursive: bool, bytes: &[u8]) -> Option<PathBuf> {
    scan::scan(folder, recursive)
        .into_iter()
        .filter(|entry| entry.size == bytes.len() as u64)
        .map(|entry| PathBuf::from(entry.path))
        .find(|path| fs::read(path).is_ok_and(|contents| contents == bytes))
}

/// A safe file name (stem and extension) for an image called `wanted` whose real format has
/// `extension`. The original extension is kept if it matches the format ("photo.jpeg" stays
/// .jpeg), otherwise the right one replaces it ("photo.jpg" that is really a WebP becomes
/// "photo.webp").
///
/// The rules are Windows' (the strictest of the three systems), so names also work if the
/// folder is later opened on another system.
fn file_name(wanted: &str, extension: &'static str) -> (String, String) {
    // Only the last part, if a path or web address slipped through.
    let wanted = wanted.rsplit(['/', '\\']).next().unwrap_or("");
    let (stem, original) = match wanted.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (wanted, None),
    };
    let same_format = |ext: &str| {
        ext.eq_ignore_ascii_case(extension)
            || (extension == "jpg" && ["jpeg", "jfif"].iter().any(|e| ext.eq_ignore_ascii_case(e)))
    };
    let extension = match original {
        Some(ext) if same_format(ext) => ext.to_owned(),
        _ => extension.to_owned(),
    };

    // Characters Windows doesn't allow in names, and control characters, become spaces.
    let cleaned: String = stem
        .chars()
        .map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { ' ' } else { c })
        .collect();
    let mut stem = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if stem.chars().count() > MAX_NAME_CHARS {
        stem = stem.chars().take(MAX_NAME_CHARS).collect();
    }
    // Names starting with "." or "$" are hidden or system files (the scan skips them, see
    // scan.rs), and Windows doesn't allow names ending in a dot or space.
    let stem = stem.trim_start_matches(['.', '$', ' ']).trim_end_matches(['.', ' ']);
    let stem = if stem.is_empty() {
        "image".to_owned()
    } else if is_reserved_name(stem) {
        format!("{stem} image")
    } else {
        stem.to_owned()
    };
    (stem, extension)
}

/// Names Windows keeps for devices (CON, NUL, COM1, …), which can't be used for files.
fn is_reserved_name(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit())
}

/// Writes a new file "stem.ext", or "stem (2).ext", "stem (3).ext", … if that's taken.
fn create_new(folder: &Path, stem: &str, extension: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
    let (path, mut file) = new_file(folder, stem, extension, |_| false)?;
    if let Err(e) = file.write_all(bytes) {
        // Don't leave half an image behind. This is the file just made here, nothing else.
        drop(file);
        let _ = fs::remove_file(&path);
        return Err(e);
    }
    Ok(path)
}

/// Makes a new, empty file "stem.ext" in `folder`, or "stem (2).ext", "stem (3).ext", … if that
/// name is taken on disk or `avoid` says so. `create_new` makes the system refuse to open a file
/// that already exists, so even a file that appears at the same moment is never overwritten.
pub(crate) fn new_file(
    folder: &Path,
    stem: &str,
    extension: &str,
    avoid: impl Fn(&Path) -> bool,
) -> std::io::Result<(PathBuf, File)> {
    for n in 1..=10_000 {
        let name = if n == 1 { format!("{stem}.{extension}") } else { format!("{stem} ({n}).{extension}") };
        let path = folder.join(name);
        if path.exists() || avoid(&path) {
            continue; // includes a folder of that name
        }
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("too many files with the same name"))
}

/// A file name from the last part of a web address, e.g. "cat%20photo.jpg?w=800" → "cat photo.jpg".
fn name_from_url(url: &str) -> String {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let after_host = path.splitn(4, '/').nth(3).unwrap_or("");
    let last = after_host.rsplit('/').find(|part| !part.is_empty()).unwrap_or("");
    percent_encoding::percent_decode_str(last).decode_utf8_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The smallest files that look like each format (only the first bytes are checked).
    fn png(extra: u8) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend([extra; 40]);
        bytes
    }

    fn webp() -> Vec<u8> {
        let mut bytes = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        bytes.extend([0; 30]);
        bytes
    }

    fn temp_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("reflib-import-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn names_in(folder: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            fs::read_dir(folder).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        names
    }

    #[test]
    fn recognises_formats_by_their_contents() {
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Some("jpg"));
        assert_eq!(image_extension(&png(0)), Some("png"));
        assert_eq!(image_extension(b"GIF89a......"), Some("gif"));
        assert_eq!(image_extension(&webp()), Some("webp"));
        assert_eq!(image_extension(b"\0\0\0\x1cftypavif\0\0\0\0"), Some("avif"));
        assert_eq!(image_extension(b"<!DOCTYPE html><html>"), None, "a web page isn't an image");
        assert_eq!(image_extension(b""), None);
    }

    #[test]
    fn makes_safe_names_with_the_right_extension() {
        assert_eq!(file_name("cat.jpg", "jpg"), ("cat".into(), "jpg".into()));
        assert_eq!(file_name("Cat.JPEG", "jpg"), ("Cat".into(), "JPEG".into()), "the original spelling is kept");
        assert_eq!(file_name("photo.jpg", "webp"), ("photo".into(), "webp".into()), "the real format wins");
        assert_eq!(file_name("download", "png"), ("download".into(), "png".into()));
        assert_eq!(file_name("", "png"), ("image".into(), "png".into()));
        assert_eq!(file_name(".hidden.png", "png"), ("hidden".into(), "png".into()), "never a hidden file");
        assert_eq!(file_name("a<b>:c?.png", "png"), ("a b c".into(), "png".into()));
        assert_eq!(file_name("ends with dot..png", "png"), ("ends with dot".into(), "png".into()));
        assert_eq!(file_name("con.png", "png"), ("con image".into(), "png".into()));
        assert_eq!(file_name("COM1.png", "png"), ("COM1 image".into(), "png".into()));
        assert_eq!(file_name("folder/inner.png", "png"), ("inner".into(), "png".into()));
        assert_eq!(file_name("C:\\Users\\x\\pic.gif", "gif"), ("pic".into(), "gif".into()));
        let (long, _) = file_name(&format!("{}.png", "ä".repeat(300)), "png");
        assert_eq!(long.chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn names_come_from_the_last_part_of_web_addresses() {
        assert_eq!(name_from_url("https://example.com/art/cat%20photo.jpg?w=800#top"), "cat photo.jpg");
        assert_eq!(name_from_url("https://example.com/art/folder/"), "folder");
        assert_eq!(name_from_url("https://example.com"), "");
        assert_eq!(name_from_url("https://example.com/?id=3"), "");
    }

    #[test]
    fn never_replaces_existing_files() {
        let folder = temp_folder("names");
        fs::write(folder.join("cat.png"), b"someone else's file").unwrap();
        fs::create_dir(folder.join("cat (2).png")).unwrap(); // a folder with that name

        let saved = save(&folder, false, "cat.png", &png(1)).unwrap();
        assert!(!saved.already);
        assert_eq!(saved.path, folder.join("cat (3).png"));
        assert_eq!(fs::read(folder.join("cat.png")).unwrap(), b"someone else's file", "untouched");

        let other = save(&folder, false, "cat.png", &png(2)).unwrap();
        assert_eq!(other.path, folder.join("cat (4).png"));
        assert_eq!(names_in(&folder), ["cat (2).png", "cat (3).png", "cat (4).png", "cat.png"]);
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn the_same_image_is_not_saved_twice() {
        let folder = temp_folder("same");
        let first = save(&folder, false, "a.png", &png(7)).unwrap();
        let again = save(&folder, false, "different name.png", &png(7)).unwrap();
        assert!(again.already);
        assert_eq!(again.path, first.path);
        // Same size, different contents: a new file.
        let different = save(&folder, false, "a.png", &png(8)).unwrap();
        assert!(!different.already);

        // A copy in a subfolder counts only when subfolders are part of the workspace.
        let sub = folder.join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("deep.webp"), webp()).unwrap();
        assert!(save(&folder, true, "x.webp", &webp()).unwrap().already);
        let top = save(&folder, false, "x.webp", &webp()).unwrap();
        assert!(!top.already);
        assert_eq!(top.path, folder.join("x.webp"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn refuses_what_isnt_an_image() {
        let folder = temp_folder("refuse");
        assert!(save(&folder, false, "page.jpg", b"<html>not found</html>").is_err());
        assert!(names_in(&folder).is_empty(), "nothing is written");
        assert!(download("file:///etc/passwd").is_err());
        fs::remove_dir_all(&folder).unwrap();
    }
}
