mod git;
pub mod remote_backend;
mod review_bridge;
pub mod ssh_config;
pub mod storage;
pub mod window;

use git::AppState;
use storage::{RecentRepo, Storage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::Manager;

pub use review_bridge::sidecar_script_path;

#[derive(Clone)]
pub enum LaunchSpec {
    Local(PathBuf),
    Remote { host: String, path: String },
}

static LAUNCH_SPEC: OnceLock<LaunchSpec> = OnceLock::new();

/// Record the initial repo to open, supplied by `diff [path]` or
/// `diff [host:path]` on the command line. Called before Tauri is built.
pub fn set_launch_spec(spec: LaunchSpec) {
    let _ = LAUNCH_SPEC.set(spec);
}


#[tauri::command]
fn list_recent_repos(host: Option<String>, storage: tauri::State<Storage>) -> Result<Vec<RecentRepo>, String> {
    match host {
        Some(h) => storage.list_for_host(&h),
        None => storage.list_all_recent(50),
    }
}

#[tauri::command]
fn record_recent_repo(host: String, path: String, storage: tauri::State<Storage>) -> Result<(), String> {
    storage.record_open(&host, &path)
}

#[tauri::command]
fn list_ssh_hosts() -> Result<Vec<ssh_config::SshHost>, String> {
    ssh_config::list_hosts()
}

#[tauri::command]
fn open_picker_repo(
    host: String,
    path: String,
    app: tauri::AppHandle,
    storage: tauri::State<Storage>,
) -> Result<(), String> {
    storage.record_open(&host, &path)?;
    window::open_repo_window(&app, &host, &path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let stop_flag = Arc::new(AtomicBool::new(false));

    let storage = Storage::open_default().expect("failed to open state.db");

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(storage)
        .manage(AppState {
            backends: Mutex::new(HashMap::new()),
            bridge: Mutex::new(None),
            event_listener: Mutex::new(None),
            event_listener_stop: stop_flag.clone(),
            clone_cancels: Mutex::new(HashMap::new()),
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

            // If invoked with a path/host:path arg, open that repo window
            // alongside the picker.
            if let Some(spec) = LAUNCH_SPEC.get() {
                let handle = app.handle().clone();
                match spec {
                    LaunchSpec::Local(path) => {
                        let _ = window::open_repo_window(
                            &handle,
                            "local",
                            &path.to_string_lossy(),
                        );
                    }
                    LaunchSpec::Remote { host, path } => {
                        let _ = window::open_repo_window(&handle, host, path);
                    }
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
            git::open_remote_repo,
            review_bridge::submit_review,
            list_recent_repos,
            record_recent_repo,
            list_ssh_hosts,
            open_picker_repo,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    let stop = stop_flag;
    app.run(move |app_handle, event| match event {
        tauri::RunEvent::Exit => {
            let state: &AppState = app_handle.state::<AppState>().inner();

            state.event_listener_stop.store(true, Ordering::Relaxed);
            if let Ok(mut guard) = state.event_listener.lock() {
                if let Some(handle) = guard.take() {
                    let _ = handle.join();
                }
            }

            if let Ok(mut guard) = state.bridge.lock() {
                if let Some(ref mut child) = *guard {
                    let _ = child.kill();
                }
            }

            if let Ok(mut map) = state.backends.lock() {
                map.clear();
            }

            stop.store(true, Ordering::Relaxed);
        }
        tauri::RunEvent::WindowEvent {
            label,
            event: tauri::WindowEvent::Destroyed,
            ..
        } => {
            let state: &AppState = app_handle.state::<AppState>().inner();
            if let Ok(mut map) = state.backends.lock() {
                map.remove(&label);
            }
            if label == "picker" {
                app_handle.exit(0);
            }
        }
        _ => {}
    });
}
