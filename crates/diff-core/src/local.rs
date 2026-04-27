use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use git2::build::CheckoutBuilder;
use git2::{Index, Patch, Repository, Status, StatusOptions, Tree};

use crate::types::{
    BackendError, ChangeKind, FileContentsBatchItem, FileContentsRequest, FileContentsResponse,
    FileEntry, RepoStatus, WatcherHandle,
};
use crate::watcher;
use crate::GitBackend;

/// Wraps a `git2::Repository` behind the `GitBackend` trait. Used directly by
/// the GUI for local repos and by `diff-agent` to serve remote requests.
pub struct LocalGitBackend {
    repo: Mutex<Repository>,
    workdir: PathBuf,
}

impl LocalGitBackend {
    /// Open a repository at `path` (discovers .git from inside subdirs).
    pub fn open(path: &Path) -> Result<Self, BackendError> {
        let repo = Repository::discover(path).map_err(BackendError::from)?;
        let workdir = repo.workdir().ok_or(BackendError::BareRepo)?.to_path_buf();
        Ok(Self {
            repo: Mutex::new(repo),
            workdir,
        })
    }

    /// Initialize a new repo at `path`. Creates the directory if missing.
    pub fn init(path: &Path) -> Result<Self, BackendError> {
        std::fs::create_dir_all(path)?;
        let repo = Repository::init(path).map_err(BackendError::from)?;
        let workdir = repo.workdir().ok_or(BackendError::BareRepo)?.to_path_buf();
        Ok(Self {
            repo: Mutex::new(repo),
            workdir,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

// ---------------------------------------------------------------------------
// GitBackend impl
// ---------------------------------------------------------------------------

impl GitBackend for LocalGitBackend {
    fn get_status(&self) -> Result<RepoStatus, BackendError> {
        let workdir_path = self.workdir.clone();

        let (status_entries, staged_counts, unstaged_counts) = {
            let repo = self
                .repo
                .lock()
                .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;

            let mut opts = StatusOptions::new();
            opts.include_untracked(true).recurse_untracked_dirs(true);

            let statuses = repo.statuses(Some(&mut opts)).map_err(BackendError::from)?;
            let status_entries: Vec<(String, Status)> = statuses
                .iter()
                .filter_map(|entry| entry.path().map(|p| (p.to_string(), entry.status())))
                .collect();

            let head_tree = repo
                .revparse_single("HEAD^{tree}")
                .ok()
                .and_then(|obj| obj.into_tree().ok());
            let head_tree_oid = head_tree.as_ref().map(|t| t.id());

            let staged_diff = repo
                .diff_tree_to_index(head_tree.as_ref(), None, None)
                .map_err(BackendError::from)?;
            let staged_delta_count = staged_diff.deltas().count();

            let unstaged_diff = repo
                .diff_index_to_workdir(None, None)
                .map_err(BackendError::from)?;
            let unstaged_delta_count = unstaged_diff.deltas().count();

            let staged_counts = count_diff_lines_parallel(
                &workdir_path,
                CountDiffKind::StagedAgainstHead(head_tree_oid),
                staged_delta_count,
            );
            let unstaged_counts = count_diff_lines_parallel(
                &workdir_path,
                CountDiffKind::Unstaged,
                unstaged_delta_count,
            );

            (status_entries, staged_counts, unstaged_counts)
        };

        let mut staged = Vec::new();
        let mut unstaged = Vec::new();
        let mut untracked = Vec::new();

        for (path, s) in status_entries {
            if s.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            ) {
                let kind = if s.contains(Status::INDEX_NEW) {
                    ChangeKind::Added
                } else if s.contains(Status::INDEX_MODIFIED) {
                    ChangeKind::Modified
                } else if s.contains(Status::INDEX_DELETED) {
                    ChangeKind::Deleted
                } else if s.contains(Status::INDEX_RENAMED) {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Typechange
                };
                let (additions, deletions) = staged_counts.get(&path).copied().unwrap_or((0, 0));
                staged.push(FileEntry {
                    path: path.clone(),
                    kind,
                    additions,
                    deletions,
                });
            }

            if s.intersects(
                Status::WT_MODIFIED
                    | Status::WT_DELETED
                    | Status::WT_RENAMED
                    | Status::WT_TYPECHANGE,
            ) {
                let kind = if s.contains(Status::WT_MODIFIED) {
                    ChangeKind::Modified
                } else if s.contains(Status::WT_DELETED) {
                    ChangeKind::Deleted
                } else if s.contains(Status::WT_RENAMED) {
                    ChangeKind::Renamed
                } else {
                    ChangeKind::Typechange
                };
                let (additions, deletions) = unstaged_counts.get(&path).copied().unwrap_or((0, 0));
                unstaged.push(FileEntry {
                    path: path.clone(),
                    kind,
                    additions,
                    deletions,
                });
            }

            if s.contains(Status::WT_NEW) {
                untracked.push(path);
            }
        }

        Ok(RepoStatus {
            staged,
            unstaged,
            untracked,
        })
    }

    fn get_branch(&self) -> Result<Option<String>, BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        let head = match repo.head() {
            Ok(h) => h,
            Err(_) => return Ok(None),
        };
        Ok(head.shorthand().map(|s| s.to_string()))
    }

    fn get_file_contents_batch(
        &self,
        requests: Vec<FileContentsRequest>,
    ) -> Result<Vec<FileContentsBatchItem>, BackendError> {
        let workdir_path = self.workdir.clone();

        let head_tree_oid = {
            let repo = self
                .repo
                .lock()
                .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
            read_head_tree(&repo).map(|t| t.id())
        };

        let workdir: &Path = &workdir_path;
        let requested_count = requests.len();
        if requested_count == 0 {
            return Ok(Vec::new());
        }

        let num_threads = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(4)
            .min(requested_count.max(1));
        let chunk_size = requested_count.div_ceil(num_threads.max(1));

        let responses: Vec<FileContentsBatchItem> = std::thread::scope(|s| {
            let handles: Vec<_> = requests
                .chunks(chunk_size)
                .map(|chunk| {
                    s.spawn(move || -> Vec<FileContentsBatchItem> {
                        let repo = match Repository::open(workdir) {
                            Ok(r) => r,
                            Err(e) => {
                                return chunk
                                    .iter()
                                    .map(|r| FileContentsBatchItem {
                                        path: r.path.clone(),
                                        response: None,
                                        error: Some(format!("worker repo open failed: {e}")),
                                    })
                                    .collect();
                            }
                        };
                        let head_tree = head_tree_oid.and_then(|oid| repo.find_tree(oid).ok());
                        let chunk_needs_index = chunk.iter().any(|r| r.staged);
                        let index = if chunk_needs_index {
                            repo.index().ok()
                        } else {
                            None
                        };
                        chunk
                            .iter()
                            .map(|req| {
                                process_file_request(
                                    &repo,
                                    head_tree.as_ref(),
                                    index.as_ref(),
                                    workdir,
                                    req,
                                )
                            })
                            .collect()
                    })
                })
                .collect();

            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        });

        Ok(responses)
    }

    fn stage_file(&self, path: &str) -> Result<(), BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        stage_path(&repo, path)
    }

    fn stage_all(&self) -> Result<(), BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        let paths = collect_status_paths(
            &repo,
            Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_RENAMED
                | Status::WT_TYPECHANGE,
            None,
        )?;
        let workdir = repo.workdir().ok_or(BackendError::BareRepo)?;
        let mut index = repo.index().map_err(BackendError::from)?;
        for (path, status) in paths {
            let repo_path = Path::new(&path);
            stage_index_path(&mut index, workdir, repo_path, Some(status))?;
        }
        index.write().map_err(BackendError::from)?;
        Ok(())
    }

