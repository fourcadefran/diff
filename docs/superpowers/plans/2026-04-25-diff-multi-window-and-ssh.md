# Plan 3 — Multi-window picker + SSH integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **NOTA:** Este plan asume Plan 1 + Plan 2 ya ejecutados. Algunas signatures podrían ajustarse según resultados reales. Revisar especialmente la API de `RemoteGitBackend::spawn` y la forma exacta de `AppState`.

**Goal:** Convertir la app de single-window a multi-window con un picker que enumera hosts (Local + entradas de `~/.ssh/config`) y repos recientes por host, persistencia SQLite de recents en la Mac, y comando Tauri `open_remote_repo` que spawnea `ssh <host> diff-agent --stdio --repo <path>` para conectarse a la PC host del usuario.

**Architecture:** El picker es una ventana Tauri creada al arrancar (label `"picker"`, dimensiones 480×600) que la app abre por default cuando no se le pasa un path por CLI. Cada repo se abre en una ventana nueva (label dinámico `"repo-<n>"`) con `WebviewWindowBuilder`.

**Importante — corrección al spec:** Tauri 2 corre todas las ventanas en un único proceso (no un proceso por ventana como decía el spec). Para soportar múltiples repos en paralelo con backends independientes, `AppState.backend` deja de ser `Mutex<Option<Box<dyn GitBackend>>>` (una sola entrada) y pasa a ser `Mutex<HashMap<String, Box<dyn GitBackend>>>` keyed por window label. Cada comando Tauri toma `window: tauri::Window` además de `state: State<AppState>` y resuelve `window.label()` para encontrar su backend. El watcher_handle también se mueve al map.

El picker usa nuevos comandos Tauri: `list_ssh_hosts`, `list_recent_repos`, `record_recent_repo`, `open_picker_repo`. SQLite vive en `~/Library/Application Support/diff/state.db` (o equivalente cross-platform via `dirs::data_dir`).

**Tech Stack:** Tauri 2 multi-window APIs (`WebviewWindowBuilder`, `Manager::get_webview_window`), `rusqlite` con `bundled`, `ssh2-config` para parsear `~/.ssh/config`.

---

## File Structure

**Modificados:**
- `src-tauri/Cargo.toml` — agregar `rusqlite = { version = "0.34", features = ["bundled"] }`, `ssh2-config = "0.4"`, `dirs = "6"`
- `src-tauri/src/lib.rs` — registrar nuevos comandos, lógica de window-creation, SQLite init
- `src-tauri/src/main.rs` — parseo de `<host>:<path>` además de `<local-path>`
- `src-tauri/tauri.conf.json` — definir la ventana inicial como picker (label, dimensiones)
- `src/App.tsx` — soporte para inicializar como picker o repo-window según query param
- `src/lib/tauri.ts` — wrappers para los nuevos comandos
- `package.json` — sin cambios (ya tiene todo)

**Creados:**
- `src-tauri/src/storage.rs` — SQLite open + queries (`list_repos_for_host`, `record_recent`)
- `src-tauri/src/ssh_config.rs` — parser de `~/.ssh/config` → `Vec<String>` de host aliases
- `src-tauri/src/window.rs` — helpers para crear ventanas de repo (label único, builder, state init)
- `src/components/picker/picker.tsx` — UI completa del picker
- `src/components/picker/host-list.tsx` — sidebar con Local + hosts SSH
- `src/components/picker/recent-list.tsx` — panel derecho con recents del host seleccionado
- `src/lib/window.ts` — wrappers frontend para detección de window kind y creación

---

## Phase A: Persistencia SQLite

### Task A1: Setup de `rusqlite` y schema

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Create: `src-tauri/src/storage.rs`

- [ ] **Step 1: Agregar dependencias a `src-tauri/Cargo.toml`**

```toml
[dependencies]
# ... existing deps ...
rusqlite = { version = "0.34", features = ["bundled"] }
dirs = "6"
ssh2-config = "0.4"
```

- [ ] **Step 2: Crear `src-tauri/src/storage.rs`**

