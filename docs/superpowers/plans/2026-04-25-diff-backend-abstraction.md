# Plan 1 — Rename + GitBackend abstraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Renombrar `cub-dev` → `diff` en todo el repo y refactorizar el backend Rust detrás de un trait `GitBackend` con una implementación local (`LocalGitBackend`) que vive en un nuevo crate `diff-core`. Al final, la app debe seguir funcionando idéntica para repos locales.

**Architecture:** Conversión de `src-tauri` (crate único) en un workspace Cargo con dos crates: `diff-core` (lógica git pura, sin Tauri) y `src-tauri` (GUI Tauri que consume `diff-core`). El `AppState` cambia de `Mutex<Option<Repository>>` a `Mutex<Option<Box<dyn GitBackend>>>`. Cada comando Tauri se vuelve un shim delgado que delega al trait.

**Tech Stack:** Rust 2021, Tauri 2, git2 0.20, notify 8, serde, Vite, React 19, Tailwind 4, Bun.

---

## File Structure

**Renombrados (sin cambio de contenido más allá del rename):**
- `sidecar/cub-mcp.js` → `sidecar/diff-mcp.js`

**Modificados:**
- `package.json` — name, scripts mcp:server/mcp:mcp paths
- `bun.lock` — name field (auto-regen también funciona, pero lo tocamos explícito)
- `src-tauri/Cargo.toml` — package name, lib name, **agregar dependencia a diff-core**
- `src-tauri/tauri.conf.json` — productName, identifier, window title, resource path
- `src-tauri/src/main.rs` — referencias `cub_dev_lib` → `diff_lib`, log prefixes
- `src-tauri/src/lib.rs` — comentario `cub [path]`, `mod git`/`mod watcher` cambian (watcher se va a diff-core)
- `src-tauri/src/git.rs` — **reescrito completo** como shims que delegan al `GitBackend` trait
- `src-tauri/src/watcher.rs` — **borrado** (movido a diff-core)
- `src-tauri/src/review_bridge.rs` — `~/.cub` → `~/.diff`, `cub-mcp.js` → `diff-mcp.js`
- `sidecar/diff-mcp.js` — `~/.cub` → `~/.diff`, strings "Cub" → "diff", server name "cub" → "diff"
- `.mcp.json` — server name "cub" → "diff", args path
- `index.html` — `<title>cub</title>` → `<title>diff</title>`
- `onboarding-prompt.md` — strings "cub" → "diff" donde corresponde
- `src/lib/perf.ts` — `[cub-perf]` → `[diff-perf]`
- `src/hooks/use-diffs.ts` — `[cub]` log prefixes
- `src/hooks/use-repo-status.ts` — localStorage key `cub:last-opened-repo` → `diff:last-opened-repo`
- `src/hooks/use-recent-repos.ts` — localStorage key `cub:recent-repos` → `diff:recent-repos`
- `src/App.tsx` — comentario y log
- `src/components/onboarding/onboarding.tsx` — heading "cub" → "diff"
- `src/components/sidebar/sidebar.tsx` — `treeId="cub-staged-tree"`/`"cub-unstaged-tree"`

**Creados:**
- `Cargo.toml` (root, workspace) — declara members `["src-tauri", "crates/diff-core"]`
- `crates/diff-core/Cargo.toml` — package, dependencias git2/notify/serde
- `crates/diff-core/src/lib.rs` — re-exports + trait `GitBackend` + `BackendError` + structs públicos
- `crates/diff-core/src/types.rs` — structs movidos desde git.rs (RepoStatus, FileEntry, ChangeKind, FileContents*, CloneProgress, BackendError)
- `crates/diff-core/src/local.rs` — `LocalGitBackend` (lógica antes en git.rs, sin código Tauri)
- `crates/diff-core/src/watcher.rs` — código de notify (movido desde src-tauri/src/watcher.rs, sin AppHandle)

---

## Phase A: Rename `cub-dev` → `diff`

### Task A1: Renombrar package metadata (Rust + JS)

**Files:**
- Modify: `package.json`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Editar `package.json` — campo `name`**

```json
{
  "name": "diff",
  "private": true,
  "version": "0.1.0",
  ...
}
```

- [ ] **Step 2: Editar `src-tauri/Cargo.toml` — `package.name` y `lib.name`**

```toml
[package]
name = "diff"
version = "0.1.0"
description = "A Tauri App"
authors = ["you"]
edition = "2021"

[lib]
name = "diff_lib"
crate-type = ["staticlib", "cdylib", "rlib"]
```

- [ ] **Step 3: Editar `src-tauri/tauri.conf.json` — productName, identifier, window title**

```jsonc
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "diff",
  "version": "0.1.0",
  "identifier": "com.fourcadefran.diff",
  ...
  "app": {
    "windows": [
      {
        "title": "diff",
        ...
      }
    ],
    ...
  },
  ...
}
```

- [ ] **Step 4: Verificar que el repo todavía buildea**

Run: `bun install && cd src-tauri && cargo check`
Expected: cargo regenera `Cargo.lock` con el nuevo nombre, compila sin errores. Bun reporta: `package "diff"`.

### Task A2: Renombrar referencias `cub_dev_lib` → `diff_lib` en main.rs

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Reemplazar las 3 referencias `cub_dev_lib::` por `diff_lib::`**

