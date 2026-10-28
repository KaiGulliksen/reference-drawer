//! Moving image files into another workspace's folder ("Move to workspace").
//!
//! This is the only place the app moves the user's files, and only when they ask for it. A move
//! never replaces anything: the file keeps its name if it's free in the new folder, and otherwise
//! gets " (2)", " (3)", … added, like a dropped image (see `import::new_file`).

use crate::import::new_file;
use crate::platform::path_key;
use crate::scan;
use std::fs::{self, File};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

/// Moves the image file at `from` into `folder` (at its top level). Returns where it is now, or
/// None if it was already in that folder. `avoid` names that mustn't be used even though no file
/// has them (see `Library::known_paths`).
pub fn move_file(from: &Path, folder: &Path, avoid: impl Fn(&Path) -> bool) -> Result<Option<PathBuf>, String> {
    if !scan::is_image(from) || !from.is_file() {
        return Err("It isn't there any more.".into());
    }
    if from.parent().is_some_and(|parent| path_key(&parent.to_string_lossy()) == path_key(&folder.to_string_lossy())) {
        return Ok(None);
    }
    let stem = from.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let extension = from.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();

    // Claim a free name with an empty file first, then put the image in its place. Only that
    // empty file (made just now, by us) can ever be replaced.
    let (to, placeholder) = new_file(folder, &stem, &extension, avoid).map_err(|e| e.to_string())?;
    drop(placeholder);
    match fs::rename(from, &to) {
        Ok(()) => Ok(Some(to)),
        // A different drive: the system can't just move it, so copy it and remove the original.
        Err(e) if e.kind() == ErrorKind::CrossesDevices => match copy_then_remove(from, &to) {
            Ok(()) => Ok(Some(to)),
            Err(e) => {
                let _ = fs::remove_file(&to); // our copy (or the empty file); the original is still there
                Err(e.to_string())
            }
        },
        Err(e) => {
            let _ = fs::remove_file(&to); // the empty file made above
            Err(e.to_string())
        }
    }
}

/// Copies `from` over the (empty) file at `to`, checks the copy is complete, then removes the
/// original. If anything goes wrong before the end, the original is left where it was.
fn copy_then_remove(from: &Path, to: &Path) -> std::io::Result<()> {
    let meta = fs::metadata(from)?;
    let copied = fs::copy(from, to)?;
    if copied != meta.len() || fs::metadata(to)?.len() != meta.len() {
        return Err(std::io::Error::other("the copy came out incomplete"));
    }
    // Keep the modified date (Windows does this by itself; macOS and Linux don't), so sorting by
    // date doesn't change because of the move.
    if let Ok(modified) = meta.modified() {
        let _ = File::options().write(true).open(to).and_then(|file| file.set_modified(modified));
    }
    fs::remove_file(from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("reflib-move-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("from/sub")).unwrap();
        fs::create_dir_all(folder.join("to")).unwrap();
        folder
    }

    #[test]
    fn moves_the_file_and_keeps_its_name() {
        let folder = temp_folder("plain");
        let from = folder.join("from/sub/cat.jpg");
        fs::write(&from, b"cat").unwrap();
        let to = move_file(&from, &folder.join("to"), |_| false).unwrap().unwrap();
        assert_eq!(to, folder.join("to/cat.jpg"));
        assert_eq!(fs::read(&to).unwrap(), b"cat");
        assert!(!from.exists());
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn never_replaces_a_file_in_the_new_folder() {
        let folder = temp_folder("taken");
        fs::write(folder.join("to/cat.jpg"), b"someone else's cat").unwrap();
        let from = folder.join("from/cat.jpg");
        fs::write(&from, b"my cat").unwrap();
        let to = move_file(&from, &folder.join("to"), |_| false).unwrap().unwrap();
        assert_eq!(to, folder.join("to/cat (2).jpg"));
        assert_eq!(fs::read(folder.join("to/cat.jpg")).unwrap(), b"someone else's cat", "untouched");
        assert_eq!(fs::read(&to).unwrap(), b"my cat");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn skips_names_it_is_told_to_avoid() {
        let folder = temp_folder("avoid");
        let from = folder.join("from/cat.png");
        fs::write(&from, b"png").unwrap();
        let taken = folder.join("to/cat.png");
        let to = move_file(&from, &folder.join("to"), |path| path == taken).unwrap().unwrap();
        assert_eq!(to, folder.join("to/cat (2).png"));
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn leaves_images_already_in_the_folder_and_refuses_other_files() {
        let folder = temp_folder("refuse");
        let here = folder.join("to/here.jpg");
        fs::write(&here, b"x").unwrap();
        assert_eq!(move_file(&here, &folder.join("to"), |_| false).unwrap(), None);
        assert!(here.exists());

        let text = folder.join("from/notes.txt");
        fs::write(&text, b"not an image").unwrap();
        assert!(move_file(&text, &folder.join("to"), |_| false).is_err());
        assert!(text.exists(), "only images are ever moved");
        assert!(move_file(&folder.join("from/gone.jpg"), &folder.join("to"), |_| false).is_err());
        assert_eq!(fs::read_dir(folder.join("to")).unwrap().count(), 1, "no empty file left behind");
        fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn copying_keeps_the_contents_and_date() {
        // The fallback for moves to another drive, tried directly (tests can't count on two drives).
        let folder = temp_folder("copy");
        let from = folder.join("from/big.jpg");
        fs::write(&from, vec![7u8; 100_000]).unwrap();
        let date = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        File::options().write(true).open(&from).unwrap().set_modified(date).unwrap();
        let to = folder.join("to/big.jpg");
        File::create(&to).unwrap();
        copy_then_remove(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(fs::read(&to).unwrap(), vec![7u8; 100_000]);
        assert_eq!(fs::metadata(&to).unwrap().modified().unwrap(), date);
        fs::remove_dir_all(&folder).unwrap();
    }
}