```rust
use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecentRepo {
    pub host: String,
    pub path: String,
    pub last_opened_at: i64, // unix epoch seconds
}

pub struct Storage {
    conn: Mutex<Connection>,
}

impl Storage {
    /// Open `~/Library/Application Support/diff/state.db` (macOS) or the
    /// equivalent on other platforms. Creates the file and schema if missing.
    pub fn open_default() -> Result<Self, String> {
        let path = default_db_path()?;
        Self::open_at(&path)
    }

    pub fn open_at(path: &std::path::Path) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create state dir: {e}"))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open db: {e}"))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS repos (
                host TEXT NOT NULL,
                path TEXT NOT NULL,
                last_opened_at INTEGER NOT NULL,
                PRIMARY KEY (host, path)
            );
            "#,
        )
        .map_err(|e| format!("create schema: {e}"))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    pub fn list_for_host(&self, host: &str) -> Result<Vec<RecentRepo>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT host, path, last_opened_at FROM repos WHERE host = ?1 ORDER BY last_opened_at DESC")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![host], |row| {
                Ok(RecentRepo {
                    host: row.get(0)?,
                    path: row.get(1)?,
                    last_opened_at: row.get(2)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    pub fn list_all_recent(&self, limit: usize) -> Result<Vec<RecentRepo>, String> {
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        let mut stmt = conn
            .prepare("SELECT host, path, last_opened_at FROM repos ORDER BY last_opened_at DESC LIMIT ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok(RecentRepo {
                    host: row.get(0)?,
                    path: row.get(1)?,
                    last_opened_at: row.get(2)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| format!("row: {e}"))?);
        }
        Ok(out)
    }

    pub fn record_open(&self, host: &str, path: &str) -> Result<(), String> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("clock: {e}"))?
            .as_secs() as i64;
        let conn = self.conn.lock().map_err(|e| format!("lock poisoned: {e}"))?;
        conn.execute(
            r#"INSERT INTO repos (host, path, last_opened_at) VALUES (?1, ?2, ?3)
               ON CONFLICT (host, path) DO UPDATE SET last_opened_at = excluded.last_opened_at"#,
            params![host, path, now],
        )
        .map_err(|e| format!("insert: {e}"))?;
        Ok(())
    }
}

fn default_db_path() -> Result<PathBuf, String> {
    let base = dirs::data_dir().ok_or_else(|| "could not resolve data dir".to_string())?;
    Ok(base.join("diff").join("state.db"))
}
```

- [ ] **Step 3: Tests del storage**

`src-tauri/tests/storage_test.rs`:
```rust
use diff_lib::storage::Storage;
use tempfile::TempDir;

#[test]
fn record_and_list_roundtrip() {
    let dir = TempDir::new().unwrap();
    let db = dir.path().join("state.db");
    let storage = Storage::open_at(&db).unwrap();

    assert!(storage.list_for_host("local").unwrap().is_empty());

    storage.record_open("local", "/home/u/projects/a").unwrap();
    storage.record_open("host1", "/srv/projects/b").unwrap();
    storage.record_open("local", "/home/u/projects/c").unwrap();

    let local = storage.list_for_host("local").unwrap();
    assert_eq!(local.len(), 2);
    // Most recent first
    assert_eq!(local[0].path, "/home/u/projects/c");

    let host = storage.list_for_host("host1").unwrap();
    assert_eq!(host.len(), 1);
    assert_eq!(host[0].path, "/srv/projects/b");
}

#[test]
fn record_open_updates_timestamp_on_conflict() {
    let dir = TempDir::new().unwrap();
    let storage = Storage::open_at(&dir.path().join("s.db")).unwrap();
    storage.record_open("local", "/x").unwrap();
    let t1 = storage.list_for_host("local").unwrap()[0].last_opened_at;
    std::thread::sleep(std::time::Duration::from_secs(1));
    storage.record_open("local", "/x").unwrap();
    let t2 = storage.list_for_host("local").unwrap()[0].last_opened_at;
    assert!(t2 > t1);
}
```

Para que estos tests compilen, agregar `pub mod storage;` en `src-tauri/src/lib.rs`.

- [ ] **Step 4: Correr tests**

Run: `cargo test -p diff --test storage_test`
Expected: PASS los 2 tests.

### Task A2: Comandos Tauri para storage

**Files:**
- Modify: `src-tauri/src/lib.rs` (agregar comandos + manage del Storage)

- [ ] **Step 1: Agregar `Storage` al estado de la app**

Modificar `src-tauri/src/lib.rs`:

```rust
mod git;
mod remote_backend;
mod review_bridge;
pub mod storage;

use storage::{RecentRepo, Storage};
// ... resto sin cambios
```

En la función `run`, después de crear `AppState`, agregar el manage del Storage. Ojo: `AppState` es per-window, pero `Storage` es global (compartido entre todas las ventanas). Tauri 2 permite `manage` adicionales:

```rust
let storage = Storage::open_default().expect("failed to open state.db");
// ... dentro del builder:
.manage(storage)
.manage(AppState { ... })  // existing
```

- [ ] **Step 2: Agregar comandos Tauri para listar y registrar recents**

Al final de `lib.rs` (o en un módulo nuevo `src-tauri/src/commands.rs` si preferís separar):

```rust
#[tauri::command]
fn list_recent_repos(host: Option<String>, storage: tauri::State<Storage>) -> Result<Vec<RecentRepo>, String> {
    match host {
        Some(h) => storage.list_for_host(&h),
        None => storage.list_all_recent(50),
    }
}

#[tauri::command]
fn record_recent_repo(host: String, path: String, storage: tauri::State<Storage>) -> Result<(), String> {
    storage.record_open(&host, &path)
}
```

Y agregarlos al `invoke_handler`:

```rust
.invoke_handler(tauri::generate_handler![
    git::open_repo,
    git::get_repo_status,
    // ... resto ...
    list_recent_repos,
    record_recent_repo,
])
```

- [ ] **Step 3: Verificar build**

Run: `cargo check --workspace`
Expected: PASS.

---

## Phase B: Lectura de `~/.ssh/config`