Edit `src-tauri/src/main.rs`:

```rust
// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--mcp" || a == "-m") {
        return run_mcp_mode();
    }

    if let Some(path) = args.iter().find(|a| !a.starts_with('-')) {
        if let Ok(abs) = std::fs::canonicalize(path) {
            diff_lib::set_launch_path(abs);
        } else {
            eprintln!("[diff] could not resolve path: {path}");
        }
    }

    diff_lib::run();
    ExitCode::SUCCESS
}

fn run_mcp_mode() -> ExitCode {
    let script = match diff_lib::sidecar_script_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[diff] {e}");
            return ExitCode::from(2);
        }
    };

    let spawn = |runtime: &str| Command::new(runtime).arg(&script).arg("mcp").status();

    let status = match spawn("node") {
        Ok(s) => s,
        Err(node_err) => match spawn("bun") {
            Ok(s) => s,
            Err(bun_err) => {
                eprintln!("[diff] failed to spawn MCP sidecar: node={node_err}, bun={bun_err}");
                return ExitCode::from(1);
            }
        },
    };

    ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8)
}
```

- [ ] **Step 2: Verificar que compila**

Run: `cd src-tauri && cargo check`
Expected: PASS, cero errores de símbolos no resueltos.

### Task A3: Renombrar el sidecar JS

**Files:**
- Rename: `sidecar/cub-mcp.js` → `sidecar/diff-mcp.js`
- Modify: `sidecar/diff-mcp.js` (post-rename)
- Modify: `package.json`
- Modify: `.mcp.json`
- Modify: `src-tauri/src/review_bridge.rs`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Renombrar el archivo**

Run: `git mv sidecar/cub-mcp.js sidecar/diff-mcp.js`

- [ ] **Step 2: Actualizar referencias internas dentro del sidecar**

Editar `sidecar/diff-mcp.js`:
- Línea 16: `path.join(os.homedir(), ".cub")` → `path.join(os.homedir(), ".diff")`
- Línea 582: `"Cub review server is not running"` → `"diff review server is not running"`
- Línea 623: `name: "cub",` → `name: "diff",`
- Línea 633: `"Fetch a code review batch from Cub..."` → `"Fetch a code review batch from diff..."`
- Línea 651: `"No reviews pending in Cub."` → `"No reviews pending in diff."` (dos ocurrencias en líneas 651 y 653)
- Línea 659: `"Cub review batch"` → `"diff review batch"`
- Línea 662: `"Failed to read Cub review:"` → `"Failed to read diff review:"`
- Línea 673: `"Mark a single review item inside a Cub review batch..."` → `"...diff review batch..."`
- Línea 701: `"Block until a new code review batch arrives in Cub..."` → `"...arrives in diff..."`
- Línea 737: `"New Cub review batch"` → `"New diff review batch"`
- Línea 751: `"Dismiss a single review item inside a Cub review batch..."` → `"...diff review batch..."`
- Línea 791: `"Usage: node sidecar/cub-mcp.js <server|mcp>"` → `"Usage: node sidecar/diff-mcp.js <server|mcp>"`
- Línea 798: `` `Cub MCP sidecar failed: ...` `` → `` `diff MCP sidecar failed: ...` ``

- [ ] **Step 3: Actualizar `package.json` scripts**

```jsonc
{
  ...
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "tauri": "tauri",
    "mcp:server": "node sidecar/diff-mcp.js server",
    "mcp:mcp": "node sidecar/diff-mcp.js mcp"
  },
  ...
}
```

- [ ] **Step 4: Actualizar `.mcp.json`**

```json
{
  "_comment": "Do not delete this file. It is used for local MCP server testing.",
  "mcpServers": {
    "diff": {
      "command": "node",
      "args": ["sidecar/diff-mcp.js", "mcp"]
    }
  }
}
```

- [ ] **Step 5: Actualizar `src-tauri/src/review_bridge.rs` — state_dir y sidecar paths**

En `src-tauri/src/review_bridge.rs`:

Línea ~67 (función `state_dir`):
```rust
fn state_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "failed to resolve home directory".to_string())?;
    Ok(home.join(".diff"))
}
```

Líneas ~91-97 (función `sidecar_script_path`):
```rust
pub fn sidecar_script_path() -> Result<PathBuf, String> {
    let root = workspace_root()?;
    let path = root.join("sidecar").join("diff-mcp.js");
    if path.exists() {
        return Ok(path);
    }
    let flat = root.join("diff-mcp.js");
    if flat.exists() {
        return Ok(flat);
    }
    Err(format!(
        "missing sidecar script (checked {} and {})",
        path.display(),
        flat.display()
    ))
}
```

- [ ] **Step 6: Actualizar `src-tauri/tauri.conf.json` — bundle resources path**

```jsonc
{
  ...
  "bundle": {
    "active": true,
    "targets": "all",
    "resources": ["../sidecar/diff-mcp.js"],
    ...
  }
}
```

- [ ] **Step 7: Verificar que cargo y bun siguen contentos**

Run: `cd src-tauri && cargo check && cd .. && bun install`
Expected: PASS sin errores.

### Task A4: Renombrar log prefixes en Rust

