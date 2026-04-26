use std::collections::HashMap;
use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{FetchOptions, RemoteCallbacks, Repository};
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use diff_core::{
    BackendError, CloneProgress, FileContentsBatchItem, FileContentsRequest, GitBackend,
    LocalGitBackend, RepoStatus, WatcherHandle,
};

fn perf_event(app: &AppHandle, op: &str, extra: serde_json::Value) {
    eprintln!("[diff-perf] rust:{op} {extra}");
    let mut payload = match extra {
        serde_json::Value::Object(m) => m,
        _ => serde_json::Map::new(),
    };
    payload.insert("op".to_string(), serde_json::Value::String(op.to_string()));
    let _ = app.emit("perf:log", serde_json::Value::Object(payload));
}

fn ms_since(start: Instant) -> f64 {
    let d = start.elapsed();
    (d.as_secs_f64() * 1000.0 * 100.0).round() / 100.0
}

pub struct AppState {
    pub backend: Mutex<Option<Box<dyn GitBackend>>>,
    pub bridge: Mutex<Option<Child>>,
    pub event_listener: Mutex<Option<JoinHandle<()>>>,
    pub event_listener_stop: Arc<AtomicBool>,
    pub clone_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub watcher_handle: Mutex<Option<WatcherHandle>>,
    pub watcher_generation: AtomicU64,
}

fn err_to_string(e: BackendError) -> String {
    e.to_string()
}

fn install_backend(
    app: &AppHandle,
    state: &AppState,
    backend: Box<dyn GitBackend>,
    workdir: String,
) -> Result<(), String> {
    let _ = state.watcher_generation.fetch_add(1, Ordering::SeqCst);

    let app_clone = app.clone();
    let handle = backend
        .subscribe_changes(Box::new(move || {
            let _ = app_clone.emit("repo:changed", ());
        }))
        .map_err(err_to_string)?;

    *state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))? = Some(backend);
    *state.watcher_handle.lock().map_err(|e| format!("lock poisoned: {e}"))? = Some(handle);

    perf_event(app, "open_repo:installed", json!({ "workdir": workdir }));
    Ok(())
}

#[tauri::command]
pub fn open_repo(path: String, app: AppHandle, state: State<AppState>) -> Result<String, String> {
    let total_start = Instant::now();
    perf_event(&app, "open_repo:start", json!({ "path": &path }));

    let backend = LocalGitBackend::open(Path::new(&path)).map_err(err_to_string)?;
    let workdir = backend.workdir().to_string_lossy().to_string();
    install_backend(&app, &state, Box::new(backend), workdir.clone())?;

    perf_event(
        &app,
        "open_repo",
        json!({ "path": &path, "workdir": &workdir, "totalMs": ms_since(total_start) }),
    );
    Ok(workdir)
}

#[tauri::command]
pub fn get_repo_status(state: State<AppState>) -> Result<RepoStatus, String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.get_status().map_err(err_to_string)
}

#[tauri::command]
pub fn get_file_contents_batch(
    requests: Vec<FileContentsRequest>,
    state: State<AppState>,
) -> Result<Vec<FileContentsBatchItem>, String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.get_file_contents_batch(requests).map_err(err_to_string)
}

#[tauri::command]
pub fn stage_file(path: String, state: State<AppState>) -> Result<(), String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.stage_file(&path).map_err(err_to_string)
}

#[tauri::command]
pub fn stage_all(state: State<AppState>) -> Result<(), String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.stage_all().map_err(err_to_string)
}

#[tauri::command]
pub fn unstage_file(path: String, state: State<AppState>) -> Result<(), String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.unstage_file(&path).map_err(err_to_string)
}

#[tauri::command]
pub fn unstage_all(state: State<AppState>) -> Result<(), String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.unstage_all().map_err(err_to_string)
}

#[tauri::command]
pub fn discard_file(path: String, state: State<AppState>) -> Result<(), String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.discard_file(&path).map_err(err_to_string)
}