### Task B1: Parser de hosts y comando Tauri

**Files:**
- Create: `src-tauri/src/ssh_config.rs`
- Modify: `src-tauri/src/lib.rs` (módulo + comando)

- [ ] **Step 1: Crear `src-tauri/src/ssh_config.rs`**

```rust
use std::path::PathBuf;

use serde::Serialize;
use ssh2_config::{ParseRule, SshConfig};

#[derive(Debug, Serialize, Clone)]
pub struct SshHost {
    pub alias: String,
    pub hostname: Option<String>,
    pub user: Option<String>,
}

/// Read `~/.ssh/config` and return the named host aliases (excluding `*` and
/// other wildcards). The default `Host *` block, if present, is filtered out.
pub fn list_hosts() -> Result<Vec<SshHost>, String> {
    let path = default_config_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }

    let mut reader = std::io::BufReader::new(
        std::fs::File::open(&path).map_err(|e| format!("open ssh config: {e}"))?,
    );
    let config = SshConfig::default()
        .parse(&mut reader, ParseRule::ALLOW_UNKNOWN_FIELDS)
        .map_err(|e| format!("parse ssh config: {e}"))?;

    let mut out = Vec::new();
    for host in config.get_hosts() {
        for pattern in &host.pattern {
            let alias = pattern.pattern.clone();
            // Skip wildcards and the implicit Host * block.
            if alias.contains('*') || alias.contains('?') || alias == "*" {
                continue;
            }
            let params = host.params.clone();
            out.push(SshHost {
                alias,
                hostname: params.host_name.clone(),
                user: params.user.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.alias.cmp(&b.alias));
    out.dedup_by(|a, b| a.alias == b.alias);
    Ok(out)
}

fn default_config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "could not resolve home dir".to_string())?;
    Ok(home.join(".ssh").join("config"))
}
```

(Notar: `ssh2-config` 0.4 puede tener pequeñas diferencias en API. Si la API real difiere, ajustar — la idea es: leer `~/.ssh/config`, obtener lista de Host blocks, filtrar wildcards.)

- [ ] **Step 2: Test del parser**

`src-tauri/tests/ssh_config_test.rs`:
```rust
use diff_lib::ssh_config;

#[test]
fn missing_file_returns_empty() {
    // Set HOME to a tempdir without ~/.ssh/config.
    let dir = tempfile::TempDir::new().unwrap();
    std::env::set_var("HOME", dir.path());
    let hosts = ssh_config::list_hosts().unwrap();
    assert!(hosts.is_empty());
}
```

(Tests más completos del parser dependen de cómo reaccione `ssh2-config` a inputs específicos — agregar fixtures si la primera versión rompe en la práctica.)

- [ ] **Step 3: Comando Tauri**

Agregar a `lib.rs`:

```rust
mod ssh_config;
// ...

#[tauri::command]
fn list_ssh_hosts() -> Result<Vec<ssh_config::SshHost>, String> {
    ssh_config::list_hosts()
}
```

Registrar en `invoke_handler`.

- [ ] **Step 4: Verificar build y tests**

Run: `cargo test -p diff --test ssh_config_test && cargo check --workspace`
Expected: PASS.

---

## Phase C: Multi-window y picker

### Task C0: Refactor de `AppState` a per-window keyed por label

**Files:**
- Modify: `src-tauri/src/git.rs` (la struct `AppState` y todos los shims)

- [ ] **Step 1: Cambiar la struct `AppState`**

```rust
pub struct AppState {
    pub backends: Mutex<HashMap<String, BackendSlot>>,
    pub bridge: Mutex<Option<Child>>,
    pub event_listener: Mutex<Option<JoinHandle<()>>>,
    pub event_listener_stop: Arc<AtomicBool>,
    pub clone_cancels: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

pub struct BackendSlot {
    pub backend: Box<dyn diff_core::GitBackend>,
    pub watcher: diff_core::WatcherHandle,
}
```

(Borrar los campos `backend`, `watcher_handle`, `watcher_generation` que existían en el AppState del Plan 1.)

- [ ] **Step 2: Helper para resolver backend por window**

Agregar a `git.rs`:

```rust
fn with_backend<R>(
    window: &tauri::Window,
    state: &AppState,
    f: impl FnOnce(&dyn diff_core::GitBackend) -> Result<R, BackendError>,
) -> Result<R, String> {
    let label = window.label().to_string();
    let map = state.backends.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    let slot = map.get(&label).ok_or("no repository open in this window")?;
    f(slot.backend.as_ref()).map_err(|e| e.to_string())
}
```

- [ ] **Step 3: Refactorizar cada `#[tauri::command]` para tomar `tauri::Window`**

Ejemplo: `get_repo_status` pasa de:
```rust
pub fn get_repo_status(state: State<AppState>) -> Result<RepoStatus, String>
```
a:
```rust
pub fn get_repo_status(window: tauri::Window, state: State<AppState>) -> Result<RepoStatus, String> {
    with_backend(&window, &state, |b| b.get_status())
}
```

