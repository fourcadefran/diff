# Plan 2 — `diff-agent` binary + RemoteGitBackend Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **NOTA:** Este plan asume que el Plan 1 (`2026-04-25-diff-backend-abstraction.md`) ya fue ejecutado y mergeado. Algunas signatures de structs/funciones podrían necesitar pequeños ajustes según el resultado real del Plan 1.

**Goal:** Crear el binario `diff-agent` que expone `LocalGitBackend` por stdio usando JSON-RPC 2.0 line-delimited, e implementar `RemoteGitBackend` en `src-tauri` que se comunica con el agente. Al final del plan, el `RemoteGitBackend` funciona contra una instancia del `diff-agent` corriendo como subproceso local (no SSH todavía).

**Architecture:** Nuevo crate `crates/diff-agent` con un solo binario que abre un repo via `LocalGitBackend` y entra en un loop: leer línea de stdin → parsear como JSON-RPC Request → dispatch al método del trait → escribir JSON-RPC Response a stdout. Notificaciones del watcher se envían como JSON-RPC Notifications (sin id). Del lado del cliente, `RemoteGitBackend` mantiene un `Child` con stdin/stdout piped y un thread reader que separa Responses (correlacionadas por id con un `HashMap<u64, Sender<Result>>`) de Notifications (que invocan al callback del watcher).

**Tech Stack:** Rust 2021, serde_json, std::process, std::sync::mpsc.

---

## File Structure

**Creados:**
- `crates/diff-agent/Cargo.toml`
- `crates/diff-agent/src/main.rs` — binario, parseo de args, loop stdio
- `crates/diff-agent/src/dispatch.rs` — match `method` → call al trait, serializa Response
- `crates/diff-core/src/protocol.rs` — tipos JSON-RPC (Request, Response, Notification, ErrorObj) y helpers de framing
- `src-tauri/src/remote_backend.rs` — `RemoteGitBackend` que implementa `GitBackend` hablando con un Child por stdio
- `crates/diff-agent/tests/agent_e2e.rs` — tests end-to-end del agente como subproceso

**Modificados:**
- `Cargo.toml` (root, workspace) — agregar `crates/diff-agent` a `members`
- `crates/diff-core/Cargo.toml` — agregar feature flag opcional o dependencia para protocol
- `crates/diff-core/src/lib.rs` — `pub mod protocol;` y re-exports
- `src-tauri/Cargo.toml` — sin cambios (`RemoteGitBackend` solo usa stdlib + diff-core)
- `src-tauri/src/lib.rs` — `mod remote_backend;` (la integración con un comando Tauri queda para Plan 3)

---

## Phase A: Tipos del protocolo en `diff-core`

### Task A1: Definir `protocol.rs` con los tipos JSON-RPC

**Files:**
- Create: `crates/diff-core/src/protocol.rs`
- Modify: `crates/diff-core/src/lib.rs`

- [ ] **Step 1: Crear `crates/diff-core/src/protocol.rs`**

```rust
//! JSON-RPC 2.0 message types used between diff-agent (server) and the
//! RemoteGitBackend in src-tauri (client). Line-delimited framing: one JSON
//! object per line, terminated with '\n'.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    #[serde(flatten)]
    pub outcome: ResponseOutcome,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseOutcome {
    Result(Value),
    Error(ErrorObj),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ErrorObj {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Marker type that always serializes as the literal "2.0" required by
/// JSON-RPC 2.0. Using a typed marker (instead of `String`) catches malformed
/// frames at the deserialization boundary.
#[derive(Debug, Clone, Copy)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "2.0" {
            Ok(JsonRpcVersion)
        } else {
            Err(serde::de::Error::custom(format!("unsupported jsonrpc version: {s}")))
        }
    }
}

// Standard JSON-RPC error codes plus our application range (-32000 to -32099).
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const APPLICATION_ERROR: i32 = -32000;
}

/// Build a Response carrying a successful result.
pub fn ok_response(id: u64, result: Value) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        outcome: ResponseOutcome::Result(result),
    }
}

/// Build a Response carrying an error.
pub fn err_response(id: u64, code: i32, message: String) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        outcome: ResponseOutcome::Error(ErrorObj {
            code,
            message,
            data: None,
        }),
    }
}

/// Build a Notification (no id, no response expected).
pub fn notification(method: &str, params: Value) -> Notification {
    Notification {
        jsonrpc: JsonRpcVersion,
        method: method.to_string(),
        params,
    }
}
```