**Files:**
- Modify: `src-tauri/src/git.rs`
- Modify: `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Reemplazar prefixes en `git.rs`**

En `src-tauri/src/git.rs`:
- Línea ~19: `eprintln!("[cub-perf] rust:{op} {extra}");` → `eprintln!("[diff-perf] rust:{op} {extra}");`
- Línea ~135: `eprintln!("[cub-watcher] failed to start: {e}");` → `eprintln!("[diff-watcher] failed to start: {e}");`

- [ ] **Step 2: Reemplazar prefixes en `watcher.rs`**

En `src-tauri/src/watcher.rs` línea ~35:
```rust
Err(errors) => {
    for err in errors {
        eprintln!("[diff-watcher] error: {err}");
    }
}
```

- [ ] **Step 3: Reemplazar comentario en `lib.rs`**

En `src-tauri/src/lib.rs` línea ~16:
```rust
/// Record an initial repository path supplied by `diff [path]` on the command
/// line. Called before Tauri is built so the frontend can pick it up on mount.
pub fn set_launch_path(path: PathBuf) {
    let _ = LAUNCH_PATH.set(path);
}
```

- [ ] **Step 4: Verificar que compila**

Run: `cd src-tauri && cargo check`
Expected: PASS.

### Task A5: Renombrar strings frontend

**Files:**
- Modify: `index.html`
- Modify: `src/lib/perf.ts`
- Modify: `src/hooks/use-diffs.ts`
- Modify: `src/hooks/use-repo-status.ts`
- Modify: `src/hooks/use-recent-repos.ts`
- Modify: `src/App.tsx`
- Modify: `src/components/onboarding/onboarding.tsx`
- Modify: `src/components/sidebar/sidebar.tsx`

- [ ] **Step 1: `index.html` — title**

```html
<title>diff</title>
```

- [ ] **Step 2: `src/lib/perf.ts` — PREFIX**

Línea ~4:
```typescript
const PREFIX = "[diff-perf]";
```

(Si la línea ~2 contiene el comentario `[cub-perf]`, también actualizalo a `[diff-perf]`.)

- [ ] **Step 3: `src/hooks/use-diffs.ts` — log prefixes**

Líneas ~152 y ~175:
```typescript
"[diff] failed to fetch diff:",
// ...
console.warn("[diff] failed to fetch diff batch:", err);
```

- [ ] **Step 4: `src/hooks/use-repo-status.ts` — localStorage key**

Línea ~24:
```typescript
const LAST_OPENED_REPO_KEY = "diff:last-opened-repo";
```

- [ ] **Step 5: `src/hooks/use-recent-repos.ts` — localStorage key**

Línea ~3:
```typescript
const STORAGE_KEY = "diff:recent-repos";
```

- [ ] **Step 6: `src/App.tsx` — comentario y log**

Líneas ~381 y ~401:
```typescript
// Honor `diff [path]` first; otherwise restore the last successfully opened repo.
// ...
.catch((e) => console.error("[diff] getLaunchPath failed:", e));
```

- [ ] **Step 7: `src/components/onboarding/onboarding.tsx` — heading**

Línea ~73 (dentro del JSX del header):
```jsx
<h1 className="font-heading text-3xl font-semibold tracking-tight">
  diff
</h1>
```

- [ ] **Step 8: `src/components/sidebar/sidebar.tsx` — treeIds**

Líneas ~114 y ~128:
```jsx
treeId="diff-staged-tree"
// ...
treeId="diff-unstaged-tree"
```

- [ ] **Step 9: Verificar que typecheck del frontend pasa**

Run: `bun run build`
Expected: `tsc` pasa sin errores, `vite build` produce `dist/`.

### Task A6: Smoke test post-rename

- [ ] **Step 1: Levantar la app y verificar branding**

Run: `bun run tauri dev`
Expected:
- Ventana titulada "diff" (no "cub-dev").
- Onboarding muestra "diff" como heading.
- En logs de stderr aparecen `[diff]`, `[diff-perf]`, `[diff-watcher]`.
- Abrir un repo local (ej. `~/projects/diff` mismo) funciona y muestra archivos.
- Hacer un cambio en disco dispara refresh (watcher funciona).

- [ ] **Step 2: Verificar review_bridge / sidecar arrancó**

Mirar stderr o `~/.diff/review-bridge.json` (debería existir si la app arrancó OK).

Run: `ls ~/.diff/`
Expected: existe `review-bridge.json`. La carpeta `~/.cub` puede seguir existiendo de antes — no la borramos, es state vieja.

---

## Phase B: Crear workspace + crate `diff-core`

### Task B1: Convertir el repo en workspace Cargo

**Files:**
- Create: `Cargo.toml` (root)
- Modify: `src-tauri/Cargo.toml`
- Create: `crates/diff-core/Cargo.toml`
- Create: `crates/diff-core/src/lib.rs` (stub)

- [ ] **Step 1: Crear `Cargo.toml` en la raíz del repo**

```toml
[workspace]
members = ["src-tauri", "crates/diff-core"]
resolver = "2"

[workspace.package]
edition = "2021"
version = "0.1.0"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
git2 = { version = "0.20", default-features = false, features = ["https", "ssh"] }
notify = "8"
notify-debouncer-full = "0.5"

