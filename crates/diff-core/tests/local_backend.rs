use std::process::Command;

use diff_core::{GitBackend, LocalGitBackend};
use tempfile::TempDir;

fn init_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();

    Command::new("git").args(["init"]).current_dir(path).status().unwrap();
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .status()
        .unwrap();
    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(path)
        .status()
        .unwrap();

    dir
}

#[test]
fn empty_repo_has_empty_status() {
    let dir = init_test_repo();
    let backend = LocalGitBackend::open(dir.path()).unwrap();
    let status = backend.get_status().unwrap();
    assert!(status.staged.is_empty());
    assert!(status.unstaged.is_empty());
    assert!(status.untracked.is_empty());
}

#[test]
fn untracked_file_appears_in_status() {
    let dir = init_test_repo();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    let backend = LocalGitBackend::open(dir.path()).unwrap();
    let status = backend.get_status().unwrap();
    assert_eq!(status.untracked, vec!["hello.txt".to_string()]);
}

#[test]
fn stage_file_moves_from_untracked_to_staged() {
    let dir = init_test_repo();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    let backend = LocalGitBackend::open(dir.path()).unwrap();

    backend.stage_file("hello.txt").unwrap();
    let status = backend.get_status().unwrap();
    assert!(status.untracked.is_empty());
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, "hello.txt");
}

#[test]
fn commit_returns_oid_and_clears_status() {
    let dir = init_test_repo();
    std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
    let backend = LocalGitBackend::open(dir.path()).unwrap();
    backend.stage_file("a.txt").unwrap();
    let oid = backend.commit("first commit", false).unwrap();
    assert_eq!(oid.len(), 40); // SHA-1 hex string
    let status = backend.get_status().unwrap();
    assert!(status.staged.is_empty());
}
