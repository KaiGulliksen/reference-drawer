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