    fn unstage_file(&self, path: &str) -> Result<(), BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        unstage_path(&repo, path)
    }

    fn unstage_all(&self) -> Result<(), BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        for path in collect_paths(
            &repo,
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_DELETED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        )? {
            unstage_path(&repo, &path)?;
        }
        Ok(())
    }

    fn discard_file(&self, path: &str) -> Result<(), BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        let workdir = repo.workdir().ok_or(BackendError::BareRepo)?;
        let relative_path = Path::new(path);
        validate_repo_relative_path(relative_path)?;
        let target = canonical_contained_target(workdir, relative_path)?;

        let status = repo
            .status_file(relative_path)
            .map_err(BackendError::from)?;
        let index_dirty = status.intersects(
            Status::INDEX_NEW
                | Status::INDEX_MODIFIED
                | Status::INDEX_DELETED
                | Status::INDEX_RENAMED
                | Status::INDEX_TYPECHANGE,
        );

        // Pure untracked file or directory: remove from disk, no git state to touch.
        if status.contains(Status::WT_NEW) && !index_dirty {
            remove_workdir_entry(&target)?;
            return Ok(());
        }

        // If index has uncommitted changes for this path, reset to HEAD first.
        if index_dirty {
            match repo.revparse_single("HEAD") {
                Ok(head_obj) => {
                    repo.reset_default(Some(&head_obj), [path])
                        .map_err(BackendError::from)?;
                }
                Err(_) => {
                    // No HEAD yet (initial commit): drop from index directly.
                    let mut index = repo.index().map_err(BackendError::from)?;
                    let _ = index.remove_path(relative_path);
                    index.write().map_err(BackendError::from)?;
                }
            }
        }

        // Restore workdir from HEAD for that path, if HEAD exists.
        if repo.revparse_single("HEAD").is_ok() {
            let mut cb = CheckoutBuilder::new();
            cb.force();
            cb.path(path);
            repo.checkout_head(Some(&mut cb))
                .map_err(BackendError::from)?;
        }

        // Staged-new with no HEAD counterpart: after reset the file is untracked. Remove it.
        if let Ok(post) = repo.status_file(relative_path) {
            let post_index_dirty = post.intersects(
                Status::INDEX_NEW
                    | Status::INDEX_MODIFIED
                    | Status::INDEX_DELETED
                    | Status::INDEX_RENAMED
                    | Status::INDEX_TYPECHANGE,
            );
            if post.contains(Status::WT_NEW) && !post_index_dirty {
                let _ = remove_workdir_entry(&target);
            }
        }

        Ok(())
    }

    fn commit(&self, message: &str, amend: bool) -> Result<String, BackendError> {
        let repo = self
            .repo
            .lock()
            .map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
        if message.trim().is_empty() {
            return Err(BackendError::Git(
                "commit message cannot be empty".to_string(),
            ));
        }
        let mut index = repo.index().map_err(BackendError::from)?;
        let tree_oid = index.write_tree().map_err(BackendError::from)?;

        if !amend {
            if let Ok(head_ref) = repo.head() {
                if let Ok(head_commit) = head_ref.peel_to_commit() {
                    if head_commit.tree_id() == tree_oid {
                        return Err(BackendError::Git(
                            "nothing to commit: index matches HEAD".to_string(),
                        ));
                    }
                }
            }
        }

        let tree = repo.find_tree(tree_oid).map_err(BackendError::from)?;
        let sig = repo.signature().map_err(BackendError::from)?;

        let oid = if amend {
            let head_ref = repo
                .head()
                .map_err(|_| BackendError::Git("cannot amend: no commit to amend".to_string()))?;
            let head_commit = head_ref.peel_to_commit().map_err(BackendError::from)?;
            head_commit
                .amend(
                    Some("HEAD"),
                    Some(&sig),
                    Some(&sig),
                    None,
                    Some(message),
                    Some(&tree),
                )
                .map_err(BackendError::from)?
        } else {
            match repo.head() {
                Ok(head_ref) => {
                    let parent = head_ref.peel_to_commit().map_err(BackendError::from)?;
                    repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
                        .map_err(BackendError::from)?
                }
                Err(_) => repo
                    .commit(Some("HEAD"), &sig, &sig, message, &tree, &[])
                    .map_err(BackendError::from)?,
            }
        };

        Ok(oid.to_string())
    }

    fn subscribe_changes(
        &self,
        sink: Box<dyn Fn() + Send + Sync>,
    ) -> Result<WatcherHandle, BackendError> {
        let watcher_inst =
            watcher::start(&self.workdir, move || sink()).map_err(BackendError::Io)?;
        Ok(WatcherHandle {
            _inner: Box::new(watcher_inst),
        })
    }
}

