//! Recognising an image by its contents, so its tags can follow it when the file is
//! renamed or moved (see `Library::reconcile`).
//!
//! Reading whole files would be slow for big folders, so the fingerprint is made from the
//! file's size plus its first and last 16 KB. Two different images practically never match
//! on all three, and only files of exactly the same size are ever compared.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use xxhash_rust::xxh3::Xxh3;

/// How much of the start and of the end of a file goes into its fingerprint.
const SAMPLE_BYTES: u64 = 16 * 1024;

pub fn fingerprint(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Xxh3::new();
    hasher.update(&size.to_le_bytes());

    let mut buffer = vec![0; SAMPLE_BYTES as usize];
    let head = read_up_to(&mut file, &mut buffer)?;
    hasher.update(&buffer[..head]);

    // The end of the file, without overlapping the part already read.
    if size > SAMPLE_BYTES {
        let tail = (size - SAMPLE_BYTES).min(SAMPLE_BYTES);
        file.seek(SeekFrom::Start(size - tail))?;
        let read = read_up_to(&mut file, &mut buffer[..tail as usize])?;
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:032x}", hasher.digest128()))
}

/// Like `read_exact`, but a shorter read at the end of the file is fine.
fn read_up_to(file: &mut File, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..])? {
            0 => break,
            n => filled += n,
        }
    }
    Ok(filled)
}
