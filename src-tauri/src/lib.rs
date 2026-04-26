mod git;
mod review_bridge;

use git::AppState;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;

pub use review_bridge::sidecar_script_path;

static LAUNCH_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Record an initial repository path supplied by `diff [path]` on the command
/// line. Called before Tauri is built so the frontend can pick it up on mount.
pub fn set_launch_path(path: PathBuf) {
    let _ = LAUNCH_PATH.set(path);
}

#[tauri::command]
fn get_launch_path() -> Option<String> {
    LAUNCH_PATH.get().map(|p| p.to_string_lossy().to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            backend: Mutex::new(None),
            bridge: Mutex::new(None),
            event_listener: Mutex::new(None),
            event_listener_stop: stop_flag.clone(),
            clone_cancels: Mutex::new(HashMap::new()),
            watcher_handle: Mutex::new(None),
            watcher_generation: AtomicU64::new(0),
        })
        .setup(|app| {
            let state = app.state::<AppState>();
            match review_bridge::start_review_server(state.inner()) {
                Ok(port) => {
                    let handle = review_bridge::start_event_listener(
                        app.handle().clone(),
                        port,
                        state.event_listener_stop.clone(),
                    );
                    if let Ok(mut guard) = state.event_listener.lock() {
                        *guard = Some(handle);
                    }
                }
                Err(e) => {
                    eprintln!("failed to start review server on launch: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            git::open_repo,
            git::get_repo_status,
            git::get_file_contents_batch,
            git::stage_file,
            git::unstage_file,
            git::stage_all,
            git::commit,
            git::unstage_all,
            git::clone_repo,
            git::cancel_clone,
            git::cleanup_path,
            git::init_repo,
            git::get_repo_branch,
            git::discard_file,
            review_bridge::submit_review,
            get_launch_path,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let stop = stop_flag;
    app.run(move |app_handle, event| {
        if let tauri::RunEvent::Exit = event {
            let state: &AppState = app_handle.state::<AppState>().inner();

            // Signal the SSE listener thread to stop
            state.event_listener_stop.store(true, Ordering::Relaxed);
            if let Ok(mut guard) = state.event_listener.lock() {
                if let Some(handle) = guard.take() {
                    // Give the thread a moment to notice the stop flag
                    let _ = handle.join();
                }
            }

            // Kill the bridge server process
            if let Ok(mut guard) = state.bridge.lock() {
                if let Some(ref mut child) = *guard {
                    let _ = child.kill();
                }
            }

            // Drop the file watcher so the background thread exits.
            if let Ok(mut guard) = state.watcher_handle.lock() {
                *guard = None;
            }

            // Ensure the stop flag is set (redundant but defensive)
            stop.store(true, Ordering::Relaxed);
        }
    });
}