// ---------------------------------------------------------------------------
// Free helper: processes a single FileContentsRequest in a worker thread
// ---------------------------------------------------------------------------

fn process_file_request(
    repo: &Repository,
    head_tree: Option<&Tree<'_>>,
    index: Option<&Index>,
    workdir: &Path,
    req: &FileContentsRequest,
) -> FileContentsBatchItem {
    let rel = Path::new(&req.path);
    let name = rel
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| req.path.clone());

    let new_side = if req.staged {
        match index {
            Some(idx) => read_index_file(repo, idx, rel),
            None => Ok(FileSideContent::absent()),
        }
    } else {
        read_workdir_file(workdir, rel)
    };

    let old_side = read_tree_file(repo, head_tree, rel);

    match (old_side, new_side) {
        (Ok(old), Ok(new)) => FileContentsBatchItem {
            path: req.path.clone(),
            response: Some(FileContentsResponse {
                name,
                old_content: old.content,
                old_binary: old.is_binary,
                new_content: new.content,
                new_binary: new.is_binary,
            }),
            error: None,
        },
        (Err(e), _) | (_, Err(e)) => FileContentsBatchItem {
            path: req.path.clone(),
            response: None,
            error: Some(e.to_string()),
        },
    }
}

// ---------------------------------------------------------------------------
// Ported helper types and functions from src-tauri/src/git.rs
// ---------------------------------------------------------------------------

