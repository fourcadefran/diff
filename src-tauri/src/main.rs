// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::{Command, ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--mcp" || a == "-m") {
        return run_mcp_mode();
    }

    // First non-flag positional = launch path
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