- [ ] **Step 2: Exportar el módulo desde `diff-core/src/lib.rs`**

Agregar al `lib.rs`:
```rust
pub mod protocol;
```

- [ ] **Step 3: Test de roundtrip serde**

`crates/diff-core/tests/protocol.rs`:
```rust
use diff_core::protocol::{
    err_response, notification, ok_response, ErrorObj, JsonRpcVersion, Message, Notification,
    Request, Response, ResponseOutcome,
};
use serde_json::json;

#[test]
fn request_roundtrip() {
    let req = Request {
        jsonrpc: JsonRpcVersion,
        id: 42,
        method: "get_status".to_string(),
        params: json!({}),
    };
    let s = serde_json::to_string(&req).unwrap();
    assert!(s.contains("\"jsonrpc\":\"2.0\""));
    assert!(s.contains("\"id\":42"));
    assert!(s.contains("\"method\":\"get_status\""));
    let _back: Request = serde_json::from_str(&s).unwrap();
}

#[test]
fn response_ok_roundtrip() {
    let resp = ok_response(7, json!({"branch": "main"}));
    let s = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, 7);
    match back.outcome {
        ResponseOutcome::Result(v) => assert_eq!(v["branch"], "main"),
        _ => panic!("expected Result"),
    }
}

#[test]
fn response_err_roundtrip() {
    let resp = err_response(8, -32000, "no repository open".to_string());
    let s = serde_json::to_string(&resp).unwrap();
    let back: Response = serde_json::from_str(&s).unwrap();
    assert_eq!(back.id, 8);
    match back.outcome {
        ResponseOutcome::Error(e) => {
            assert_eq!(e.code, -32000);
            assert_eq!(e.message, "no repository open");
        }
        _ => panic!("expected Error"),
    }
}

#[test]
fn notification_roundtrip() {
    let notif = notification("repo:changed", json!({}));
    let s = serde_json::to_string(&notif).unwrap();
    let back: Notification = serde_json::from_str(&s).unwrap();
    assert_eq!(back.method, "repo:changed");
}

#[test]
fn message_enum_dispatches_correctly() {
    let req_str = r#"{"jsonrpc":"2.0","id":1,"method":"get_status","params":{}}"#;
    let resp_str = r#"{"jsonrpc":"2.0","id":1,"result":{"branch":"main"}}"#;
    let notif_str = r#"{"jsonrpc":"2.0","method":"repo:changed","params":{}}"#;

    assert!(matches!(serde_json::from_str::<Message>(req_str).unwrap(), Message::Request(_)));
    assert!(matches!(serde_json::from_str::<Message>(resp_str).unwrap(), Message::Response(_)));
    assert!(matches!(serde_json::from_str::<Message>(notif_str).unwrap(), Message::Notification(_)));
}

#[test]
fn rejects_wrong_jsonrpc_version() {
    let bad = r#"{"jsonrpc":"1.0","id":1,"method":"x","params":{}}"#;
    assert!(serde_json::from_str::<Request>(bad).is_err());
}
```

- [ ] **Step 4: Correr tests**

Run: `cargo test -p diff-core --test protocol`
Expected: PASS los 6 tests.

---

## Phase B: Implementar `diff-agent`

### Task B1: Crear el crate `diff-agent`

**Files:**
- Create: `crates/diff-agent/Cargo.toml`
- Create: `crates/diff-agent/src/main.rs` (skeleton)
- Modify: `Cargo.toml` (workspace root)

