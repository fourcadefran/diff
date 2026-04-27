---
title: diff — soporte para repos remotos vía SSH
date: 2026-04-25
status: design-approved
brainstorm_source: 2026-04-25-cub-remote-brainstorm-state.md
---

# diff — soporte para repos remotos vía SSH

## 1. Visión y alcance

**diff** es un fork de [cub-dev](https://github.com/ephraimduncan/cub-dev) (Tauri + React + Rust con `git2`) que agrega soporte para operar repos en hosts remotos vía SSH, en paralelo con repos locales. Pensado para uso personal de un solo usuario (`fourcadefran`) que trabaja desde una Mac y tiene sus repos en una PC host (Linux/WSL2).

El fork ya existe en `github.com/fourcadefran/diff` con `upstream = ephraimduncan/cub-dev` configurado para cherry-pickear bugfixes puntuales.

### En alcance

- Refactor del backend Rust detrás de un trait `GitBackend` con dos implementaciones (local in-process, remoto vía RPC sobre SSH).
- Binario standalone `diff-agent` que corre en el host y expone `LocalGitBackend` por stdio.
- GUI Tauri con modelo multi-ventana: una ventana picker + una ventana por repo abierto.
- Lectura de `~/.ssh/config` de la Mac para enumerar hosts disponibles.
- Persistencia local en la Mac (SQLite) de repos recientes por host.
- Rename completo del producto: `cub-dev` → `diff`, `com.ephraimduncan.cub-dev` → `com.fourcadefran.diff`.

### Fuera de alcance (sub-proyectos posteriores con su propio brainstorming)

- **Diff entre branches** y vista comparativa.
- **Listado de PRs** dentro de la app.
- **Reviews de PRs.**
- Auto-instalación del agente desde la Mac (estilo VS Code Remote / JetBrains Gateway). v1 usa `cargo build` manual en el host.
- Auto-update del cliente o del agente. v1 distribuye con `bun tauri build` local.
- Code signing / notarización del `.app` macOS.
- Multiplexing de sesiones SSH (ControlMaster). v1 usa una sesión por ventana.
- Custom icon y branding visual (más allá del rename de strings). Se mantienen los íconos heredados de cub-dev.
- "Add custom host" UI (hosts fuera de `~/.ssh/config`).
- Soporte para repos remotos en el `review_bridge` / sidecar MCP. Se mantienen local-only.
- Gestión de credenciales en la GUI (tokens, SSH keys). El agente hereda las del host.

## 2. Arquitectura

### Modelo de procesos

- **GUI Tauri** (Mac): un proceso por ventana. La primera ventana al arrancar `diff` es el **picker**; cada repo abierto desde el picker spawnea su propia ventana de repo (proceso independiente).
- **Agente** (`diff-agent`): un proceso por ventana de repo remoto. Vive en el host. Lo lanza la GUI con `ssh <host> diff-agent --stdio --repo <path>`. Vida y muerte atadas a la sesión SSH.
- **Repos locales:** la ventana de repo usa `LocalGitBackend` directamente, sin proceso intermedio.

### Workspace Cargo (tres crates)

```
diff/
  crates/
    diff-core/       # GitBackend trait, LocalGitBackend, watcher, protocol
      src/
        lib.rs
        local.rs
        watcher.rs
        protocol.rs
    diff-agent/      # binario diff-agent: loop stdio JSON-RPC sobre LocalGitBackend
      src/main.rs
  src-tauri/         # GUI Tauri: comandos shim + RemoteGitBackend + picker
    src/
      git.rs         # solo Tauri command shims que delegan al backend
      remote_backend.rs
      lib.rs
      main.rs
      review_bridge.rs   # sin cambios (local-only)
  src/               # React (frontend)
```

Reglas de dependencia:
- `diff-core` no depende de Tauri ni de UI.
- `diff-agent` solo depende de `diff-core`.
- `src-tauri` depende de `diff-core` + Tauri.

Esto permite que el host buildee `cargo build --release --bin diff-agent` (tras `git pull` del repo) sin instalar el toolchain ni dependencias de Tauri.

### Trait central: `GitBackend`

Sync, definido en `diff-core`:

```rust
pub trait GitBackend: Send {
    fn get_status(&self) -> Result<RepoStatus, BackendError>;
    fn get_file_contents_batch(&self, reqs: Vec<FileContentsRequest>) -> Result<Vec<FileContentsBatchItem>, BackendError>;
    fn stage_file(&self, path: &str) -> Result<(), BackendError>;
    fn unstage_file(&self, path: &str) -> Result<(), BackendError>;
    fn stage_all(&self) -> Result<(), BackendError>;
    fn unstage_all(&self) -> Result<(), BackendError>;
    fn commit(&self, msg: &str, amend: bool) -> Result<String, BackendError>;
    fn discard_file(&self, path: &str) -> Result<(), BackendError>;
    fn get_branch(&self) -> Result<Option<String>, BackendError>;
    fn subscribe_changes(&self, sink: Box<dyn Fn() + Send>) -> Result<WatcherHandle, BackendError>;
}
```

Razones del diseño sync:
- Los comandos Tauri actuales (`pub fn`) son sync. Mantenerlo sync evita arrastrar `tokio` + `async-trait`.
- Cada ventana es su propio proceso → no hay contención cross-window sobre el backend.
- El `RemoteGitBackend` mantiene `Mutex<BufReader<ChildStdout>>` + `Mutex<ChildStdin>` y bloquea por call. Aceptable porque solo una ventana habla con cada backend.

### Funciones top-level (no requieren repo abierto)

`open_repo`, `clone_repo`, `init_repo`, `cleanup_path`, `get_repo_branch(path)` no necesitan un backend abierto. Quedan como funciones libres en `diff-core` (`pub fn`) y se invocan desde el shim Tauri o desde el agente según corresponda. Estructura específica a definir en el plan de implementación.

### Implementaciones del trait

- **`LocalGitBackend`** (en `diff-core/src/local.rs`): wrapping de `git2::Repository` + watcher con `notify`. Es esencialmente la lógica actual de `src-tauri/src/git.rs` reorganizada detrás del trait.
- **`RemoteGitBackend`** (en `src-tauri/src/remote_backend.rs`): mantiene un `Child` de `ssh` (con stdin/stdout piped) + el reader/writer thread-safes. Cada llamada al trait serializa un Request JSON-RPC y bloquea hasta la Response. Un thread separado lee notificaciones del agente y dispara el callback de `subscribe_changes`.

### Cambios al `AppState`

Pasa de:

```rust
pub struct AppState {
    pub repo: Mutex<Option<Repository>>,
    // ...
}
```

a:

```rust
pub struct AppState {
    pub backend: Mutex<Option<Box<dyn GitBackend>>>,
    // ...
}
```

El `bridge`, `event_listener`, `clone_cancels`, y `watcher` de `review_bridge` se mantienen sin cambios (siguen siendo locales al proceso GUI).

## 3. Protocolo stdio (RPC entre GUI y agente)

### Formato

Line-delimited JSON-RPC 2.0 (un mensaje por línea, terminada en `\n`). Cada mensaje es JSON válido sin saltos de línea literales (los strings JSON los escapean).

### Tipos de mensaje

```jsonc
// GUI → agente: pedir una operación
{"jsonrpc":"2.0","id":42,"method":"get_status","params":{}}

// agente → GUI: respuesta a una request
{"jsonrpc":"2.0","id":42,"result":{"staged":[...],"unstaged":[...],"untracked":[...]}}
{"jsonrpc":"2.0","id":42,"error":{"code":-32000,"message":"no repository open"}}

// agente → GUI: notificación push (sin id, no espera response)
{"jsonrpc":"2.0","method":"repo:changed","params":{}}
```

### Mapeo método ↔ trait

Los nombres de método en el protocolo coinciden 1:1 con los métodos del trait `GitBackend` (`get_status`, `stage_file`, `commit`, etc.). Los `params` son los argumentos del método como objeto JSON. Los structs `RepoStatus`, `FileContentsRequest`, `FileContentsBatchItem`, etc., se mueven a `diff-core` con `Serialize`/`Deserialize` para usarse como payloads del protocolo.

### Notificaciones (watcher)

1. La GUI llama `backend.subscribe_changes(callback)` al abrir el repo.
2. `LocalGitBackend` arma un `notify::Watcher` y dispara `callback()` ante cambios relevantes.
3. `RemoteGitBackend` registra el callback y arranca un thread que lee el stdout del agente; cuando ve un mensaje sin `id` con `method: "repo:changed"`, invoca el callback.
4. El callback (mismo en ambos casos) emite el evento Tauri `repo:changed` al frontend, que ya sabe re-pedir el status.

### Loop principal del agente

`diff-agent --stdio --repo <path>`:

1. Parsea args. Abre el repo con `LocalGitBackend::open(path)` o falla con error en stderr y exit code distinto de cero.
2. Llama `subscribe_changes(|| emit_notification("repo:changed"))`.
3. Loop: `read_line(stdin) → parse Request → dispatch al trait → write_line(stdout, Response)`. EOF en stdin → exit limpio.

### Lifecycle de la sesión SSH

- La GUI spawnea `Command::new("ssh").args(["<host>", "diff-agent", "--stdio", "--repo", "<path>"])` con `stdin/stdout = piped`.
- Si SSH o el agente mueren, la GUI lo detecta (broken pipe en la próxima call) y muestra error en la UI con un botón "reconectar".
- Al cerrar la ventana, la GUI mata el `Child` (que cierra SSH, que termina al agente).

### Errores de instalación

Si `ssh host diff-agent ...` falla con exit code 127 o stderr "command not found", la GUI muestra el mensaje:

> El agente no está instalado en `<host>`. Conectate por SSH y corré:
>
> `git clone https://github.com/fourcadefran/diff && cd diff && cargo build --release --bin diff-agent`

## 4. UX

### Picker (ventana especial)

- Tamaño: ~480×600, single-instance (si ya está abierta, traer al frente).
- Layout: sidebar izquierdo con lista de hosts; panel derecho con recents del host seleccionado.
- Hosts listados:
  - `Local` (siempre arriba, separado).
  - Cada `Host` definido en `~/.ssh/config` de la Mac (parseado read-only con `ssh2-config` o crate equivalente).
- Recents del panel derecho: filtrados por host, ordenados por `last_opened_at` desc.
- Acciones: `Open repo…` (file picker para local; input de path manual para hosts remotos), `Clone…`, `Init…`, click en un recent para abrir.
- Foco inicial: el último repo abierto de cualquier host (si hay).

### Apertura de ventanas de repo

- Cada vez que se elige un repo en el picker → la GUI abre una ventana nueva (programáticamente, vía `tauri::WebviewWindowBuilder`). El picker queda detrás (no se cierra).
- La ventana del repo es la UI actual (componentes React existentes).
- Title de la ventana: `<host>:<repo-name>` para remotos, `<repo-name>` para locales.
- Si se cierran todas las ventanas de repo, queda solo el picker. Si se cierra el picker también → la app sale.

### Args de CLI

- `diff` → solo abre el picker.
- `diff <local-path>` → abre el picker + una ventana del repo en `<local-path>`.
- `diff <host>:<remote-path>` → abre el picker + una ventana del repo remoto. `<host>` debe existir en `~/.ssh/config`.

### Persistencia (SQLite en la Mac)

Ubicación: `~/Library/Application Support/diff/state.db`.

Crate: `rusqlite` con feature `bundled` (en `src-tauri`, no en `diff-core`).

Schema v1:

```sql
CREATE TABLE repos (
    host TEXT NOT NULL,            -- "local" o el alias de ~/.ssh/config
    path TEXT NOT NULL,            -- absolute path en ese host
    last_opened_at INTEGER NOT NULL,
    PRIMARY KEY (host, path)
);
```

### Credenciales

Sin código nuevo. El agente hereda el entorno SSH del host (sus `~/.ssh/`, `~/.gitconfig`, `gh auth`). Cuando se implementen push/pull (no en este spec), usarán lo que el host tenga configurado.

## 5. Manejo de errores y testing

### `BackendError`

Definido en `diff-core`:

```rust
pub enum BackendError {
    Git(String),                          // errores de git2 (workdir, OID, etc.)
    Transport(String),                    // broken pipe, EOF inesperado, JSON malformado
    Protocol { code: i32, message: String },  // errores devueltos por el agente
}
```

El shim Tauri serializa a `String` antes de devolver al frontend (mantiene compatibilidad con la API actual `Result<T, String>`).

### Casos remotos

- **SSH cae a mitad de operación:** próxima call → `Transport`. UI muestra toast "conexión perdida" + botón "reconectar". Reconectar = re-spawn del child SSH y re-suscribir al watcher.
- **Agente no instalado:** detectado por exit code 127 o stderr "command not found". Variante específica `Transport("agent-not-installed")`. UI muestra el mensaje de instrucciones.

### Testing

- **`diff-core`:** tests unitarios de `LocalGitBackend` contra repos temporales (`tempfile::TempDir`). Cubren la lógica core, que es la misma que ejercita el `RemoteGitBackend` vía RPC.
- **Protocolo:** tests de roundtrip serde de cada `Request` / `Response` / `Notification`.
- **`RemoteGitBackend` end-to-end:** test que spawna `diff-agent --stdio --repo <tmpdir>` como subprocess local (no SSH) y ejerce las operaciones contra un `RemoteGitBackend` apuntando a ese subproceso. Valida el protocolo completo sin depender de un host real.
- **GUI:** sin tests automatizados nuevos (no hay infra hoy en cub-dev). Verificación manual de cada feature.

## 6. Distribución

- **Cliente Mac (GUI):** `bun tauri build` localmente cuando termine una feature → arrastrar el `.dmg` resultante a `/Applications`. Sin firma, sin notarización (la primera vez click derecho → Open → Open).
- **Agente del host:** el host ya tiene Rust instalado. Para actualizar:
  ```sh
  ssh <host>
  cd ~/path/to/diff
  git pull
  cargo build --release --bin diff-agent
  ```
- **Sin CI**, sin GitHub Releases, sin auto-update. Único usuario, controla manualmente cuándo actualiza.
- El binario `diff-agent` debe quedar en una ubicación que esté en el `PATH` del usuario en el host (típicamente `~/.cargo/bin/` si se usa `cargo install --path crates/diff-agent`, o el `target/release/diff-agent` agregado al PATH).

## 7. Orden de implementación (preview)

El plan detallado lo arma el skill `writing-plans` después de aprobar este spec. Preview de pasos:

1. **Rename `cub-dev` → `diff`** en todo el repo (`package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` con `productName` + `identifier` + window title, `cub_dev_lib` → `diff_lib`, paths `~/.cub` → `~/.diff` en `review_bridge`). Commit aislado.
2. **Reestructurar workspace:** crear `crates/diff-core/`, mover `git.rs` y `watcher.rs` adentro, dejar `src-tauri` consumiéndolo. App debe seguir funcionando idéntica.
3. **Definir `GitBackend` trait** y refactorizar el contenido movido para implementarlo como `LocalGitBackend`. Comandos Tauri pasan a delegar al trait. Verificación: la app sigue funcionando para repos locales.
4. **Definir el protocolo y crear `crates/diff-agent/`** con su loop stdio. Tests unitarios del protocolo y del agente.
5. **Implementar `RemoteGitBackend`** y verificar end-to-end contra `diff-agent` corriendo localmente como subprocess.
6. **Multi-window + picker:** convertir la app actual en "ventana de repo", crear el picker como ventana separada, persistencia SQLite de recents.
7. **Conectar via SSH real:** integrar lectura de `~/.ssh/config` en el picker, spawn de `ssh host diff-agent`, end-to-end con la PC host del usuario.

Cada paso deja la app en estado funcional para el caso local. El soporte remoto se enciende en el paso 5 (contra subprocess) y queda end-to-end en el paso 7.
