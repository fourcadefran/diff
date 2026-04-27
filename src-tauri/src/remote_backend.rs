use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use diff_core::protocol::{Message, Request, Response, ResponseOutcome};
use diff_core::{
    BackendError, FileContentsBatchItem, FileContentsRequest, GitBackend, RepoStatus,
    WatcherHandle,
};
use serde_json::{json, Value};

/// Connection to a remote `diff-agent` process. Owns the SSH/subprocess child,
/// a request id counter, a map of pending responses, and a background thread
/// that reads frames from the agent's stdout.
pub struct RemoteGitBackend {
    inner: Arc<Inner>,
}

struct Inner {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Sender<Response>>>,
    notification_sink: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    reader_handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl RemoteGitBackend {
    /// Spawn `command` (already configured with args, stdin/stdout piped) and
    /// install the reader thread. The `Command` should be e.g. `ssh host
    /// diff-agent --stdio --repo <path>` or, for tests, the local `diff-agent`
    /// binary directly.
    pub fn spawn(mut command: Command) -> Result<Self, BackendError> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|e| BackendError::Transport(format!("spawn failed: {e}")))?;

        let stdin = child.stdin.take().ok_or_else(|| {
            BackendError::Transport("child has no stdin".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            BackendError::Transport("child has no stdout".to_string())
        })?;

        let inner = Arc::new(Inner {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(1),
            pending: Mutex::new(HashMap::new()),
            notification_sink: Mutex::new(None),
            reader_handle: Mutex::new(None),
        });

        let inner_for_reader = Arc::clone(&inner);
        let handle = thread::spawn(move || reader_loop(inner_for_reader, BufReader::new(stdout)));

        *inner.reader_handle.lock().unwrap() = Some(handle);

        Ok(Self { inner })
    }

    fn call(&self, method: &str, params: Value) -> Result<Value, BackendError> {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);

        let (tx, rx): (Sender<Response>, Receiver<Response>) = mpsc::channel();
        self.inner
            .pending
            .lock()
            .map_err(|e| BackendError::Transport(format!("pending lock poisoned: {e}")))?
            .insert(id, tx);

        let req = Request {
            jsonrpc: diff_core::protocol::JsonRpcVersion,
            id,
            method: method.to_string(),
            params,
        };
        let line = serde_json::to_string(&req)
            .map_err(|e| BackendError::Transport(format!("serialize: {e}")))?;

        {
            let mut stdin = self
                .inner
                .stdin
                .lock()
                .map_err(|e| BackendError::Transport(format!("stdin lock poisoned: {e}")))?;
            writeln!(stdin, "{}", line)
                .map_err(|e| BackendError::Transport(format!("write: {e}")))?;
            stdin
                .flush()
                .map_err(|e| BackendError::Transport(format!("flush: {e}")))?;
        }

        let resp = rx
            .recv()
            .map_err(|_| BackendError::Transport("agent closed before responding".to_string()))?;

        match resp.outcome {
            ResponseOutcome::Result(v) => Ok(v),
            ResponseOutcome::Error(e) => Err(BackendError::Protocol {
                code: e.code,
                message: e.message,
            }),
        }
    }
}

fn reader_loop(inner: Arc<Inner>, mut stdout: BufReader<ChildStdout>) {
    let mut line = String::new();
    loop {
        line.clear();
        match stdout.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(_) => break,
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg = match serde_json::from_str::<Message>(trimmed) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("[diff-remote] failed to parse frame: {e} (line={trimmed})");
                continue;
            }
        };
        match msg {
            Message::Response(resp) => {
                let id = resp.id;
                let tx_opt = inner.pending.lock().ok().and_then(|mut p| p.remove(&id));
                if let Some(tx) = tx_opt {
                    let _ = tx.send(resp);
                }
            }
            Message::Notification(notif) => {
                if notif.method == "repo:changed" {
                    if let Ok(guard) = inner.notification_sink.lock() {
                        if let Some(cb) = guard.as_ref() {
                            cb();
                        }
                    }
                }
            }
            Message::Request(_) => {
                // Server should not send requests to the client. Ignore.
            }
        }
    }
    // Cancel any pending callers so they unblock with Transport error.
    if let Ok(mut p) = inner.pending.lock() {
        p.clear();
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Explicitly kill the child so the SSH session terminates promptly.
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
        }
    }
}

impl GitBackend for RemoteGitBackend {
    fn get_status(&self) -> Result<RepoStatus, BackendError> {
        let v = self.call("get_status", json!({}))?;
        serde_json::from_value(v).map_err(|e| BackendError::Transport(format!("decode: {e}")))
    }

    fn get_branch(&self) -> Result<Option<String>, BackendError> {
        let v = self.call("get_branch", json!({}))?;
        serde_json::from_value(v).map_err(|e| BackendError::Transport(format!("decode: {e}")))
    }

    fn get_file_contents_batch(
        &self,
        requests: Vec<FileContentsRequest>,
    ) -> Result<Vec<FileContentsBatchItem>, BackendError> {
        let v = self.call("get_file_contents_batch", json!({ "requests": requests }))?;
        serde_json::from_value(v).map_err(|e| BackendError::Transport(format!("decode: {e}")))
    }

    fn stage_file(&self, path: &str) -> Result<(), BackendError> {
        self.call("stage_file", json!({ "path": path }))?;
        Ok(())
    }

    fn unstage_file(&self, path: &str) -> Result<(), BackendError> {
        self.call("unstage_file", json!({ "path": path }))?;
        Ok(())
    }

    fn stage_all(&self) -> Result<(), BackendError> {
        self.call("stage_all", json!({}))?;
        Ok(())
    }

    fn unstage_all(&self) -> Result<(), BackendError> {
        self.call("unstage_all", json!({}))?;
        Ok(())
    }

    fn discard_file(&self, path: &str) -> Result<(), BackendError> {
        self.call("discard_file", json!({ "path": path }))?;
        Ok(())
    }

    fn commit(&self, message: &str, amend: bool) -> Result<String, BackendError> {
        let v = self.call("commit", json!({ "message": message, "amend": amend }))?;
        serde_json::from_value(v).map_err(|e| BackendError::Transport(format!("decode: {e}")))
    }

    fn subscribe_changes(
        &self,
        sink: Box<dyn Fn() + Send + Sync>,
    ) -> Result<WatcherHandle, BackendError> {
        *self
            .inner
            .notification_sink
            .lock()
            .map_err(|e| BackendError::Transport(format!("sink lock poisoned: {e}")))? = Some(sink);

        // The "handle" we return is a guard struct that clears the sink on drop.
        struct RemoteWatcherGuard(Arc<Inner>);
        impl Drop for RemoteWatcherGuard {
            fn drop(&mut self) {
                if let Ok(mut g) = self.0.notification_sink.lock() {
                    *g = None;
                }
            }
        }

        Ok(WatcherHandle {
            _inner: Box::new(RemoteWatcherGuard(Arc::clone(&self.inner))),
        })
    }
}