Aplicar el mismo patrón a `stage_file`, `unstage_file`, `stage_all`, `unstage_all`, `discard_file`, `commit`, `get_file_contents_batch`.

- [ ] **Step 4: Refactorizar `install_backend` para guardar en el map**

```rust
fn install_backend(
    app: &AppHandle,
    window: &tauri::Window,
    state: &AppState,
    backend: Box<dyn GitBackend>,
) -> Result<(), String> {
    let label = window.label().to_string();
    let app_clone = app.clone();
    let label_for_emit = label.clone();
    let watcher = backend
        .subscribe_changes(Box::new(move || {
            // Emit only to the specific window that owns this backend.
            if let Some(w) = app_clone.get_webview_window(&label_for_emit) {
                let _ = w.emit("repo:changed", ());
            }
        }))
        .map_err(err_to_string)?;

    let mut map = state.backends.lock().map_err(|e| format!("lock poisoned: {e}"))?;
    map.insert(label, BackendSlot { backend, watcher });
    Ok(())
}
```

- [ ] **Step 5: Refactorizar `open_repo`, `init_repo`, `clone_repo` para tomar `tauri::Window`**

```rust
#[tauri::command]
pub fn open_repo(
    path: String,
    app: AppHandle,
    window: tauri::Window,
    state: State<AppState>,
) -> Result<String, String> {
    let backend = LocalGitBackend::open(Path::new(&path)).map_err(err_to_string)?;
    let workdir = backend.workdir().to_string_lossy().to_string();
    install_backend(&app, &window, &state, Box::new(backend))?;
    Ok(workdir)
}
```

(Mismo patrón para los otros.)

- [ ] **Step 6: Cleanup al cerrar ventana**

Agregar al `RunEvent` handler en `lib.rs`:

```rust
tauri::RunEvent::WindowEvent { label, event, .. } => {
    if let tauri::WindowEvent::Destroyed = event {
        let state: &AppState = app_handle.state::<AppState>().inner();
        if let Ok(mut map) = state.backends.lock() {
            map.remove(&label);
        }
        // If the picker window is destroyed, exit the whole app.
        if label == "picker" {
            app_handle.exit(0);
        }
    }
}
```

- [ ] **Step 7: Actualizar `manage(AppState{...})` en `lib.rs`**

```rust
.manage(AppState {
    backends: Mutex::new(HashMap::new()),
    bridge: Mutex::new(None),
    event_listener: Mutex::new(None),
    event_listener_stop: stop_flag.clone(),
    clone_cancels: Mutex::new(HashMap::new()),
})
```

- [ ] **Step 8: Verificar build (los tests existentes seguirán pasando ya que no usan tauri::Window)**

Run: `cargo check --workspace && cargo test -p diff-core -p diff-agent`
Expected: PASS.

### Task C1: Definir el picker como ventana inicial en `tauri.conf.json`

**Files:**
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Cambiar la ventana por defecto a "picker"**

```jsonc
{
  "$schema": "https://schema.tauri.app/config/2",
  "productName": "diff",
  "version": "0.1.0",
  "identifier": "com.fourcadefran.diff",
  "build": { ... },
  "app": {
    "windows": [
      {
        "label": "picker",
        "title": "diff",
        "url": "index.html?kind=picker",
        "width": 480,
        "height": 600,
        "minWidth": 380,
        "minHeight": 480,
        "center": true,
        "resizable": true
      }
    ],
    "security": {
      "csp": "default-src 'self'; connect-src ipc: http://ipc.localhost http://127.0.0.1:*; style-src 'self' 'unsafe-inline'"
    }
  },
  "bundle": { ... }
}
```

(El `url` con query param `?kind=picker` es lo que el frontend usa para decidir qué renderizar.)

- [ ] **Step 2: Verificar que la app arranca con la ventana del picker (todavía sin contenido)**

Run: `bun run tauri dev`
Expected: ventana 480×600 titulada "diff", contenido React actual (la app de repo) por ahora — eso lo cambiamos en task C2.

### Task C2: Frontend — distinguir picker vs repo window

**Files:**
- Create: `src/lib/window.ts`
- Modify: `src/App.tsx`
- Create: `src/components/picker/picker.tsx`
- Create: `src/components/picker/host-list.tsx`
- Create: `src/components/picker/recent-list.tsx`

- [ ] **Step 1: Helper `src/lib/window.ts` para detectar window kind**

```typescript
export type WindowKind = "picker" | "repo";

export function getWindowKind(): WindowKind {
  const params = new URLSearchParams(window.location.search);
  return params.get("kind") === "picker" ? "picker" : "repo";
}

export function getRepoParams(): { host: string; path: string } | null {
  const params = new URLSearchParams(window.location.search);
  const host = params.get("host");
  const path = params.get("path");
  if (!host || !path) return null;
  return { host, path };
}
```

- [ ] **Step 2: `src/App.tsx` — branch top-level por window kind**

Al inicio de `App.tsx`, antes del componente principal:

```tsx
import { getWindowKind } from "@/lib/window";
import { Picker } from "@/components/picker/picker";

export default function App() {
  const kind = getWindowKind();
  if (kind === "picker") {
    return <Picker />;
  }
  return <RepoApp />;
}

function RepoApp() {
  // mover acá el código actual de App() (todo el contenido del componente actual)
  // ...
}
```

(La conversión es mecánica: renombrar el `App` actual a `RepoApp`, crear nuevo `App` que decide.)

- [ ] **Step 3: `src/components/picker/picker.tsx` — esqueleto del picker**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { HostList } from "./host-list";
import { RecentList } from "./recent-list";
import { Toaster } from "sonner";

export interface SshHost {
  alias: string;
  hostname?: string;
  user?: string;
}

export function Picker() {
  const [sshHosts, setSshHosts] = useState<SshHost[]>([]);
  const [selectedHost, setSelectedHost] = useState<string>("local");

  useEffect(() => {
    invoke<SshHost[]>("list_ssh_hosts")
      .then(setSshHosts)
      .catch((e) => console.error("[diff-picker] list_ssh_hosts:", e));
  }, []);

  return (
    <main className="flex h-dvh bg-background text-foreground">
      <HostList
        sshHosts={sshHosts}
        selected={selectedHost}
        onSelect={setSelectedHost}
      />
      <div className="flex-1 border-l border-border">
        <RecentList host={selectedHost} />
      </div>
      <Toaster />
    </main>
  );
}
```

- [ ] **Step 4: `src/components/picker/host-list.tsx`**

```tsx
import type { SshHost } from "./picker";
import { IconServer, IconHomeBolt } from "@tabler/icons-react";

interface Props {
  sshHosts: SshHost[];
  selected: string;
  onSelect: (host: string) => void;
}

export function HostList({ sshHosts, selected, onSelect }: Props) {
  return (
    <aside className="w-44 flex flex-col gap-1 p-2">
      <button
        type="button"
        onClick={() => onSelect("local")}
        className={`flex items-center gap-2 px-2 py-1.5 rounded-md text-sm ${
          selected === "local" ? "bg-accent" : "hover:bg-accent/50"
        }`}
      >
        <IconHomeBolt className="size-4" />
        Local
      </button>
      <div className="border-t border-border my-1" />
      {sshHosts.length === 0 ? (
        <p className="text-xs text-muted-foreground px-2 py-1">
          No SSH hosts in ~/.ssh/config
        </p>
      ) : (
        sshHosts.map((h) => (
          <button
            key={h.alias}
            type="button"
            onClick={() => onSelect(h.alias)}
            className={`flex items-center gap-2 px-2 py-1.5 rounded-md text-sm ${
              selected === h.alias ? "bg-accent" : "hover:bg-accent/50"
            }`}
          >
            <IconServer className="size-4" />
            <span className="truncate">{h.alias}</span>
          </button>
        ))
      )}
    </aside>
  );
}
```

- [ ] **Step 5: `src/components/picker/recent-list.tsx`**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { IconFolderOpen, IconGitBranch } from "@tabler/icons-react";
import { toast } from "sonner";

interface RecentRepo {
  host: string;
  path: string;
  last_opened_at: number;
}

interface Props {
  host: string;
}

export function RecentList({ host }: Props) {
  const [recents, setRecents] = useState<RecentRepo[]>([]);
  const [remotePathInput, setRemotePathInput] = useState("");

  useEffect(() => {
    invoke<RecentRepo[]>("list_recent_repos", { host })
      .then(setRecents)
      .catch((e) => console.error("[diff-picker] list_recent_repos:", e));
  }, [host]);

  const openRepo = async (path: string) => {
    try {
      await invoke("open_picker_repo", { host, path });
      // Window opens in background; refresh recents.
      const fresh = await invoke<RecentRepo[]>("list_recent_repos", { host });
      setRecents(fresh);
    } catch (e) {
      toast.error(`Open failed: ${e}`);
    }
  };

  const onPickLocal = async () => {
    if (host !== "local") {
      toast.error("Local picker only available for Local host");
      return;
    }
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string") {
      openRepo(selected);
    }
  };

  const onOpenRemote = async () => {
    const path = remotePathInput.trim();
    if (!path) return;
    openRepo(path);
    setRemotePathInput("");
  };

  return (
    <div className="flex flex-col h-full p-3 gap-3">
      <div className="flex gap-2">
        {host === "local" ? (
          <button
            type="button"
            onClick={onPickLocal}
            className="flex items-center gap-2 px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-md hover:opacity-90"
          >
            <IconFolderOpen className="size-4" />
            Open repo…
          </button>
        ) : (
          <div className="flex flex-1 gap-2">
            <input
              type="text"
              placeholder="/path/on/remote/host"
              value={remotePathInput}
              onChange={(e) => setRemotePathInput(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && onOpenRemote()}
              className="flex-1 px-2 py-1.5 text-sm bg-card border border-border rounded-md"
            />
            <button
              type="button"
              onClick={onOpenRemote}
              className="px-3 py-1.5 text-sm bg-primary text-primary-foreground rounded-md hover:opacity-90"
            >
              Open
            </button>
          </div>
        )}
      </div>

      <div className="flex flex-col gap-1 overflow-y-auto">
        <p className="text-xs uppercase tracking-wide text-muted-foreground px-2">
          Recent
        </p>
        {recents.length === 0 ? (
          <p className="text-sm text-muted-foreground px-2 py-1">
            No recent repositories.
          </p>
        ) : (
          recents.map((r) => (
            <button
              key={`${r.host}:${r.path}`}
              type="button"
              onClick={() => openRepo(r.path)}
              className="flex items-center gap-2 px-2 py-1.5 text-sm rounded-md hover:bg-accent text-left"
            >
              <IconGitBranch className="size-4 text-muted-foreground shrink-0" />
              <span className="truncate">{r.path}</span>
            </button>
          ))
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 6: Verificar build del frontend**

Run: `bun run build`
Expected: typecheck pasa, build produce dist/.

### Task C3: Comando `open_picker_repo` que abre una ventana nueva

**Files:**
- Create: `src-tauri/src/window.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Crear `src-tauri/src/window.rs`**

