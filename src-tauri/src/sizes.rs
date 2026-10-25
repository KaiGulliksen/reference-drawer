//! Remembers the pixel size of every image, so layouts that need it (the masonry grid)
//! can be drawn straight away instead of opening every file on each scan.
//!
//! This is only a cache, kept in the app's cache folder (`sizes.db`). Deleting it is safe:
//! sizes are read again from the files, which only reads their headers.

use crate::platform::path_key;
use crate::scan::ImageEntry;
use crate::thumbs;
use rusqlite::{Connection, OptionalExtension, params};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;
use std::sync::Mutex;
use std::thread;

pub struct SizeCache {
    conn: Mutex<Connection>,
}

impl SizeCache {
    pub fn open(file: &Path) -> Result<Self, String> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let conn = match Self::connect(file) {
            Ok(conn) => conn,
            // A damaged cache isn't worth an error: start a fresh one.
            Err(_) => {
                let _ = fs::remove_file(file);
                Self::connect(file).map_err(|e| e.to_string())?
            }
        };
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn connect(file: &Path) -> rusqlite::Result<Connection> {
        let conn = Connection::open(file)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // The file's size and modified time are part of the entry, so an edited image is measured again.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sizes (
                path     TEXT    PRIMARY KEY,
                size     INTEGER NOT NULL,
                modified INTEGER NOT NULL,
                width    INTEGER NOT NULL,
                height   INTEGER NOT NULL
            ) WITHOUT ROWID;",
        )?;
        Ok(conn)
    }

    /// Fills in `width` and `height` for every image: from the cache where possible,
    /// otherwise by reading the file's header (several files at a time).
    pub fn fill(&self, images: &mut [ImageEntry]) {
        let mut missing = Vec::new();
        {
            let conn = self.conn.lock().unwrap();
            let Ok(mut find) =
                conn.prepare("SELECT width, height FROM sizes WHERE path = ?1 AND size = ?2 AND modified = ?3")
            else {
                return;
            };
            for (i, image) in images.iter_mut().enumerate() {
                let found: Option<(u32, u32)> = find
                    .query_row(params![path_key(&image.path), image.size as i64, image.modified as i64], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()
                    .unwrap_or(None);
                match found {
                    Some((width, height)) => (image.width, image.height) = (width, height),
                    None => missing.push(i),
                }
            }
        }
        if missing.is_empty() {
            return;
        }

        for (i, width, height) in read_sizes(images, &missing) {
            (images[i].width, images[i].height) = (width, height);
        }

        // Remember them, including the ones that couldn't be read (0 × 0), so broken files
        // aren't tried again on every scan. Saving is best effort; it's only a cache.
        let mut conn = self.conn.lock().unwrap();
        if let Ok(tx) = conn.transaction() {
            if let Ok(mut save) = tx.prepare(
                "INSERT OR REPLACE INTO sizes (path, size, modified, width, height) VALUES (?1, ?2, ?3, ?4, ?5)",
            ) {
                for &i in &missing {
                    let image = &images[i];
                    let _ = save.execute(params![
                        path_key(&image.path),
                        image.size as i64,
                        image.modified as i64,
                        image.width,
                        image.height
                    ]);
                }
            }
            let _ = tx.commit();
        }
    }
}

/// Reads the sizes of `images[i]` for each index in `which`, spread over several threads.
fn read_sizes(images: &[ImageEntry], which: &[usize]) -> Vec<(usize, u32, u32)> {
    let threads = thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 8);
    let chunk = which.len().div_ceil(threads).max(1);
    thread::scope(|scope| {
        let workers: Vec<_> = which
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .map(|&i| {
                            let path = Path::new(&images[i].path);
                            // A broken file must never take the scan down, so catch panics too.
                            let (w, h) = catch_unwind(AssertUnwindSafe(|| thumbs::dimensions(path).ok()))
                                .ok()
                                .flatten()
                                .unwrap_or((0, 0));
                            (i, w, h)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn entry(path: &Path) -> ImageEntry {
        let meta = fs::metadata(path).unwrap();
        let modified = meta.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
        ImageEntry {
            path: path.to_string_lossy().into_owned(),
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            size: meta.len(),
            modified,
            width: 0,
            height: 0,
        }
    }

    fn save_png(path: &Path, w: u32, h: u32) {
        ImageBuffer::<Rgb<u8>, _>::from_pixel(w, h, Rgb([200, 100, 50])).save(path).unwrap();
    }

    #[test]
    fn sizes_are_read_then_remembered_then_refreshed_when_a_file_changes() {
        let dir = std::env::temp_dir().join(format!("reflib-test-sizes-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let (wide, broken) = (dir.join("wide.png"), dir.join("broken.jpg"));
        save_png(&wide, 300, 100);
        fs::write(&broken, b"not really a jpeg").unwrap();
        let cache = SizeCache::open(&dir.join("sizes.db")).unwrap();

        let mut images = vec![entry(&wide), entry(&broken)];
        cache.fill(&mut images);
        assert_eq!((images[0].width, images[0].height), (300, 100));
        assert_eq!((images[1].width, images[1].height), (0, 0), "unreadable files get 0 × 0");

        // Second time round the sizes come from the cache: prove it by planting a fake entry.
        cache
            .conn
            .lock()
            .unwrap()
            .execute("UPDATE sizes SET width = 1234 WHERE path = ?1", [path_key(&images[0].path)])
            .unwrap();
        let mut again = vec![entry(&wide)];
        cache.fill(&mut again);
        assert_eq!(again[0].width, 1234, "taken from the cache, not the file");

        // Editing the file changes its size and modified time, so it is measured again.
        std::thread::sleep(std::time::Duration::from_millis(20));
        save_png(&wide, 64, 128);
        let mut edited = vec![entry(&wide)];
        cache.fill(&mut edited);
        assert_eq!((edited[0].width, edited[0].height), (64, 128));
    }

    #[test]
    fn sideways_photos_report_their_displayed_size() {
        // The test images include a JPEG stored sideways with EXIF orientation 6 ("rotate 90°").
        // Build one here: 40 × 20 pixels stored, so it is shown as 20 × 40.
        let dir = std::env::temp_dir().join(format!("reflib-test-exif-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("rotated.jpg");
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode_image(&ImageBuffer::<Rgb<u8>, _>::from_pixel(40, 20, Rgb([1, 2, 3])))
            .unwrap();
        // Insert a minimal EXIF block (APP1) with Orientation = 6 right after the start marker.
        let exif: &[u8] = &[
            b'E', b'x', b'i', b'f', 0, 0, // "Exif\0\0"
            b'I', b'I', 42, 0, 8, 0, 0, 0, // little-endian TIFF header, first directory at offset 8
            1, 0, // one entry
            0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, // tag 0x0112 (Orientation), SHORT, count 1, value 6
            0, 0, 0, 0, // no next directory
        ];
        let mut with_exif = vec![0xFF, 0xD8, 0xFF, 0xE1];
        with_exif.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        with_exif.extend_from_slice(exif);
        with_exif.extend_from_slice(&jpeg[2..]);
        fs::write(&file, with_exif).unwrap();

        let cache = SizeCache::open(&dir.join("sizes.db")).unwrap();
        let mut images = vec![entry(&file)];
        cache.fill(&mut images);
        assert_eq!((images[0].width, images[0].height), (20, 40));
    }
}
