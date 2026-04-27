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