```rust
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

static NEXT_REPO_LABEL: AtomicU64 = AtomicU64::new(1);

pub fn open_repo_window(app: &AppHandle, host: &str, path: &str) -> Result<(), String> {
    let n = NEXT_REPO_LABEL.fetch_add(1, Ordering::SeqCst);
    let label = format!("repo-{n}");

    let title = if host == "local" {
        path.rsplit('/').next().unwrap_or(path).to_string()
    } else {
        format!("{host}:{}", path.rsplit('/').next().unwrap_or(path))
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
```

Agregar `urlencoding = "2"` a `src-tauri/Cargo.toml`.

- [ ] **Step 2: Comando Tauri `open_picker_repo` en `lib.rs`**

```rust
mod window;

#[tauri::command]
fn open_picker_repo(
    host: String,
    path: String,
    app: tauri::AppHandle,
    storage: tauri::State<Storage>,
) -> Result<(), String> {
    storage.record_open(&host, &path)?;
    window::open_repo_window(&app, &host, &path)
}
```

Registrar en `invoke_handler`.

- [ ] **Step 3: Verificar que se puede abrir una ventana de repo desde el picker**

Run: `bun run tauri dev`
Verificar:
- [ ] El picker arranca con tamaño 480×600.
- [ ] Sidebar muestra "Local" + cualquier host de tu `~/.ssh/config`.
- [ ] Click en Local → "Open repo…" → seleccionás un dir → se abre una ventana nueva 1200×800 con la app de repo, y aparece el repo en la lista de Recents al volver al picker.
- [ ] El picker queda abierto detrás de la ventana del repo.

(En este punto, los repos remotos todavía no funcionan — `open_repo` del lado Rust todavía solo abre con `LocalGitBackend`. Eso es la próxima task.)

### Task C4: Wire `RemoteGitBackend` para hosts SSH

**Files:**
- Modify: `src-tauri/src/git.rs` (modificar `open_repo` o agregar `open_remote_repo`)
- Modify: `src-tauri/src/window.rs` (pasar host en URL)
- Modify: `src/App.tsx` (en RepoApp, leer `host`/`path` de query y llamar al comando correcto)

- [ ] **Step 1: Decidir API: cambiar `open_repo` para aceptar host, o agregar `open_remote_repo`**

Mantenemos `open_repo(path)` para compatibilidad y agregamos `open_remote_repo(host, path)` como variante separada. El frontend decide cuál llamar según el `host` de la URL.

- [ ] **Step 2: Agregar `open_remote_repo` en `src-tauri/src/git.rs`**

Agregar al final de `git.rs`:

```rust
use std::process::Command;

#[tauri::command]
pub fn open_remote_repo(
    host: String,
    path: String,
    app: AppHandle,
    window: tauri::Window,
    state: State<AppState>,
) -> Result<String, String> {
    let mut cmd = Command::new("ssh");
    cmd.args([&host, "diff-agent", "--stdio", "--repo", &path]);

    let backend = crate::remote_backend::RemoteGitBackend::spawn(cmd)
        .map_err(|e| format!("ssh spawn failed: {e}"))?;

    // Sanity-check: get_branch is cheap, surfaces "command not found" early.
    if let Err(e) = backend.get_branch() {
        return Err(match e {
            BackendError::Transport(msg) => format!(
                "Could not connect to diff-agent on {host}. \
                 Install on the host with: \
                 git clone https://github.com/fourcadefran/diff && cd diff && cargo build --release --bin diff-agent\n\nDetails: {msg}"
            ),
            other => other.to_string(),
        });
    }

    install_backend(&app, &window, &state, Box::new(backend))?;
    Ok(format!("{host}:{path}"))
}
```

Registrar `open_remote_repo` en el `invoke_handler` de `lib.rs`.

- [ ] **Step 3: Frontend — `src/lib/tauri.ts` agrega wrapper**

```typescript
export function openRemoteRepo(host: string, path: string): Promise<string> {
  return invoke<string>("open_remote_repo", { host, path });
}
```

- [ ] **Step 4: `src/App.tsx` — RepoApp lee `host`/`path` de URL y abre con el comando correcto**