[profile.dev.package."*"]
opt-level = 3
```

Notar que el `[profile.dev.package."*"]` que estaba en `src-tauri/Cargo.toml` se mueve al workspace root (los profiles van solo en el manifest top-level).

- [ ] **Step 2: Crear `crates/diff-core/Cargo.toml`**

```toml
[package]
name = "diff-core"
edition.workspace = true
version.workspace = true

[dependencies]
serde.workspace = true
serde_json.workspace = true
git2.workspace = true
notify.workspace = true
notify-debouncer-full.workspace = true

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Crear el lib stub `crates/diff-core/src/lib.rs`**

```rust
//! diff-core — git operations and protocol types shared between the Tauri GUI
//! and the standalone diff-agent binary.

pub mod local;
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
```

Crear también stubs vacíos por ahora para que compile:

`crates/diff-core/src/types.rs`:
```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, thiserror::Error)]
pub enum BackendError {}

// Placeholder structs — se completan en tasks B3 y B4.
#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Typechange,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FileEntry {
    pub path: String,
    pub kind: ChangeKind,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Serialize, Deserialize)]
pub struct RepoStatus {
    pub staged: Vec<FileEntry>,
    pub unstaged: Vec<FileEntry>,
    pub untracked: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct FileContentsRequest {
    pub path: String,
    pub staged: bool,
}

#[derive(Serialize, Deserialize)]
pub struct FileContentsResponse {
    pub name: String,
    pub old_content: Option<String>,
    pub old_binary: bool,
    pub new_content: Option<String>,
    pub new_binary: bool,
}

#[derive(Serialize, Deserialize)]
pub struct FileContentsBatchItem {
    pub path: String,
    pub response: Option<FileContentsResponse>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, Clone)]
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
```

`crates/diff-core/src/watcher.rs` (stub temporal — se llena en B2):
```rust
// Stub: real content arrives in Task B2.
```

`crates/diff-core/src/local.rs` (stub temporal — se llena en B5):
```rust
use crate::{BackendError, GitBackend, FileContentsBatchItem, FileContentsRequest, RepoStatus, WatcherHandle};

pub struct LocalGitBackend;

impl GitBackend for LocalGitBackend {
    fn get_status(&self) -> Result<RepoStatus, BackendError> { unimplemented!() }
    fn get_file_contents_batch(&self, _: Vec<FileContentsRequest>) -> Result<Vec<FileContentsBatchItem>, BackendError> { unimplemented!() }
    fn stage_file(&self, _: &str) -> Result<(), BackendError> { unimplemented!() }
    fn unstage_file(&self, _: &str) -> Result<(), BackendError> { unimplemented!() }
    fn stage_all(&self) -> Result<(), BackendError> { unimplemented!() }
    fn unstage_all(&self) -> Result<(), BackendError> { unimplemented!() }
    fn commit(&self, _: &str, _: bool) -> Result<String, BackendError> { unimplemented!() }
    fn discard_file(&self, _: &str) -> Result<(), BackendError> { unimplemented!() }
    fn get_branch(&self) -> Result<Option<String>, BackendError> { unimplemented!() }
    fn subscribe_changes(&self, _: Box<dyn Fn() + Send + Sync>) -> Result<WatcherHandle, BackendError> { unimplemented!() }
}
```

- [ ] **Step 4: Actualizar `src-tauri/Cargo.toml` para usar workspace**

```toml
[package]
name = "diff"
version.workspace = true
description = "A Tauri App"
authors = ["you"]
edition.workspace = true

[lib]
name = "diff_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
diff-core = { path = "../crates/diff-core" }
tauri = { version = "2", features = [] }
tauri-plugin-opener = "2"
tauri-plugin-dialog = "2"
serde.workspace = true
serde_json.workspace = true
git2.workspace = true
ureq = { version = "3", features = ["json"] }
notify.workspace = true
notify-debouncer-full.workspace = true
```

Sacar el bloque `[profile.dev.package."*"]` que estaba al final (ahora vive en el workspace root).

- [ ] **Step 5: Agregar `thiserror` al workspace**

En `Cargo.toml` root, agregar a `[workspace.dependencies]`:
```toml
thiserror = "2"
```

En `crates/diff-core/Cargo.toml` `[dependencies]`:
```toml
thiserror.workspace = true
```

- [ ] **Step 6: Verificar que el workspace compila**

Run: `cargo check --workspace`
Expected: PASS. Cargo detecta el workspace, builda `diff-core` (con stubs `unimplemented!()`) y `diff` (todavía con la lógica vieja en src-tauri/src/git.rs sin tocar).

### Task B2: Mover `watcher.rs` a `diff-core`

**Files:**
- Create: `crates/diff-core/src/watcher.rs` (con contenido real)
- Delete: `src-tauri/src/watcher.rs`
- Modify: `src-tauri/src/lib.rs` (quitar `mod watcher;`)
- Modify: `src-tauri/src/git.rs` (cambiar `crate::watcher` → `diff_core::watcher`)

- [ ] **Step 1: Copiar el contenido de `src-tauri/src/watcher.rs` a `crates/diff-core/src/watcher.rs`, sin Tauri**

