//! Watching the open workspace's folder, so images that are added, removed or renamed (in
//! Explorer, Finder, a browser's downloads, …) show up without pressing Refresh.
//!
//! The system tells the app about changes (ReadDirectoryChangesW on Windows, FSEvents on macOS,
//! inotify on Linux), so watching costs nothing while the folder stays the same. Changes come in
//! bursts (copying 50 images is hundreds of events), so the interface is told once things have
//! been quiet for a moment, and then rescans the folder. Only one folder is watched at a time.

use crate::scan;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

/// The event that tells the interface a workspace's folder changed (the payload is its id).
/// Must match `FOLDER_CHANGED_EVENT` in api.ts.
pub const FOLDER_CHANGED_EVENT: &str = "folder-changed";
/// How long the folder must be quiet before the interface is told.
const QUIET: Duration = Duration::from_millis(400);
/// During a long burst (a big copy), tell the interface at least this often, so it shows progress.
const MAX_WAIT: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct FolderWatch(Mutex<Option<Watching>>);

struct Watching {
    workspace_id: i64,
    folder: PathBuf,
    recursive: bool,
    /// Watching stops when this is dropped.
    _watcher: RecommendedWatcher,
}

impl FolderWatch {
    /// Watches a workspace's folder (and stops watching the one before). Nothing changes if it's
    /// already being watched the same way. If the system can't watch it (some network drives),
    /// the Refresh button still works.
    pub fn watch(&self, app: &AppHandle, workspace_id: i64, folder: &Path, recursive: bool) {
        let mut current = self.0.lock().unwrap();
        if current.as_ref().is_some_and(|w| w.workspace_id == workspace_id && w.folder == folder && w.recursive == recursive)
        {
            return;
        }
        *current = None;
        match start(app.clone(), workspace_id, folder, recursive) {
            Ok(watcher) => {
                *current = Some(Watching { workspace_id, folder: folder.to_path_buf(), recursive, _watcher: watcher })
            }
            Err(e) => eprintln!("Can't watch {} for changes: {e}", folder.display()),
        }
    }
}

fn start(app: AppHandle, workspace_id: i64, folder: &Path, recursive: bool) -> notify::Result<RecommendedWatcher> {
    let (changed, changes) = mpsc::channel::<()>();
    let watched = folder.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if event.is_ok_and(|event| matters(&event, &watched)) {
            let _ = changed.send(());
        }
    })?;
    watcher.watch(folder, if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive })?;

    // Waits for changes and tells the interface. Ends when the watcher is dropped (that drops
    // `changed`, its way of sending here).
    std::thread::spawn(move || {
        while changes.recv().is_ok() {
            let first = Instant::now();
            loop {
                let left = MAX_WAIT.saturating_sub(first.elapsed());
                if left.is_zero() {
                    break;
                }
                match changes.recv_timeout(QUIET.min(left)) {
                    Ok(()) => continue, // still busy
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
            let _ = app.emit(FOLDER_CHANGED_EVENT, workspace_id);
        }
    });
    Ok(watcher)
}

/// Whether a change could affect which images the workspace shows: an image file, or a folder
/// (no extension; renaming or deleting one changes what's inside it). Hidden files and folders
/// are skipped, like the scan does, and so is just reading a file.
fn matters(event: &notify::Event, folder: &Path) -> bool {
    if matches!(event.kind, EventKind::Access(_)) {
        return false;
    }
    // The system lost track (too many changes at once): rescan to be sure.
    if event.need_rescan() || event.paths.is_empty() {
        return true;
    }
    event.paths.iter().any(|path| {
        let inside = path.strip_prefix(folder).unwrap_or(path);
        let hidden = inside.components().any(|part| {
            part.as_os_str().to_str().is_some_and(|name| name.starts_with('.') || name.starts_with('$'))
        });
        !hidden && (scan::is_image(path) || path.extension().is_none())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};

    fn event(kind: EventKind, path: &Path) -> notify::Event {
        notify::Event::new(kind).add_path(path.to_path_buf())
    }

    #[test]
    fn only_changes_to_images_and_folders_matter() {
        let folder = std::env::temp_dir().join("art");
        let image = folder.join("sub").join("cat.JPG");
        assert!(matters(&event(EventKind::Create(CreateKind::File), &image), &folder));
        assert!(matters(&event(EventKind::Remove(RemoveKind::Any), &image), &folder));
        assert!(matters(&event(EventKind::Modify(ModifyKind::Any), &folder.join("sub")), &folder), "a folder");
        assert!(!matters(&event(EventKind::Access(AccessKind::Any), &image), &folder), "only read");
        assert!(!matters(&event(EventKind::Create(CreateKind::File), &folder.join("notes.txt")), &folder));
        assert!(!matters(&event(EventKind::Create(CreateKind::File), &folder.join("cat.jpg.crdownload")), &folder));
        assert!(!matters(&event(EventKind::Create(CreateKind::File), &folder.join(".git").join("HEAD")), &folder));
        assert!(!matters(&event(EventKind::Create(CreateKind::File), &folder.join("._cat.jpg")), &folder));
        // A workspace inside a hidden folder still works: only what's inside the workspace counts.
        let hidden = std::env::temp_dir().join(".art");
        assert!(matters(&event(EventKind::Create(CreateKind::File), &hidden.join("cat.png")), &hidden));
    }
}