enum CountDiffKind {
    /// HEAD tree → index.
    StagedAgainstHead(Option<git2::Oid>),
    /// index → workdir.
    Unstaged,
}

fn build_diff_for_count<'a>(
    repo: &'a Repository,
    kind: &CountDiffKind,
) -> Result<git2::Diff<'a>, git2::Error> {
    match kind {
        CountDiffKind::StagedAgainstHead(tree_oid) => {
            let tree = tree_oid.and_then(|oid| repo.find_tree(oid).ok());
            repo.diff_tree_to_index(tree.as_ref(), None, None)
        }
        CountDiffKind::Unstaged => repo.diff_index_to_workdir(None, None),
    }
}

/// Compute per-file `(additions, deletions)` for a diff in parallel. Each
/// worker opens its own `Repository` and re-creates the diff (cheap: ~ms),
/// then computes `Patch::from_diff` on its slice of delta indexes. git2's
/// `Diff` is `!Send`/`!Sync`, so this is the only way to split xdiff work
/// across cores — which matters because xdiff for 600+ files is CPU-bound
/// and was pinning a single core at ~300ms.
fn count_diff_lines_parallel(
    workdir: &Path,
    kind: CountDiffKind,
    delta_count: usize,
) -> HashMap<String, (u32, u32)> {
    if delta_count == 0 {
        return HashMap::new();
    }
    let num_threads = std::thread::available_parallelism()
        .map(|n| n.get().min(8))
        .unwrap_or(4)
        .min(delta_count);
    let chunk_size = delta_count.div_ceil(num_threads);
    let kind_ref = &kind;

    std::thread::scope(|s| {
        let handles: Vec<_> = (0..delta_count)
            .step_by(chunk_size)
            .map(|start| {
                let end = (start + chunk_size).min(delta_count);
                s.spawn(move || -> HashMap<String, (u32, u32)> {
                    let Ok(repo) = Repository::open(workdir) else {
                        return HashMap::new();
                    };
                    let Ok(diff) = build_diff_for_count(&repo, kind_ref) else {
                        return HashMap::new();
                    };
                    let mut counts = HashMap::new();
                    for idx in start..end {
                        let Some(delta) = diff.get_delta(idx) else {
                            continue;
                        };
                        let Some(path) = delta
                            .new_file()
                            .path()
                            .or_else(|| delta.old_file().path())
                            .and_then(|p| p.to_str())
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        let Ok(Some(patch)) = Patch::from_diff(&diff, idx) else {
                            continue;
                        };
                        let Ok((_, adds, dels)) = patch.line_stats() else {
                            continue;
                        };
                        counts.insert(
                            path,
                            (
                                u32::try_from(adds).unwrap_or(u32::MAX),
                                u32::try_from(dels).unwrap_or(u32::MAX),
                            ),
                        );
                    }
                    counts
                })
            })
            .collect();

        let mut merged = HashMap::with_capacity(delta_count);
        for handle in handles {
            if let Ok(map) = handle.join() {
                merged.extend(map);
            }
        }
        merged
    })
}

struct FileSideContent {
    content: Option<String>,
    is_binary: bool,
}

impl FileSideContent {
    /// File is absent from this side (no tree entry, or file doesn't exist on disk).
    fn absent() -> Self {
        Self {
            content: None,
            is_binary: false,
        }
    }
}