`crates/diff-core/src/watcher.rs`:
```rust
use std::ffi::OsStr;
use std::path::{Component, Path};
use std::time::Duration;

use notify::{Config, RecommendedWatcher, RecursiveMode};
use notify_debouncer_full::{new_debouncer_opt, DebounceEventResult, Debouncer, NoCache};

/// Debounced filesystem watcher. Dropping it stops the background thread and
/// releases the watch on the underlying directory.
pub struct RepoWatcher {
    _debouncer: Debouncer<RecommendedWatcher, NoCache>,
}

/// Start a recursive watch on `workdir`. Worktree changes and status-relevant
/// `.git` changes are coalesced with a 300 ms debounce. The provided `on_change`
/// callback is invoked from a notify worker thread.
pub fn start<F>(workdir: &Path, on_change: F) -> Result<RepoWatcher, String>
where
    F: Fn() + Send + Sync + 'static,
{
    let mut debouncer: Debouncer<RecommendedWatcher, NoCache> = new_debouncer_opt(
        Duration::from_millis(300),
        None,
        move |result: DebounceEventResult| match result {
            Ok(events) => {
                let relevant = events
                    .iter()
                    .any(|ev| ev.event.paths.iter().any(|p| path_is_relevant(p)));
                if relevant {
                    on_change();
                }
            }
            Err(errors) => {
                for err in errors {
                    eprintln!("[diff-watcher] error: {err}");
                }
            }
        },
        NoCache::new(),
        Config::default(),
    )
    .map_err(|e| format!("failed to create watcher: {e}"))?;

    debouncer
        .watch(workdir, RecursiveMode::Recursive)
        .map_err(|e| format!("failed to watch {}: {e}", workdir.display()))?;

    Ok(RepoWatcher {
        _debouncer: debouncer,
    })
}

fn path_is_relevant(path: &Path) -> bool {
    let mut inside_git = false;
    let mut first_git_component: Option<&OsStr> = None;
    let mut last_git_component: Option<&OsStr> = None;

    for component in path.components() {
        match component {
            Component::Normal(name) if name == ".git" => inside_git = true,
            Component::Normal(name) if inside_git => {
                if first_git_component.is_none() {
                    first_git_component = Some(name);
                }
                last_git_component = Some(name);
            }
            _ => {}
        }
    }

    if !inside_git {
        return true;
    }
    let Some(first_git_component) = first_git_component else {
        return false;
    };
    if last_git_component
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".lock"))
    {
        return false;
    }

    match first_git_component.to_str() {
        Some(
            "HEAD" | "index" | "packed-refs" | "MERGE_HEAD" | "CHERRY_PICK_HEAD" | "REVERT_HEAD"
            | "REBASE_HEAD" | "ORIG_HEAD" | "refs",
        ) => true,
        Some(
            "objects" | "logs" | "hooks" | "info" | "lfs" | "fsmonitor--daemon" | "rr-cache"
            | "worktrees" | "modules" | "COMMIT_EDITMSG" | "FETCH_HEAD",
        ) => false,
        _ => false,
    }
}
```

- [ ] **Step 2: Actualizar `src-tauri/src/lib.rs` para quitar `mod watcher`**

Editar `src-tauri/src/lib.rs`:
```rust
mod git;
mod review_bridge;
// (mod watcher; eliminado — vive en diff-core ahora)

use git::AppState;
// resto sin cambios por ahora
```

- [ ] **Step 3: Actualizar `restart_watcher` en `git.rs` para usar `diff_core::watcher`**

En `src-tauri/src/git.rs`, las dos referencias a `crate::watcher` (líneas ~70 y ~39) cambian a `diff_core::watcher`:

```rust
// línea ~39:
pub watcher: Mutex<Option<diff_core::watcher::RepoWatcher>>,

// línea ~70 (dentro del thread spawn):
let result = diff_core::watcher::start(&workdir, {
    let app = app.clone();
    move || {
        let _ = app.emit("repo:changed", ());
    }
});
```

(Notar que `start` ahora toma una clausura en lugar de `app: AppHandle`. El `app.emit("repo:changed", ())` que antes vivía dentro de `start` se mueve afuera.)

- [ ] **Step 4: Borrar `src-tauri/src/watcher.rs`**

Run: `git rm src-tauri/src/watcher.rs`

- [ ] **Step 5: Verificar build**

Run: `cargo check --workspace`
Expected: PASS.

### Task B3: Definir `BackendError` real y mover los structs comunes

**Files:**
- Modify: `crates/diff-core/src/types.rs`

- [ ] **Step 1: Reescribir `crates/diff-core/src/types.rs` con tipos completos**

```rust
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
```

- [ ] **Step 2: Verificar build**

Run: `cargo check --workspace`
Expected: PASS.

### Task B4: Implementar `LocalGitBackend` en `diff-core`

**Files:**
- Modify: `crates/diff-core/src/local.rs` (reemplazar el stub completo)

Esta es la tarea más grande del plan: mover toda la lógica de `src-tauri/src/git.rs` (excepto los `#[tauri::command]`) a `LocalGitBackend`. El código es esencialmente el mismo, sólo cambia: (a) toma `&self` en lugar de leer de `state.repo`, (b) no emite `perf_event` (eso es cosa del shim Tauri, lo agregamos de vuelta en task C2 si hace falta), (c) devuelve `BackendError` en lugar de `String`.

