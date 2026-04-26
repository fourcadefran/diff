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
