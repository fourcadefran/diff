//! diff-core — git operations and protocol types shared between the Tauri GUI
//! and the standalone diff-agent binary.

pub mod local;
pub mod protocol;
pub mod types;
pub mod watcher;

pub use local::LocalGitBackend;
pub use types::{
    BackendError, ChangeKind, CloneProgress, FileContentsBatchItem, FileContentsRequest,
    FileContentsResponse, FileEntry, RepoStatus, WatcherHandle,
};

/// Operations exposed by any git backend (local in-process or remote RPC).
pub trait GitBackend: Send {
    fn get_status(&self) -> Result<RepoStatus, BackendError>;
    fn get_file_contents_batch(
        &self,
        requests: Vec<FileContentsRequest>,
    ) -> Result<Vec<FileContentsBatchItem>, BackendError>;
    fn stage_file(&self, path: &str) -> Result<(), BackendError>;
    fn unstage_file(&self, path: &str) -> Result<(), BackendError>;
    fn stage_all(&self) -> Result<(), BackendError>;
    fn unstage_all(&self) -> Result<(), BackendError>;
    fn commit(&self, message: &str, amend: bool) -> Result<String, BackendError>;
    fn discard_file(&self, path: &str) -> Result<(), BackendError>;
    fn get_branch(&self) -> Result<Option<String>, BackendError>;
    fn subscribe_changes(
        &self,
        sink: Box<dyn Fn() + Send + Sync>,
    ) -> Result<WatcherHandle, BackendError>;
}
