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