fn decode_file_side(bytes: &[u8]) -> FileSideContent {
    match std::str::from_utf8(bytes) {
        Ok(text) => FileSideContent {
            content: Some(text.to_owned()),
            is_binary: false,
        },
        Err(_) => FileSideContent {
            content: None,
            is_binary: true,
        },
    }
}

fn read_head_tree(repo: &Repository) -> Option<Tree<'_>> {
    repo.revparse_single("HEAD^{tree}")
        .ok()
        .and_then(|obj| obj.into_tree().ok())
}

fn read_tree_file(
    repo: &Repository,
    tree: Option<&Tree<'_>>,
    path: &Path,
) -> Result<FileSideContent, BackendError> {
    let Some(tree) = tree else {
        return Ok(FileSideContent::absent());
    };

    let Ok(entry) = tree.get_path(path) else {
        return Ok(FileSideContent::absent());
    };

    let object = entry.to_object(repo).map_err(|e| {
        BackendError::Git(format!(
            "cannot read HEAD object for {}: {e}",
            path.display()
        ))
    })?;
    let blob = object.into_blob().map_err(|_| {
        BackendError::Git(format!("HEAD entry for {} is not a blob", path.display()))
    })?;

    Ok(decode_file_side(blob.content()))
}

fn validate_repo_relative_path(path: &Path) -> Result<(), BackendError> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(BackendError::InvalidPath(path.display().to_string())),
        }
    }
    Ok(())
}

