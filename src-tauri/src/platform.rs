//! Differences between Windows, macOS and Linux that the Rust side needs to know about.
//! Keep platform checks here so the rest of the code doesn't need to care.

/// Whether file names that differ only in capital letters are the same file on this system:
/// yes on Windows and (by default) macOS, no on Linux.
pub const PATHS_IGNORE_CASE: bool = !cfg!(target_os = "linux");

/// A version of a path that is equal for two paths pointing at the same file.
pub fn path_key(path: &str) -> String {
    if PATHS_IGNORE_CASE { path.to_lowercase() } else { path.to_owned() }
}

/// The message the interface sends (with the dropped folders attached) to ask where they are.
/// Must match `DROPPED_PATHS_MESSAGE` in platform.ts.
pub const DROPPED_PATHS_MESSAGE: &str = "reference-library:dropped-paths";
/// The event that brings the answer back to the interface: a list of full paths.
pub const DROPPED_PATHS_EVENT: &str = "dropped-paths";

/// Windows only: lets the interface find out where dropped folders are, so dropping a folder
/// onto the window can add it as a workspace.
///
/// The app handles drops in the web view itself (that's what lets it receive images dragged
/// from a browser), and web pages aren't told where dropped files are. WebView2 can tell the
/// app, though: the interface posts `DROPPED_PATHS_MESSAGE` with the dropped items attached,
/// and here they're turned back into paths. On macOS and Linux the web view has no such
/// feature, so folders are added with the + button there.
#[cfg(windows)]
pub fn watch_dropped_paths(window: &tauri::WebviewWindow) -> tauri::Result<()> {
    use tauri::{Emitter, Manager};
    use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2};
    use webview2_com::{WebMessageReceivedEventHandler, take_pwstr};
    use windows_core::{Interface, PWSTR};

    let app = window.app_handle().clone();
    window.with_webview(move |webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            // Tauri's own messages arrive here too; only answer ours. (Tauri also sees ours, can't
            // read it, and logs a harmless "JSON error" in the web view's console.)
            let mut message = PWSTR::null();
            if args.TryGetWebMessageAsString(&mut message).is_err() || take_pwstr(message) != DROPPED_PATHS_MESSAGE {
                return Ok(());
            }
            let objects = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>()?.AdditionalObjects()?;
            let mut count = 0;
            objects.Count(&mut count)?;
            let mut paths = Vec::new();
            for i in 0..count {
                if let Ok(file) = objects.GetValueAtIndex(i)?.cast::<ICoreWebView2File>() {
                    let mut path = PWSTR::null();
                    file.Path(&mut path)?;
                    paths.push(take_pwstr(path));
                }
            }
            let _ = app.emit(DROPPED_PATHS_EVENT, paths);
            Ok(())
        }));
        let mut token = 0;
        if let Err(e) = core.add_WebMessageReceived(&handler, &mut token) {
            eprintln!("Couldn't listen for dropped folders: {e}");
        }
    })
}

#[cfg(not(windows))]
pub fn watch_dropped_paths(_window: &tauri::WebviewWindow) -> tauri::Result<()> {
    Ok(())
}

/// Windows only: while the window is minimised, asks the web view to let go of the memory it
/// isn't using, and to take it back when the window returns.
///
/// Nearly all of the app's memory is the web view's (decoded thumbnails, the graphics cache), and
/// a reference app spends a lot of its time minimised next to a drawing program. WebView2 has a
/// setting made for exactly this ("memory usage target level"); the cost is a brief moment of
/// catching up when the window comes back. Only when minimised, never just because another
/// window has the focus: the app is often looked at while working in something else.
/// WebKit (macOS and Linux) has no such setting, and manages this by itself.
#[cfg(windows)]
pub fn save_memory_while_minimized(window: &tauri::WebviewWindow) {
    use std::sync::atomic::{AtomicBool, Ordering};
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL, ICoreWebView2_19,
    };
    use windows_core::Interface;

    let target = window.clone();
    let was_minimized = AtomicBool::new(false);
    window.on_window_event(move |event| {
        // Minimising and restoring both arrive as a change of size.
        if !matches!(event, tauri::WindowEvent::Resized(_)) {
            return;
        }
        let minimized = target.is_minimized().unwrap_or(false);
        if was_minimized.swap(minimized, Ordering::Relaxed) == minimized {
            return; // an ordinary resize
        }
        let _ = target.with_webview(move |webview| unsafe {
            let level = if minimized {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
            } else {
                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
            };
            // Older WebView2 versions don't have the setting; then nothing changes.
            if let Ok(core) = webview.controller().CoreWebView2().and_then(|core| core.cast::<ICoreWebView2_19>()) {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
        });
    });
}

#[cfg(not(windows))]
pub fn save_memory_while_minimized(_window: &tauri::WebviewWindow) {}