- [ ] **Step 1: Agregar `crates/diff-agent` al workspace**

Editar `Cargo.toml` root:
```toml
[workspace]
members = ["src-tauri", "crates/diff-core", "crates/diff-agent"]
resolver = "2"
# resto sin cambios
```

- [ ] **Step 2: Crear `crates/diff-agent/Cargo.toml`**

```toml
[package]
name = "diff-agent"
edition.workspace = true
version.workspace = true

[[bin]]
name = "diff-agent"
path = "src/main.rs"

[dependencies]
diff-core = { path = "../diff-core" }
serde.workspace = true
serde_json.workspace = true
```

- [ ] **Step 3: Crear el skeleton `crates/diff-agent/src/main.rs`**

```rust
use std::path::PathBuf;
use std::process::ExitCode;

mod dispatch;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opts = match parse_args(&args) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("[diff-agent] {e}");
            eprintln!("usage: diff-agent --stdio --repo <path>");
            return ExitCode::from(2);
        }
    };

    if let Err(e) = dispatch::run(opts) {
        eprintln!("[diff-agent] fatal: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[derive(Debug)]
pub struct Options {
    pub repo: PathBuf,
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut stdio = false;
    let mut repo: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--stdio" => stdio = true,
            "--repo" => {
                i += 1;
                let value = args.get(i).ok_or("--repo requires a value")?;
                repo = Some(PathBuf::from(value));
            }
            other => return Err(format!("unknown argument: {other}")),
        }
        i += 1;
    }
    if !stdio {
        return Err("--stdio is required (only stdio transport is supported in v1)".to_string());
    }
    let repo = repo.ok_or("--repo is required")?;
    Ok(Options { repo })
}
```

- [ ] **Step 4: Verificar que el workspace compila con el nuevo crate (todavía sin dispatch.rs)**

Crear stub `crates/diff-agent/src/dispatch.rs`:
```rust
use crate::Options;

pub fn run(_opts: Options) -> Result<(), String> {
    Err("not implemented yet".to_string())
}
```

Run: `cargo check --workspace`
Expected: PASS.

### Task B2: Implementar el loop stdio del agente

**Files:**
- Modify: `crates/diff-agent/src/dispatch.rs`

- [ ] **Step 1: Reemplazar `dispatch.rs` con la implementación real del loop**

