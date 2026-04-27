use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, WebviewUrl, WebviewWindowBuilder};

static NEXT_REPO_LABEL: AtomicU64 = AtomicU64::new(1);

/// Open a new repo window. The frontend reads `host`/`path` from the URL query
/// string and decides whether to call `open_repo` (local) or `open_remote_repo`.
pub fn open_repo_window(app: &AppHandle, host: &str, path: &str) -> Result<(), String> {
    let n = NEXT_REPO_LABEL.fetch_add(1, Ordering::SeqCst);
    let label = format!("repo-{n}");

    let basename = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(path);
    let title = if host == "local" {
        basename.to_string()
    } else {
        format!("{host}:{basename}")
    };

    let url = format!(
        "index.html?kind=repo&host={}&path={}",
        urlencoding::encode(host),
        urlencoding::encode(path)
    );

    WebviewWindowBuilder::new(app, &label, WebviewUrl::App(url.into()))
        .title(title)
        .inner_size(1200.0, 800.0)
        .min_inner_size(900.0, 600.0)
        .build()
        .map_err(|e| format!("create window: {e}"))?;

    Ok(())
}