En el effect de mount de `RepoApp`, en lugar de `getLaunchPath`, usar `getRepoParams`:

```tsx
import { getRepoParams } from "@/lib/window";
import { openRemoteRepo } from "@/lib/tauri";

// ... dentro de RepoApp:
useEffect(() => {
  const params = getRepoParams();
  if (!params) return;
  if (params.host === "local") {
    open(params.path).catch((e) => toast.error(`Open failed: ${e}`));
  } else {
    openRemoteRepo(params.host, params.path).catch((e) =>
      toast.error(`Connect failed: ${e}`)
    );
  }
}, [open]);
```

(El `open` ya existente es el wrapper de `openRepo` para repos locales.)

- [ ] **Step 5: Test manual end-to-end con tu PC host**

Pre-requisitos:
- Tu PC host tiene `diff` clonado y `cargo build --release --bin diff-agent` corrido. El binario está en `~/.cargo/bin/diff-agent` o en el PATH del shell SSH.
- `~/.ssh/config` en la Mac tiene una entrada para tu host.
- `ssh <host>` desde la terminal funciona sin pedir password (key auth).

Run en la Mac: `bun run tauri dev`
Verificar:
- [ ] El picker arranca, el sidebar lista tu host SSH.
- [ ] Seleccionás el host, escribís el path absoluto a un repo en el host, click Open.
- [ ] Se abre una ventana nueva titulada `<host>:<repo-name>`.
- [ ] La ventana muestra el status del repo remoto (staged/unstaged/untracked).
- [ ] Stage / unstage / commit funcionan y se reflejan en `git log` del host.
- [ ] Modificar un archivo en el host (vía otra terminal SSH) dispara refresh en la app.

### Task C5: Manejo de "agente no encontrado" con UX clara

**Files:**
- Modify: `src/App.tsx` (RepoApp) — toast con mensaje específico para `agent-not-installed`

- [ ] **Step 1: Detectar el error de "command not found" en el frontend y mostrar instrucciones**

En el `catch` del `openRemoteRepo`:

```tsx
openRemoteRepo(params.host, params.path).catch((e) => {
  const msg = String(e);
  if (msg.includes("Could not connect to diff-agent")) {
    toast.error(msg, { duration: 30000 });
  } else {
    toast.error(`Connect failed: ${msg}`);
  }
});
```

(El `Result<...>` del comando Tauri ya viene con el mensaje completo formateado en la lógica del Step 2 de C4.)

- [ ] **Step 2: Verificar smoke**

Si tu host **no** tiene el agente instalado:
Run: `bun run tauri dev`, intentar abrir un repo remoto.
Expected: toast con el mensaje de instalación visible 30s, ventana del repo vacía o con error.

---

## Phase D: CLI args para `diff <host>:<path>`

### Task D1: Parseo de `host:path` en main.rs

**Files:**
- Modify: `src-tauri/src/main.rs`

- [ ] **Step 1: Reemplazar el parseo actual**

```rust
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--mcp" || a == "-m") {
        return run_mcp_mode();
    }

    if let Some(arg) = args.iter().find(|a| !a.starts_with('-')) {
        if let Some((host, path)) = parse_host_path(arg) {
            // Remote repo: store as "host:path" launch spec.
            diff_lib::set_launch_spec(diff_lib::LaunchSpec::Remote {
                host: host.to_string(),
                path: path.to_string(),
            });
        } else {
            match std::fs::canonicalize(arg) {
                Ok(abs) => diff_lib::set_launch_spec(diff_lib::LaunchSpec::Local(abs)),
                Err(_) => eprintln!("[diff] could not resolve path: {arg}"),
            }
        }
    }

    diff_lib::run();
    ExitCode::SUCCESS
}

fn parse_host_path(arg: &str) -> Option<(&str, &str)> {
    // Match "host:/abs/path" but NOT "/abs/path" or "C:/Users/...".
    // Heuristic: there's a colon, and what's before the colon doesn't contain '/'.
    let (head, tail) = arg.split_once(':')?;
    if head.is_empty() || head.contains('/') || head.contains('\\') {
        return None;
    }
    if tail.is_empty() {
        return None;
    }
    Some((head, tail))
}
```

- [ ] **Step 2: Cambiar `LAUNCH_PATH` a `LAUNCH_SPEC` en `lib.rs`**

```rust
use std::sync::OnceLock;

#[derive(Clone)]
pub enum LaunchSpec {
    Local(std::path::PathBuf),
    Remote { host: String, path: String },
}

static LAUNCH_SPEC: OnceLock<LaunchSpec> = OnceLock::new();

pub fn set_launch_spec(spec: LaunchSpec) {
    let _ = LAUNCH_SPEC.set(spec);
}

#[tauri::command]
fn get_launch_spec() -> Option<serde_json::Value> {
    LAUNCH_SPEC.get().map(|spec| match spec {
        LaunchSpec::Local(p) => serde_json::json!({ "kind": "local", "path": p.to_string_lossy() }),
        LaunchSpec::Remote { host, path } => {
            serde_json::json!({ "kind": "remote", "host": host, "path": path })
        }
    })
}
```

