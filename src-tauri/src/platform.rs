//! Differences between Windows, macOS and Linux that the Rust side needs to know about.
//! Keep platform checks here so the rest of the code doesn't need to care.

/// Whether file names that differ only in capital letters are the same file on this system:
/// yes on Windows and (by default) macOS, no on Linux.
pub const PATHS_IGNORE_CASE: bool = !cfg!(target_os = "linux");

/// A version of a path that is equal for two paths pointing at the same file.
pub fn path_key(path: &str) -> String {
    if PATHS_IGNORE_CASE { path.to_lowercase() } else { path.to_owned() }
}
