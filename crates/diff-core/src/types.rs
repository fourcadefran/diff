use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Serialize, Deserialize)]
#[serde(tag = "kind", content = "message")]
pub enum BackendError {
    /// Errors originating in git2 (open, status, diff, commit, etc.).
    #[error("git error: {0}")]
    Git(String),

    /// Transport-level failures: broken pipe to the agent, malformed JSON-RPC
    /// frames, EOF before response, etc. Only emitted by RemoteGitBackend.
    #[error("transport error: {0}")]
    Transport(String),

    /// Structured error returned by the agent in a JSON-RPC error response.
    #[error("agent error ({code}): {message}")]
    Protocol { code: i32, message: String },

    /// Local I/O errors that don't come from git2 (read, stat, remove, etc.).
    #[error("io error: {0}")]
    Io(String),

    /// Caller invoked an operation that requires an open repo without one.
    #[error("no repository open")]
    NoRepoOpen,

    /// Bare repositories are unsupported by the GUI.
    #[error("bare repositories are not supported")]
    BareRepo,

    /// Path validation rejected a relative path with `..` or absolute components.
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

impl From<git2::Error> for BackendError {
    fn from(err: git2::Error) -> Self {
        BackendError::Git(err.to_string())
    }
}

impl From<std::io::Error> for BackendError {
    fn from(err: std::io::Error) -> Self {
        BackendError::Io(err.to_string())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoStatus {
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub untracked: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileContentsRequest {
    pub path: String,
    pub staged: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileContentsResponse {
    pub name: String,
    pub old_content: Option<String>,
    pub old_binary: bool,
    pub new_content: Option<String>,
    pub new_binary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileContentsBatchItem {
    pub path: String,
    pub response: Option<FileContentsResponse>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CloneProgress {
    pub id: String,
    pub phase: String,
    pub received_objects: usize,
    pub total_objects: usize,
    pub indexed_objects: usize,
    pub received_bytes: usize,
    pub checkout_current: usize,
    pub checkout_total: usize,
}

/// Handle returned by `subscribe_changes`. Dropping it stops the underlying
/// watcher (filesystem notifier for local, notification reader thread for remote).
pub struct WatcherHandle {
    pub _inner: Box<dyn std::any::Any + Send + Sync>,
}