```rust
use std::io::{self, BufRead, Write};
use std::sync::{Arc, Mutex};

use diff_core::protocol::{
    err_response, error_codes, notification, ok_response, Message, Request, Response,
};
use diff_core::{GitBackend, LocalGitBackend, WatcherHandle};
use serde_json::{json, Value};

use crate::Options;

pub fn run(opts: Options) -> Result<(), String> {
    let backend = LocalGitBackend::open(&opts.repo).map_err(|e| e.to_string())?;
    let backend: Arc<dyn GitBackend> = Arc::new(backend);

    // Stdout shared between the request loop (responses) and the watcher
    // callback (notifications). Mutex avoids interleaved JSON lines.
    let stdout = Arc::new(Mutex::new(io::stdout()));

    let stdout_for_watcher = Arc::clone(&stdout);
    let _watcher: WatcherHandle = backend
        .subscribe_changes(Box::new(move || {
            let notif = notification("repo:changed", json!({}));
            if let Ok(mut out) = stdout_for_watcher.lock() {
                let _ = writeln!(out, "{}", serde_json::to_string(&notif).unwrap());
                let _ = out.flush();
            }
        }))
        .map_err(|e| e.to_string())?;

    let stdin = io::stdin();
    let reader = stdin.lock();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => return Err(format!("stdin read error: {e}")),
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Message>(&line) {
            Ok(Message::Request(req)) => handle_request(backend.as_ref(), req),
            Ok(_) => continue, // Responses/Notifications ignoradas: somos servidor.
            Err(e) => err_response(0, error_codes::PARSE_ERROR, format!("parse error: {e}")),
        };

        let mut out = stdout.lock().map_err(|e| format!("stdout poisoned: {e}"))?;
        writeln!(out, "{}", serde_json::to_string(&response).map_err(|e| e.to_string())?)
            .map_err(|e| format!("stdout write: {e}"))?;
        out.flush().map_err(|e| format!("stdout flush: {e}"))?;
    }

    Ok(())
}

fn handle_request(backend: &dyn GitBackend, req: Request) -> Response {
    match req.method.as_str() {
        "get_status" => map_result(req.id, backend.get_status()),
        "get_branch" => map_result(req.id, backend.get_branch()),
        "stage_file" => match params_str_field(&req.params, "path") {
            Ok(path) => map_unit(req.id, backend.stage_file(&path)),
            Err(e) => err_response(req.id, error_codes::INVALID_PARAMS, e),
        },
        "unstage_file" => match params_str_field(&req.params, "path") {
            Ok(path) => map_unit(req.id, backend.unstage_file(&path)),
            Err(e) => err_response(req.id, error_codes::INVALID_PARAMS, e),
        },
        "stage_all" => map_unit(req.id, backend.stage_all()),
        "unstage_all" => map_unit(req.id, backend.unstage_all()),
        "discard_file" => match params_str_field(&req.params, "path") {
            Ok(path) => map_unit(req.id, backend.discard_file(&path)),
            Err(e) => err_response(req.id, error_codes::INVALID_PARAMS, e),
        },
        "commit" => {
            let message = match params_str_field(&req.params, "message") {
                Ok(m) => m,
                Err(e) => return err_response(req.id, error_codes::INVALID_PARAMS, e),
            };
            let amend = req.params.get("amend").and_then(|v| v.as_bool()).unwrap_or(false);
            map_result(req.id, backend.commit(&message, amend))
        }
        "get_file_contents_batch" => match req.params.get("requests").cloned() {
            Some(v) => match serde_json::from_value(v) {
                Ok(reqs) => map_result(req.id, backend.get_file_contents_batch(reqs)),
                Err(e) => err_response(req.id, error_codes::INVALID_PARAMS, e.to_string()),
            },
            None => err_response(
                req.id,
                error_codes::INVALID_PARAMS,
                "missing 'requests' field".to_string(),
            ),
        },
        other => err_response(
            req.id,
            error_codes::METHOD_NOT_FOUND,
            format!("unknown method: {other}"),
        ),
    }
}

fn params_str_field(params: &Value, field: &str) -> Result<String, String> {
    params
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing or non-string field '{field}'"))
}

fn map_result<T: serde::Serialize>(id: u64, r: Result<T, diff_core::BackendError>) -> Response {
    match r {
        Ok(value) => match serde_json::to_value(&value) {
            Ok(v) => ok_response(id, v),
            Err(e) => err_response(id, error_codes::INTERNAL_ERROR, format!("serialize: {e}")),
        },
        Err(e) => err_response(id, error_codes::APPLICATION_ERROR, e.to_string()),
    }
}

fn map_unit(id: u64, r: Result<(), diff_core::BackendError>) -> Response {
    match r {
        Ok(()) => ok_response(id, json!(null)),
        Err(e) => err_response(id, error_codes::APPLICATION_ERROR, e.to_string()),
    }
}
```

- [ ] **Step 2: Verificar que compila**

Run: `cargo check --workspace`
Expected: PASS.

- [ ] **Step 3: Smoke manual del agente**

Run en una terminal:
```bash
cd /tmp && rm -rf testrepo && mkdir testrepo && cd testrepo && git init && git config user.email t@t.com && git config user.name T
echo '{"jsonrpc":"2.0","id":1,"method":"get_status","params":{}}' | cargo run -p diff-agent -- --stdio --repo /tmp/testrepo
```

