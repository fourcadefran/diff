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
            if alias.contains('*') || alias.contains('?') {
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
