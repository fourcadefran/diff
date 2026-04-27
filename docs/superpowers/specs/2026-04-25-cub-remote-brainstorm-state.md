---
title: Brainstorm en curso — fork de cub.dev con soporte local+remoto
date_started: 2026-04-25
status: brainstorming-in-progress
upstream_repo: github.com/ephraimduncan/cub-dev (a forkear)
---

# Handoff de sesión

Este archivo es un dump del estado de un brainstorming que arrancó en una sesión anterior de Claude Code, en el repo upstream `cub.dev` (antes del fork). Cuando lo leas en una sesión nueva, retomá desde **"Próximas preguntas abiertas"** sin volver a hacer las preguntas que ya están respondidas.

## Contexto del usuario

- Trabaja desde una **Mac** y se conecta por **SSH a una PC host** (Linux/WSL2) donde viven sus repos.
- Usa la app **cub.dev** (Tauri + React + TS, backend Rust en `src-tauri/`) para gestionar el versionado.
- Quiere poder controlar el versionado de los repos del host **desde la Mac**, sin perder la posibilidad de operar también en repos locales de la Mac.

## Idea original (antes de pivotar)

El usuario arrancó pidiendo dos cosas:
1. Un **listado de PRs abiertas** dentro de la app (no existe hoy: ninguna integración con GitHub).
2. Un **icono de escritorio** para abrir la app sin tener que correr el comando.

Después pivoteó a una idea más ambiciosa que **subsume** la del icono (porque el "abrir la app" pasa a ser parte del flujo distribuido) y **deja para fase posterior** el listado de PRs.

## Decisiones tomadas

### 1. Arquitectura: híbrido local + remoto (opción C)
La app debe soportar repos locales **y** remotos en paralelo (puede haber un repo local abierto al mismo tiempo que uno remoto). No son modos exclusivos.

Esto implica:
- Refactor de `src-tauri/src/git.rs` detrás de una capa de transporte abstracta (los comandos git invocan un trait/interface, con dos implementaciones: local in-process, y remota vía RPC al agente del host).
- Un proceso `cub agent` (headless) que corre en el host y expone los mismos comandos sobre algún transporte (HTTP/WebSocket sobre túnel SSH — a definir).
- La GUI Tauri puede correr local (Mac) y operar tanto sobre repos locales como remotos.

### 2. Modelo de ventanas (opción 2)
**Múltiples ventanas, una por repo.** Cada vez que se abre un repo (local o remoto) se abre una ventana nueva, independiente. Como VS Code o iTerm. Permite tenerlas lado a lado físicamente.

### 3. Modelo de conexión (opción 1, evolucionar a 3)
Empezar con **lectura de `~/.ssh/config`** de la Mac: la app muestra los hosts ya configurados ahí y reusa claves/ProxyJump existentes. **Más adelante** sumar entrada manual ("Add custom host…") para hosts que no estén en `~/.ssh/config`.

### 4. Forkear el proyecto
El cambio es estructural y no va a entrar upstream. El usuario va a:
- Forkear `github.com/ephraimduncan/cub-dev` a su cuenta (`fourcadefran`).
- Renombrar: nuevo `productName`, nuevo `identifier` (`com.ephraimduncan.cub-dev` → `com.fourcadefran.<nombre-nuevo>`), su propio icono, su propio canal de releases.
- Dejar `upstream` como remoto secundario para cherry-pickear bugfixes puntuales.

### 5. Listado de PRs → fase posterior
Se planifica como sub-proyecto separado, después de que el soporte local+remoto esté funcionando.

## Próximas preguntas abiertas (retomar acá)

El brainstorming quedó pausado antes de cubrir varios puntos importantes. En la sesión nueva, hacer **una pregunta por mensaje**, multiple-choice cuando se pueda, en este orden:

1. **Ciclo de vida del agente en el host** — ¿se auto-instala/actualiza desde la Mac (à la VS Code Remote: la primera vez que conectás, te instala el binario por SSH), se instala manualmente con un comando (`curl … | sh`), o se levanta on-demand (la Mac abre SSH y ejecuta `cub agent --stdio` por la sesión)?
2. **Transporte** — HTTP sobre túnel SSH local / WebSocket sobre túnel SSH / stdio sobre la sesión SSH (lo más simple, sin puertos). Tradeoffs: stdio es más simple y no requiere puertos libres, pero limita streaming; HTTP/WS son más flexibles pero requieren puertos.
3. **Auth y credenciales** — ¿dónde viven los tokens (GitHub, etc.)? Probablemente en el host (donde está el repo), pero la Mac los necesita para operaciones desde la GUI. ¿Forwarding del SSH agent? ¿Almacenamiento en el host con la GUI sin acceso directo?
4. **Flujo de primer arranque (launch UX)** — `cub` desde la terminal, ¿qué muestra?: ¿un picker de "Local / host1 / host2" → repo? ¿O `cub` abre lo último, y `cub --pick` muestra el picker?
5. **Persistencia de estado** — recents (último repo por host), preferencias por host, sesión SSH cacheada (multiplexing) o reconexión cada vez.
6. **Distribución** — Mac `.app` (con DMG firmado), agente del host como binario standalone (Linux x86_64/arm64). ¿Auto-update del cliente? ¿Y del agente?
7. **Refactor del transporte** — pre-trabajo concreto: extraer las funciones de `src-tauri/src/git.rs` detrás de un trait `GitBackend` (o equivalente), con `LocalGitBackend` (lo que hay hoy) y `RemoteGitBackend` (cliente RPC al agente). Los comandos de Tauri delegan al backend del repo abierto.
8. **Icono / branding del fork** — nombre nuevo, icono nuevo, bundle id nuevo. Probablemente conviene resolver esto cerca del final, no al principio.

Una vez cerradas estas, se escribe el spec en `docs/superpowers/specs/2026-04-25-cub-remote-design.md` (o la fecha del día en que se cierre) y se pasa al skill de `writing-plans`.

## Cómo retomar en la sesión nueva

1. Cloná el fork.
2. Copiá este archivo a `docs/superpowers/specs/2026-04-25-cub-remote-brainstorm-state.md` (creá el directorio si no existe) y commiteá.
3. Abrí Claude Code en la raíz del fork.
4. Pegale al asistente algo así:
   > "Estoy retomando un brainstorming que arrancó en otra sesión. Leé `docs/superpowers/specs/2026-04-25-cub-remote-brainstorm-state.md` y seguí desde 'Próximas preguntas abiertas' sin re-preguntar lo ya decidido."
5. El asistente debería invocar el skill `superpowers:brainstorming` y arrancar por la pregunta 1 de la lista.