- [ ] **Step 1: Crear el archivo `crates/diff-core/src/local.rs` con la struct base**

```rust
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{
    FetchOptions, Index, Patch, RemoteCallbacks, Repository, Status, StatusOptions, Tree,
};

use crate::types::{
    BackendError, ChangeKind, CloneProgress, FileContentsBatchItem, FileContentsRequest,
    FileContentsResponse, FileEntry, RepoStatus, WatcherHandle,
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
        let workdir = repo
            .workdir()
            .ok_or(BackendError::BareRepo)?
            .to_path_buf();
        Ok(Self {
            repo: Mutex::new(repo),
            workdir,
        })
    }

    /// Initialize a new repo at `path`. Creates the directory if missing.
    pub fn init(path: &Path) -> Result<Self, BackendError> {
        std::fs::create_dir_all(path)?;
        let repo = Repository::init(path).map_err(BackendError::from)?;
        let workdir = repo
            .workdir()
            .ok_or(BackendError::BareRepo)?
            .to_path_buf();
        Ok(Self {
            repo: Mutex::new(repo),
            workdir,
        })
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}
```

- [ ] **Step 2: Mover los helpers internos de `git.rs` (decode_file_side, read_head_tree, read_tree_file, validate_repo_relative_path, read_workdir_file, read_index_file, stage_path, stage_index_path, unstage_path, collect_status_paths, collect_paths, remove_workdir_entry, canonical_contained_target, build_diff_for_count, count_diff_lines_parallel, FileSideContent, CountDiffKind)**

Copiarlos tal cual desde `src-tauri/src/git.rs` (líneas ~275 a ~1178), agregándolos al final de `crates/diff-core/src/local.rs`. Cambios:

- Las funciones que devolvían `Result<_, String>` ahora devuelven `Result<_, BackendError>`. Los `format!("...: {e}")` cambian a `BackendError::Git(...)` o `?` directo (el `From<git2::Error>` lo cubre).
- `validate_repo_relative_path` devuelve `Result<(), BackendError>` con `BackendError::InvalidPath`.
- `count_diff_lines_parallel` y `build_diff_for_count` se quedan como funciones libres dentro del módulo `local` (no son métodos del trait).
- `FileSideContent` y sus impls se mantienen igual.
- `CountDiffKind` igual.
- `remove_workdir_entry`, `canonical_contained_target` igual con cambio de `String` → `BackendError::Io`/`BackendError::InvalidPath`.

Ejemplo de la migración para `validate_repo_relative_path`:

```rust
fn validate_repo_relative_path(path: &Path) -> Result<(), BackendError> {
    for component in path.components() {
        match component {
            Component::Normal(_) | Component::CurDir => {}
            _ => return Err(BackendError::InvalidPath(path.display().to_string())),
        }
    }
    Ok(())
}
```

Ejemplo para `read_workdir_file`:

```rust
fn read_workdir_file(workdir: &Path, path: &Path) -> Result<FileSideContent, BackendError> {
    validate_repo_relative_path(path)?;
    let abs = workdir.join(path);
    let meta = match std::fs::symlink_metadata(&abs) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSideContent::absent());
        }
        Err(e) => return Err(BackendError::Io(format!("cannot stat {}: {e}", path.display()))),
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
        Err(e) => Err(BackendError::Io(format!("cannot read {}: {e}", path.display()))),
    }
}
```

Estos helpers todos siguen siendo `fn` libres (no métodos), llamados desde los métodos del trait.

- [ ] **Step 3: Implementar `GitBackend` para `LocalGitBackend` — métodos read-only (`get_status`, `get_branch`, `get_file_contents_batch`)**

Agregar al final de `local.rs`:

```rust
impl GitBackend for LocalGitBackend {
    fn get_status(&self) -> Result<RepoStatus, BackendError> {
        let workdir_path = self.workdir.clone();

        let (status_entries, staged_counts, unstaged_counts) = {
            let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;

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
            let unstaged_counts =
                count_diff_lines_parallel(&workdir_path, CountDiffKind::Unstaged, unstaged_delta_count);

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

        Ok(RepoStatus { staged, unstaged, untracked })
    }

    fn get_branch(&self) -> Result<Option<String>, BackendError> {
        let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
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
            let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
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
                            .map(|req| process_file_request(&repo, head_tree.as_ref(), index.as_ref(), workdir, req))
                            .collect()
                    })
                })
                .collect();

            handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        });

        Ok(responses)
    }

    // ... resto de métodos en steps siguientes
}

fn process_file_request(
    repo: &Repository,
    head_tree: Option<&Tree<'_>>,
    index: Option<&Index>,
    workdir: &Path,
    req: &FileContentsRequest,
) -> FileContentsBatchItem {
    let rel = Path::new(&req.path);
    let name = rel.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| req.path.clone());

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
```

- [ ] **Step 4: Implementar los métodos write (`stage_file`, `unstage_file`, `stage_all`, `unstage_all`, `commit`, `discard_file`)**

Agregar dentro del `impl GitBackend for LocalGitBackend`:

```rust
fn stage_file(&self, path: &str) -> Result<(), BackendError> {
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
    stage_path(&repo, path)
}

fn stage_all(&self) -> Result<(), BackendError> {
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
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
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
    unstage_path(&repo, path)
}

fn unstage_all(&self) -> Result<(), BackendError> {
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
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
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
    let workdir = repo.workdir().ok_or(BackendError::BareRepo)?;
    let relative_path = Path::new(path);
    validate_repo_relative_path(relative_path)?;
    let target = canonical_contained_target(workdir, relative_path)?;

    let status = repo.status_file(relative_path).map_err(BackendError::from)?;
    let index_dirty = status.intersects(
        Status::INDEX_NEW
            | Status::INDEX_MODIFIED
            | Status::INDEX_DELETED
            | Status::INDEX_RENAMED
            | Status::INDEX_TYPECHANGE,
    );

    if status.contains(Status::WT_NEW) && !index_dirty {
        remove_workdir_entry(&target)?;
        return Ok(());
    }

    if index_dirty {
        match repo.revparse_single("HEAD") {
            Ok(head_obj) => {
                repo.reset_default(Some(&head_obj), [path]).map_err(BackendError::from)?;
            }
            Err(_) => {
                let mut index = repo.index().map_err(BackendError::from)?;
                let _ = index.remove_path(relative_path);
                index.write().map_err(BackendError::from)?;
            }
        }
    }

    if repo.revparse_single("HEAD").is_ok() {
        let mut cb = CheckoutBuilder::new();
        cb.force();
        cb.path(path);
        repo.checkout_head(Some(&mut cb)).map_err(BackendError::from)?;
    }

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
    let repo = self.repo.lock().map_err(|e| BackendError::Git(format!("lock poisoned: {e}")))?;
    if message.trim().is_empty() {
        return Err(BackendError::Git("commit message cannot be empty".to_string()));
    }
    let mut index = repo.index().map_err(BackendError::from)?;
    let tree_oid = index.write_tree().map_err(BackendError::from)?;

    if !amend {
        if let Ok(head_ref) = repo.head() {
            if let Ok(head_commit) = head_ref.peel_to_commit() {
                if head_commit.tree_id() == tree_oid {
                    return Err(BackendError::Git("nothing to commit: index matches HEAD".to_string()));
                }
            }
        }
    }

    let tree = repo.find_tree(tree_oid).map_err(BackendError::from)?;
    let sig = repo.signature().map_err(BackendError::from)?;

    let oid = if amend {
        let head_ref = repo.head().map_err(|_| BackendError::Git("cannot amend: no commit to amend".to_string()))?;
        let head_commit = head_ref.peel_to_commit().map_err(BackendError::from)?;
        head_commit
            .amend(Some("HEAD"), Some(&sig), Some(&sig), None, Some(message), Some(&tree))
            .map_err(BackendError::from)?
    } else {
        match repo.head() {
            Ok(head_ref) => {
                let parent = head_ref.peel_to_commit().map_err(BackendError::from)?;
                repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent]).map_err(BackendError::from)?
            }
            Err(_) => repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[]).map_err(BackendError::from)?,
        }
    };

    Ok(oid.to_string())
}

fn subscribe_changes(
    &self,
    sink: Box<dyn Fn() + Send + Sync>,
) -> Result<WatcherHandle, BackendError> {
    let watcher_inst = watcher::start(&self.workdir, move || sink()).map_err(BackendError::Io)?;
    Ok(WatcherHandle {
        _inner: Box::new(watcher_inst),
    })
}
```

- [ ] **Step 5: Verificar build**

Run: `cargo check --workspace`
Expected: PASS. Pueden aparecer warnings de funciones no usadas (e.g. `CloneProgress` no se usa todavía); ignorables.

### Task B5: Tests unitarios mínimos para `LocalGitBackend`

**Files:**
- Create: `crates/diff-core/tests/local_backend.rs`

- [ ] **Step 1: Escribir test de smoke: open + status vacío**

`crates/diff-core/tests/local_backend.rs`:
```rust
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
```

- [ ] **Step 2: Correr tests**

Run: `cargo test -p diff-core`
Expected: PASS los 4 tests. Si alguno falla por configuración de git (e.g. no hay `git` en PATH) lo marcamos como skipped con un comentario.

---

## Phase C: Refactorizar `src-tauri` para usar el trait

### Task C1: Reemplazar `AppState.repo` por `AppState.backend`

**Files:**
- Modify: `src-tauri/src/git.rs` (el `AppState` struct)
- Modify: `src-tauri/src/lib.rs` (el `manage(AppState{...})`)

- [ ] **Step 1: Modificar la struct `AppState` en `src-tauri/src/git.rs`**

Reemplazar líneas 33-41 con:

```rust
pub struct AppState {
    pub backend: Mutex<Option<Box<dyn diff_core::GitBackend>>>,
    pub bridge: Mutex<Option<Child>>,
    pub event_listener: Mutex<Option<JoinHandle<()>>>,
    pub event_listener_stop: Arc<AtomicBool>,
    pub clone_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub watcher_handle: Mutex<Option<diff_core::WatcherHandle>>,
    pub watcher_generation: AtomicU64,
}
```