#[tauri::command]
pub fn commit(
    message: String,
    amend: Option<bool>,
    state: State<AppState>,
) -> Result<String, String> {
    let lock = state.backend.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let backend = lock.as_ref().ok_or("no repository open")?;
    backend.commit(&message, amend.unwrap_or(false)).map_err(err_to_string)
}

#[tauri::command]
pub fn init_repo(path: String, app: AppHandle, state: State<AppState>) -> Result<String, String> {
    let backend = LocalGitBackend::init(Path::new(&path)).map_err(err_to_string)?;
    let workdir = backend.workdir().to_string_lossy().to_string();
    install_backend(&app, &state, Box::new(backend), workdir.clone())?;
    Ok(workdir)
}

#[tauri::command]
pub fn get_repo_branch(path: String) -> Result<Option<String>, String> {
    let backend = LocalGitBackend::open(Path::new(&path)).map_err(err_to_string)?;
    backend.get_branch().map_err(err_to_string)
}

#[tauri::command]
pub fn clone_repo(
    url: String,
    dest: String,
    id: String,
    app: AppHandle,
    state: State<AppState>,
) -> Result<String, String> {
    let cancel = Arc::new(AtomicBool::new(false));
    state
        .clone_cancels
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .insert(id.clone(), cancel.clone());

    let result = (|| -> Result<Repository, String> {
        let mut cb = RemoteCallbacks::new();
        let a1 = app.clone();
        let i1 = id.clone();
        let c1 = cancel.clone();
        cb.transfer_progress(move |p| {
            if c1.load(Ordering::SeqCst) {
                return false;
            }
            let _ = a1.emit(
                "clone:progress",
                CloneProgress {
                    id: i1.clone(),
                    phase: "fetch".to_string(),
                    received_objects: p.received_objects(),
                    total_objects: p.total_objects(),
                    indexed_objects: p.indexed_objects(),
                    received_bytes: p.received_bytes(),
                    checkout_current: 0,
                    checkout_total: 0,
                },
            );
            true
        });
        let mut fo = FetchOptions::new();
        fo.remote_callbacks(cb);

        let mut co = CheckoutBuilder::new();
        let a2 = app.clone();
        let i2 = id.clone();
        let c2 = cancel.clone();
        co.progress(move |_, cur, tot| {
            if c2.load(Ordering::SeqCst) {
                return;
            }
            let _ = a2.emit(
                "clone:progress",
                CloneProgress {
                    id: i2.clone(),
                    phase: "checkout".to_string(),
                    received_objects: 0,
                    total_objects: 0,
                    indexed_objects: 0,
                    received_bytes: 0,
                    checkout_current: cur,
                    checkout_total: tot,
                },
            );
        });

        RepoBuilder::new()
            .fetch_options(fo)
            .with_checkout(co)
            .clone(&url, Path::new(&dest))
            .map_err(|e| {
                if cancel.load(Ordering::SeqCst) {
                    "clone cancelled".to_string()
                } else {
                    format!("clone failed: {e}")
                }
            })
    })();

    if let Ok(mut guard) = state.clone_cancels.lock() {
        guard.remove(&id);
    }

    let _repo = result?;
    // Re-open via LocalGitBackend now that the clone finished.
    let backend = LocalGitBackend::open(Path::new(&dest)).map_err(err_to_string)?;
    let workdir = backend.workdir().to_string_lossy().to_string();
    install_backend(&app, &state, Box::new(backend), workdir.clone())?;
    Ok(workdir)
}

#[tauri::command]
pub fn cancel_clone(id: String, state: State<AppState>) -> Result<(), String> {
    if let Some(flag) = state
        .clone_cancels
        .lock()
        .map_err(|e| format!("lock poisoned: {e}"))?
        .get(&id)
    {
        flag.store(true, Ordering::SeqCst);
    }
    Ok(())
}

#[tauri::command]
pub fn cleanup_path(path: String) -> Result<(), String> {
    let p = Path::new(&path);
    if p.exists() {
        std::fs::remove_dir_all(p).map_err(|e| format!("cleanup failed: {e}"))?;
    }
    Ok(())
}