Expected: una línea de output JSON con `"id":1`, `"result":{"staged":[],"unstaged":[],"untracked":[]}`.

### Task B3: Tests end-to-end del agente como subproceso

**Files:**
- Create: `crates/diff-agent/tests/agent_e2e.rs`

- [ ] **Step 1: Helper para spawnear el agente y dialogarle por stdio**

```rust
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

use serde_json::{json, Value};
use tempfile::TempDir;

struct AgentHandle {
    child: Child,
    stdin: Mutex<ChildStdin>,
    stdout: Mutex<BufReader<ChildStdout>>,
    next_id: Mutex<u64>,
}

impl AgentHandle {
    fn spawn(repo: &std::path::Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_diff-agent");
        let mut child = Command::new(bin)
            .args(["--stdio", "--repo", repo.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn diff-agent");
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        AgentHandle {
            child,
            stdin: Mutex::new(stdin),
            stdout: Mutex::new(BufReader::new(stdout)),
            next_id: Mutex::new(1),
        }
    }

    fn call(&self, method: &str, params: Value) -> Value {
        let id = {
            let mut n = self.next_id.lock().unwrap();
            let v = *n;
            *n += 1;
            v
        };
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        {
            let mut stdin = self.stdin.lock().unwrap();
            writeln!(stdin, "{}", serde_json::to_string(&req).unwrap()).unwrap();
            stdin.flush().unwrap();
        }

        // Skip notifications until we see our response.
        loop {
            let mut line = String::new();
            self.stdout.lock().unwrap().read_line(&mut line).expect("read");
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&line).expect("parse line");
            if v.get("id").and_then(|v| v.as_u64()) == Some(id) {
                return v;
            }
            // It's a notification — keep reading.
        }
    }

    fn shutdown(mut self) {
        drop(self.stdin.into_inner().unwrap()); // EOF → agent exits.
        let _ = self.child.wait();
    }
}

fn init_test_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    let path = dir.path();
    Command::new("git").args(["init"]).current_dir(path).status().unwrap();
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
```

- [ ] **Step 2: Test: status vacío**

Agregar al mismo archivo:
```rust
#[test]
fn agent_returns_empty_status_on_fresh_repo() {
    let dir = init_test_repo();
    let agent = AgentHandle::spawn(dir.path());
    let resp = agent.call("get_status", json!({}));
    let result = &resp["result"];
    assert!(result["staged"].as_array().unwrap().is_empty());
    assert!(result["unstaged"].as_array().unwrap().is_empty());
    assert!(result["untracked"].as_array().unwrap().is_empty());
    agent.shutdown();
}
```

- [ ] **Step 3: Test: untracked aparece, stage lo mueve**

```rust
#[test]
fn agent_stage_flow() {
    let dir = init_test_repo();
    std::fs::write(dir.path().join("a.txt"), "hi").unwrap();
    let agent = AgentHandle::spawn(dir.path());

    let resp = agent.call("get_status", json!({}));
    assert_eq!(resp["result"]["untracked"][0], "a.txt");

    let resp = agent.call("stage_file", json!({"path": "a.txt"}));
    assert!(resp.get("error").is_none(), "got error: {resp:?}");

    let resp = agent.call("get_status", json!({}));
    assert!(resp["result"]["untracked"].as_array().unwrap().is_empty());
    assert_eq!(resp["result"]["staged"][0]["path"], "a.txt");

    let resp = agent.call("commit", json!({"message": "init", "amend": false}));
    let oid = resp["result"].as_str().unwrap();
    assert_eq!(oid.len(), 40);

    agent.shutdown();
}
```

- [ ] **Step 4: Test: método desconocido devuelve error structured**

```rust
#[test]
fn agent_unknown_method_returns_error() {
    let dir = init_test_repo();
    let agent = AgentHandle::spawn(dir.path());
    let resp = agent.call("nonexistent", json!({}));
    assert_eq!(resp["error"]["code"], -32601);
    assert!(resp["error"]["message"].as_str().unwrap().contains("nonexistent"));
    agent.shutdown();
}
```

