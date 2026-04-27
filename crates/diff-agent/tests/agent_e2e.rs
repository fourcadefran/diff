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
    Command::new("git").args(["init", "-q"]).current_dir(path).status().unwrap();
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

#[test]
fn agent_unknown_method_returns_error() {
    let dir = init_test_repo();
    let agent = AgentHandle::spawn(dir.path());
    let resp = agent.call("nonexistent", json!({}));
    assert_eq!(resp["error"]["code"], -32601);
    assert!(resp["error"]["message"].as_str().unwrap().contains("nonexistent"));
    agent.shutdown();
}
