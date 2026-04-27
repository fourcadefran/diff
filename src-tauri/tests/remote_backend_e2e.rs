use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use diff_core::GitBackend;
use diff_lib::remote_backend::RemoteGitBackend;
use tempfile::TempDir;

fn agent_binary_path() -> PathBuf {
    // The integration tests in src-tauri can't use CARGO_BIN_EXE_diff-agent
    // (different crate). Locate the binary via env override or default path.
    if let Ok(p) = std::env::var("DIFF_AGENT_BIN") {
        return PathBuf::from(p);
    }
    // Default: target/debug relative to the workspace root.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest.parent().unwrap();
    workspace_root.join("target").join("debug").join("diff-agent")
}

fn init_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    Command::new("git").args(["init", "-q"]).current_dir(path).status().unwrap();
    Command::new("git")
        .args(["config", "user.email", "t@t.com"])
        .current_dir(path)
        .status()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "T"])
        .current_dir(path)
        .status()
        .unwrap();
    dir
}

fn build_agent_once() {
    let _ = Command::new("cargo")
        .args(["build", "-p", "diff-agent"])
        .status()
        .expect("cargo build");
}

#[test]
fn remote_backend_smoke() {
    build_agent_once();
    let agent = agent_binary_path();
    assert!(agent.exists(), "agent binary not found at {}", agent.display());

    let dir = init_test_repo();
    let mut cmd = Command::new(&agent);
    cmd.args(["--stdio", "--repo", dir.path().to_str().unwrap()]);

    let backend = RemoteGitBackend::spawn(cmd).expect("spawn");

    let status = backend.get_status().expect("get_status");
    assert!(status.staged.is_empty());

    std::fs::write(dir.path().join("a.txt"), "hi").unwrap();

    // Watcher event arrival is timing-dependent; not verified here. Just
    // re-fetch status manually.
    let status = backend.get_status().expect("get_status 2");
    assert_eq!(status.untracked, vec!["a.txt".to_string()]);

    backend.stage_file("a.txt").expect("stage");
    let status = backend.get_status().expect("get_status 3");
    assert!(status.untracked.is_empty());
    assert_eq!(status.staged.len(), 1);

    let oid = backend.commit("init", false).expect("commit");
    assert_eq!(oid.len(), 40);
}

#[test]
fn remote_backend_pushes_watcher_notifications() {
    build_agent_once();
    let agent = agent_binary_path();
    let dir = init_test_repo();
    let mut cmd = Command::new(&agent);
    cmd.args(["--stdio", "--repo", dir.path().to_str().unwrap()]);
    let backend = RemoteGitBackend::spawn(cmd).expect("spawn");

    let counter = Arc::new(AtomicUsize::new(0));
    let counter_clone = Arc::clone(&counter);
    let _watcher = backend
        .subscribe_changes(Box::new(move || {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        }))
        .expect("subscribe_changes");

    // Trigger a filesystem change that the agent's watcher will detect.
    std::fs::write(dir.path().join("trigger.txt"), "x").unwrap();

    // Wait up to 2 seconds for the notification to round-trip.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while counter.load(Ordering::SeqCst) == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(counter.load(Ordering::SeqCst) >= 1, "no watcher notification received");
}