Reemplazar el comando `get_launch_path` viejo por `get_launch_spec` en el `invoke_handler`.

- [ ] **Step 3: Modificar el `setup` de Tauri para que, si hay LaunchSpec, abra una ventana de repo además del picker**

En el `.setup` callback de `lib.rs`:

```rust
.setup(|app| {
    let state = app.state::<AppState>();
    match review_bridge::start_review_server(state.inner()) {
        Ok(port) => { ... existing ... }
        Err(e) => eprintln!("..."),
    }

    // If launched with a path, open the repo window in addition to the picker.
    if let Some(spec) = LAUNCH_SPEC.get() {
        match spec {
            LaunchSpec::Local(path) => {
                let _ = window::open_repo_window(app.handle(), "local", &path.to_string_lossy());
            }
            LaunchSpec::Remote { host, path } => {
                let _ = window::open_repo_window(app.handle(), host, path);
            }
        }
    }

    Ok(())
})
```

- [ ] **Step 4: Frontend — `src/lib/tauri.ts` actualizar wrapper**

```typescript
export type LaunchSpec =
  | { kind: "local"; path: string }
  | { kind: "remote"; host: string; path: string };

export function getLaunchSpec(): Promise<LaunchSpec | null> {
  return invoke<LaunchSpec | null>("get_launch_spec");
}
```

(Y borrar el `getLaunchPath` viejo. Cualquier referencia a `getLaunchPath` en el código frontend pasa a usar `getLaunchSpec`. Como ahora la lógica de "qué repo abrir" la decide la URL de la ventana — establecida por el `setup` de Rust — el código de `App.tsx` que leía `getLaunchPath` ya no es necesario en RepoApp; queda obsoleto y puede borrarse.)

- [ ] **Step 5: Verificar**

Run: `bun run tauri dev -- --release` no, mejor: build local y probar.

```bash
bun run tauri build --debug
./src-tauri/target/debug/diff /tmp/somerepo
./src-tauri/target/debug/diff myhost:/srv/projects/foo
```

Expected: en cada caso, arranca el picker + se abre la ventana correspondiente al repo indicado.

---

## Phase E: Cierre

### Task E1: Verificación final de Plan 3

- [ ] **Step 1: Workspace test full**

Run: `cargo test --workspace`
Expected: PASS todo (incluye storage, ssh_config, agent e2e, remote backend e2e).

- [ ] **Step 2: Build completo**

Run: `bun run tauri build`
Expected: produce el `.app` / `.dmg` para macOS.

- [ ] **Step 3: Smoke manual completo**

- [ ] Picker arranca solo (sin args), 480×600.
- [ ] Sidebar lista Local + hosts SSH.
- [ ] Recents persisten entre arranques (cerrar app, reabrir, recents siguen ahí).
- [ ] Open de un repo local abre ventana nueva, picker queda detrás.
- [ ] Open de un repo remoto vía SSH funciona end-to-end.
- [ ] CLI: `diff /path/local` abre picker + ventana del repo local.
- [ ] CLI: `diff host:/remote/path` abre picker + ventana del repo remoto.
- [ ] Cerrar la ventana del picker mata la app entera.
- [ ] Cerrar una ventana de repo no afecta a las otras.

### Task E2: Cleanup de código obsoleto

- [ ] **Step 1: Eliminar `getLaunchPath` viejo del frontend si quedó alguna referencia**

Run: `grep -r "getLaunchPath" src/`
Expected: vacío (o borrarlas).

- [ ] **Step 2: Verificar que no quedan ramas dead-code en `App.tsx` (e.g. el effect viejo de restore)**

El localStorage `diff:last-opened-repo` ya no es necesario porque la persistencia ahora vive en SQLite. Decidir: borrar el código del localStorage en `use-repo-status.ts` y `use-recent-repos.ts`, o dejarlo (no molesta). Recomendación: borrarlo para reducir code paths.

- [ ] **Step 3: Actualizar `onboarding-prompt.md` o eliminarlo**

`onboarding-prompt.md` es un doc histórico de cómo se generó el componente original de Onboarding (que sigue existiendo, ahora usado solo dentro de RepoApp cuando no hay workdir, o quizás reemplazado por ir directo al picker). Decidir si mantenerlo. Recomendación: mantener pero anotar que el flujo de entrada ahora es el picker, no Onboarding.

---

## Self-Review Checklist (al cerrar el plan)

- [ ] El picker es la ventana inicial single-instance.
- [ ] Cada repo abierto va a su propia ventana programática.
- [ ] La SQLite persiste recents por host y los rankea por `last_opened_at`.
- [ ] La lectura de `~/.ssh/config` solo lista hosts con nombre (no wildcards).
- [ ] `RemoteGitBackend` funciona end-to-end contra un host SSH real.
- [ ] El error "agent-not-installed" muestra instrucciones claras al usuario.
- [ ] CLI args `diff <local-path>` y `diff <host>:<path>` funcionan.
- [ ] Toda la suite de tests del workspace pasa.