- [ ] **Step 5: Correr todos los tests**

Run: `cargo test -p diff-agent`
Expected: PASS los 3 tests.

---

## Phase C: Implementar `RemoteGitBackend` en `src-tauri`

### Task C1: Skeleton del `RemoteGitBackend`

**Files:**
- Create: `src-tauri/src/remote_backend.rs`
- Modify: `src-tauri/src/lib.rs` (declarar `mod remote_backend;`)

- [ ] **Step 1: Crear `src-tauri/src/remote_backend.rs`**

```rust
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
```

- [ ] **Step 2: Declarar el módulo en `src-tauri/src/lib.rs`**

Agregar al principio:
```rust
mod git;
mod remote_backend;
mod review_bridge;
```

- [ ] **Step 3: Verificar build**

Run: `cargo check --workspace`
Expected: PASS.

### Task C2: Test end-to-end de `RemoteGitBackend` contra el binario real

**Files:**
- Create: `src-tauri/tests/remote_backend_e2e.rs`

- [ ] **Step 1: Test que spawna `diff-agent` como subprocess local**

```rust
use std::path::PathBuf;
use std::process::Command;

use diff_core::GitBackend;
use diff_lib::remote_backend::RemoteGitBackend;
use tempfile::TempDir;

fn agent_binary_path() -> PathBuf {
    // The integration tests in src-tauri can't use CARGO_BIN_EXE_diff-agent
    // (different crate). Locate the binary via cargo metadata or env override.
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
    Command::new("git").args(["init"]).current_dir(path).status().unwrap();
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
```

Notar que el test toma `RemoteGitBackend` desde `diff_lib::remote_backend` — para esto, `mod remote_backend;` en `src-tauri/src/lib.rs` debe ser `pub mod remote_backend;`.

- [ ] **Step 2: Cambiar `mod remote_backend;` a `pub mod remote_backend;` en `src-tauri/src/lib.rs`**

- [ ] **Step 3: Test de notificación de watcher**

Agregar al mismo `tests/remote_backend_e2e.rs`:
```rust
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

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
```

- [ ] **Step 4: Correr los tests**

Run: `cargo test -p diff --test remote_backend_e2e`
Expected: PASS los 2 tests. El test del watcher tiene un `sleep` de hasta 2s — si flake, ajustar el timeout. Si falla consistentemente, debug con `RUST_LOG`/`stderr` del agente (ya redirige a `Stdio::inherit`).

### Task C3: Verificación final del Plan 2

- [ ] **Step 1: Correr toda la suite del workspace**

Run: `cargo test --workspace`
Expected: PASS todo. No debe haber regresiones en los tests del Plan 1.

- [ ] **Step 2: Verificar que `diff-agent` builda en release**

Run: `cargo build --release -p diff-agent`
Expected: produce `target/release/diff-agent`.

- [ ] **Step 3: Verificar que la GUI sigue funcionando como antes**

Run: `bun run tauri dev`
Verificar manualmente: la app local sigue trabajando idéntica al final del Plan 1 (el `RemoteGitBackend` existe pero no está cableado a ningún comando Tauri todavía — eso es Plan 3).

---

## Self-Review Checklist (al cerrar el plan)

- [ ] El crate `diff-agent` produce un binario standalone que arranca con `--stdio --repo <path>`.
- [ ] El protocolo JSON-RPC tiene tests de roundtrip serde para los 3 tipos de mensaje.
- [ ] El `RemoteGitBackend` implementa todos los métodos del trait `GitBackend`.
- [ ] El reader thread separa correctamente Responses (correlacionadas por id) de Notifications.
- [ ] Drop de `RemoteGitBackend` mata al child process (verificar que no quedan procesos `diff-agent` colgados después de tests).
- [ ] Los tests end-to-end usan el binario real, no mocks.