(El `watcher` antes era un `RepoWatcher`; ahora es un `WatcherHandle` opaco devuelto por el trait.)

- [ ] **Step 2: Actualizar el `.manage(AppState{...})` en `src-tauri/src/lib.rs`**

Reemplazar el bloque `manage` (líneas ~34-42) con:

```rust
.manage(AppState {
    backend: Mutex::new(None),
    bridge: Mutex::new(None),
    event_listener: Mutex::new(None),
    event_listener_stop: stop_flag.clone(),
    clone_cancels: Mutex::new(HashMap::new()),
    watcher_handle: Mutex::new(None),
    watcher_generation: AtomicU64::new(0),
})
```

- [ ] **Step 3: Actualizar el `RunEvent::Exit` handler en lib.rs**

El bloque que cerraba el `watcher` (líneas ~104-106) cambia a:

```rust
if let Ok(mut guard) = state.watcher_handle.lock() {
    *guard = None;
}
```

- [ ] **Step 4: Verificar — sabemos que va a fallar porque los comandos todavía leen `state.repo`**

Run: `cargo check -p diff`
Expected: FAIL con varios errores `no field 'repo' on type 'AppState'`. Esto está bien, los arreglamos en C2.

### Task C2: Reescribir `src-tauri/src/git.rs` como shims

**Files:**
- Modify: `src-tauri/src/git.rs` (rewrite total — solo shims que delegan al backend)

Este archivo pasa de 1414 líneas a ~250 líneas porque toda la lógica vive ahora en `diff-core`. Solo quedan las anotaciones `#[tauri::command]` y la creación/cierre del backend en `open_repo`/`init_repo`/`clone_repo`.

- [ ] **Step 1: Reescribir `src-tauri/src/git.rs` completo**

```rust
use std::collections::HashMap;
use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use git2::build::{CheckoutBuilder, RepoBuilder};
use git2::{FetchOptions, RemoteCallbacks, Repository};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, State};

use diff_core::{
    BackendError, FileContentsBatchItem, FileContentsRequest, GitBackend, LocalGitBackend,
    RepoStatus, WatcherHandle,
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

#[derive(Serialize, Clone)]
pub struct CloneProgress {
    pub id: String,
    pub phase: &'static str,
    pub received_objects: usize,
    pub total_objects: usize,
    pub indexed_objects: usize,
    pub received_bytes: usize,
    pub checkout_current: usize,
    pub checkout_total: usize,
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
                    phase: "fetch",
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
                    phase: "checkout",
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
```

Notar:
- `clone_repo` mantiene el código de `git2::RepoBuilder` con callbacks de progreso porque la API streaming del backend trait no expone callbacks de clone (out of scope para v1, las llamadas `clone_repo` en repos remotos van a venir en plan 2). Después del clone, abre el repo vía `LocalGitBackend::open` y lo instala.
- `perf_event` se conserva pero solo se llama en `open_repo` ahora. La instrumentación detallada por método no se reproduce — es ruido y el comentario original decía "Remove once the lag investigation is done".

- [ ] **Step 2: Verificar build**

Run: `cargo check --workspace`
Expected: PASS.

### Task C3: Smoke test final del Plan 1

- [ ] **Step 1: Levantar la app y verificar paridad funcional con pre-refactor**

Run: `bun run tauri dev`

Verificar manualmente en la app:
- [ ] Abre un repo local (ej. el mismo `~/projects/diff`).
- [ ] Lista de archivos staged/unstaged/untracked aparece bien.
- [ ] Stage / unstage de un archivo individual funciona.
- [ ] Stage all / unstage all funcionan.
- [ ] Commit con un mensaje funciona.
- [ ] Discard de un archivo modificado funciona.
- [ ] Modificar un archivo en disco dispara refresh (el watcher emite `repo:changed`).
- [ ] Diff view muestra contenido de archivo correctamente (verifica `get_file_contents_batch`).
- [ ] Init de un repo nuevo en directorio vacío funciona.
- [ ] Clone de un repo público (HTTPS) funciona y muestra progreso.

- [ ] **Step 2: Correr tests del workspace**

Run: `cargo test --workspace`
Expected: PASS los tests de `diff-core`.

- [ ] **Step 3: Build de release para verificar que el bundle se arma**

Run: `bun run tauri build`
Expected: produce `src-tauri/target/release/bundle/...` con el `.app`/`.dmg` titulado "diff" y bundle id `com.fourcadefran.diff`.

---

## Self-Review Checklist (al cerrar el plan, antes de marcar tareas como completadas)

- [ ] Todos los `cub` / `cub-dev` / `cub_dev_lib` removidos. Verificar con: `grep -ri "cub" --exclude-dir=node_modules --exclude-dir=target --exclude="bun.lock" --exclude="*.lock" .` (filtrar matches en CSS `cubic-bezier`).
- [ ] El `Cargo.toml` workspace lista correctamente `src-tauri` y `crates/diff-core`.
- [ ] `diff-core` no depende de `tauri` (verificar `cargo tree -p diff-core | grep -i tauri` → vacío).
- [ ] Los structs públicos en `diff-core::types` tienen `Serialize + Deserialize` (los necesita el plan 2).
- [ ] La app sigue funcionando para repos locales con paridad funcional.