fn read_workdir_file(workdir: &Path, path: &Path) -> Result<FileSideContent, BackendError> {
    // Reject anything other than normal/cur-dir components so we never escape
    // the workdir. Avoids the per-file `canonicalize()` (5+ stat syscalls per
    // path) that previously dominated `get_file_contents_batch:readMs`.
    validate_repo_relative_path(path)?;
    let abs = workdir.join(path);
    // Component filter blocks `..` but not symlinks; a tracked file like
    // `evil.txt -> /etc/passwd` would otherwise be followed by `fs::read`.
    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSideContent::absent());
        }
        Err(e) => {
            return Err(BackendError::Io(format!(
                "cannot stat {}: {e}",
                path.display()
            )));
        }
    };
    if meta.file_type().is_symlink() {
        return Err(BackendError::InvalidPath(format!(
            "symlink not permitted: {}",
            path.display()
        )));
    }
    match std::fs::read(&abs) {
        Ok(bytes) => Ok(decode_file_side(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(FileSideContent::absent()),
        Err(e) => Err(BackendError::Io(format!(
            "cannot read {}: {e}",
            path.display()
        ))),
    }
}

fn read_index_file(
    repo: &Repository,
    index: &Index,
    path: &Path,
) -> Result<FileSideContent, BackendError> {
    let entry = match index.get_path(path, 0) {
        Some(e) => e,
        None => return Ok(FileSideContent::absent()),
    };
    let blob = repo.find_blob(entry.id).map_err(|e| {
        BackendError::Git(format!(
            "cannot read index blob for {}: {e}",
            path.display()
        ))
    })?;
    Ok(decode_file_side(blob.content()))
}

fn stage_path(repo: &Repository, path: &str) -> Result<(), BackendError> {
    let input_path = Path::new(path);
    validate_repo_relative_path(input_path)?;
    let mut repo_path = PathBuf::new();
    for component in input_path.components() {
        if let Component::Normal(part) = component {
            repo_path.push(part);
        }
    }

    let is_directory_path = path.ends_with('/');
    let workdir = repo.workdir().ok_or(BackendError::BareRepo)?;
    let target = workdir.join(&repo_path);

    let mut index = repo.index().map_err(BackendError::from)?;

    if is_directory_path || target.is_dir() {
        let paths = collect_status_paths(
            repo,
            Status::WT_NEW
                | Status::WT_MODIFIED
                | Status::WT_DELETED
                | Status::WT_RENAMED
                | Status::WT_TYPECHANGE,
            (!repo_path.as_os_str().is_empty()).then_some(repo_path.as_path()),
        )?;

        for (path, status) in paths {
            let child_path = Path::new(&path);
            if child_path == repo_path || !child_path.starts_with(&repo_path) {
                continue;
            }

            stage_index_path(&mut index, workdir, child_path, Some(status))?;
        }

        index.write().map_err(BackendError::from)?;

        return Ok(());
    }

    stage_index_path(
        &mut index,
        workdir,
        &repo_path,
        repo.status_file(&repo_path).ok(),
    )?;

    index.write().map_err(BackendError::from)?;

    Ok(())
}

fn stage_index_path(
    index: &mut Index,
    workdir: &Path,
    repo_path: &Path,
    status: Option<Status>,
) -> Result<(), BackendError> {
    if workdir.join(repo_path).exists() {
        index.add_path(repo_path).map_err(BackendError::from)?;
        return Ok(());
    }

    let Some(status) = status else {
        return Ok(());
    };

    if !status.intersects(
        Status::WT_DELETED
            | Status::WT_RENAMED
            | Status::WT_TYPECHANGE
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    ) {
        return Ok(());
    }

    index.remove_path(repo_path).map_err(BackendError::from)?;
    Ok(())
}

fn unstage_path(repo: &Repository, path: &str) -> Result<(), BackendError> {
    let head_result = repo.revparse_single("HEAD");

    match head_result {
        Ok(head_obj) => {
            repo.reset_default(Some(&head_obj), [path])
                .map_err(BackendError::from)?;
        }
        Err(_) => {
            // No HEAD yet (initial commit) — remove from index directly
            let mut index = repo.index().map_err(BackendError::from)?;
            index
                .remove_path(Path::new(path))
                .map_err(BackendError::from)?;
            index.write().map_err(BackendError::from)?;
        }
    }

    Ok(())
}

fn collect_status_paths(
    repo: &Repository,
    flags: Status,
    pathspec: Option<&Path>,
) -> Result<Vec<(String, Status)>, BackendError> {
    let mut opts = StatusOptions::new();
    opts.include_untracked(true).recurse_untracked_dirs(true);
    if let Some(pathspec) = pathspec {
        opts.pathspec(pathspec).disable_pathspec_match(true);
    }

    let statuses = repo.statuses(Some(&mut opts)).map_err(BackendError::from)?;

    let mut paths = Vec::new();
    for entry in statuses.iter() {
        if entry.status().intersects(flags) {
            if let Some(path) = entry.path() {
                paths.push((path.to_string(), entry.status()));
            }
        }
    }

    paths.sort_by(|a, b| a.0.cmp(&b.0));
    paths.dedup_by(|a, b| a.0 == b.0);
    Ok(paths)
}

fn collect_paths(repo: &Repository, flags: Status) -> Result<Vec<String>, BackendError> {
    collect_status_paths(repo, flags, None)
        .map(|paths| paths.into_iter().map(|(path, _)| path).collect::<Vec<_>>())
}

// Single-stat removal: classify file-vs-dir-vs-missing atomically, then act.
// Avoids the TOCTOU `exists()` → `is_dir()` → `exists()` chain.
fn remove_workdir_entry(target: &Path) -> Result<(), BackendError> {
    let meta = match std::fs::symlink_metadata(target) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => {
            return Err(BackendError::Io(format!(
                "cannot stat {}: {e}",
                target.display()
            )));
        }
    };
    let ft = meta.file_type();
    if ft.is_dir() {
        std::fs::remove_dir_all(target).map_err(|e| BackendError::Io(format!("remove failed: {e}")))
    } else {
        std::fs::remove_file(target).map_err(|e| BackendError::Io(format!("remove failed: {e}")))
    }
}

fn canonical_contained_target(
    workdir: &Path,
    relative_path: &Path,
) -> Result<PathBuf, BackendError> {
    let canonical_workdir = workdir
        .canonicalize()
        .map_err(|e| BackendError::Io(format!("cannot canonicalize workdir: {e}")))?;
    let target = canonical_workdir.join(relative_path);
    let safe_target = if target.exists() {
        target
            .canonicalize()
            .map_err(|e| BackendError::Io(format!("cannot canonicalize target: {e}")))?
    } else {
        let parent = target
            .parent()
            .ok_or_else(|| BackendError::InvalidPath("invalid path".to_string()))?;
        let parent_canonical = parent
            .canonicalize()
            .map_err(|e| BackendError::Io(format!("cannot canonicalize parent: {e}")))?;
        parent_canonical.join(target.file_name().unwrap_or_default())
    };
    if !safe_target.starts_with(&canonical_workdir) {
        return Err(BackendError::InvalidPath(
            "path traversal detected".to_string(),
        ));
    }
    Ok(safe_target)
}
