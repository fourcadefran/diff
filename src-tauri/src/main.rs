// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--mcp" || a == "-m") {
        return run_mcp_mode();
    }

    // First non-flag positional: either `<host>:<path>` (remote) or `<local-path>`.
    if let Some(arg) = args.iter().find(|a| !a.starts_with('-')) {
        match parse_host_path(arg) {
            Some((host, path)) => {
                diff_lib::set_launch_spec(diff_lib::LaunchSpec::Remote {
                    host: host.to_string(),
                    path: path.to_string(),
                });
            }
            None => match std::fs::canonicalize(arg) {
                Ok(abs) => diff_lib::set_launch_spec(diff_lib::LaunchSpec::Local(abs)),
                Err(_) => eprintln!("[diff] could not resolve path: {arg}"),
            },
        }
    }

    diff_lib::run();
    ExitCode::SUCCESS
}

/// Match "host:/abs/path" but NOT "/abs/path" or "C:/Users/..." (the latter
/// has '/' in the head which we reject).
fn parse_host_path(arg: &str) -> Option<(&str, &str)> {
    let (head, tail) = arg.split_once(':')?;
    if head.is_empty() || head.contains('/') || head.contains('\\') {
        return None;
    }
    if tail.is_empty() {
        return None;
    }
    Some((head, tail))
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
