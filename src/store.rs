use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Clone)]
pub struct Saved {
    pub name: String,
    #[serde(default)]
    pub auto_named: bool,
    pub cwd: PathBuf,
    pub claude_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub usage: crate::session::Usage,
    #[serde(default)]
    pub style: Option<String>,
    #[serde(default)]
    pub ctx_used: u64,
    #[serde(default)]
    pub ctx_window: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Only in reload snapshots: a still-running `claude` to reattach to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live: Option<Live>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Live {
    pub handoff: crate::claude::Handoff,
    pub busy: bool,
    pub turn_ms: u64,
    #[serde(default)]
    pub permissions: Vec<crate::session::Permission>,
    #[serde(default)]
    pub queue: Vec<(String, serde_json::Value)>,
    #[serde(default)]
    pub claude_id: Option<String>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn config_root() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config"))
}

pub fn config_dir() -> PathBuf {
    config_root().join("den")
}

/// den used to be called ctui: move its settings over once. A reload snapshot
/// written by a still-running ctui is picked up even after the move.
pub fn migrate_legacy() {
    let old = config_root().join("ctui");
    let new = config_dir();
    if !old.is_dir() {
        return;
    }
    if !new.exists() {
        let _ = std::fs::rename(&old, &new);
    } else if old.join("reload.json").exists() {
        let _ = std::fs::rename(old.join("reload.json"), new.join("reload.json"));
    }
}

fn state_file() -> PathBuf {
    config_dir().join("sessions.json")
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub onboarded: bool,
    /// Folders whose sub-folders are projects (e.g. ~/code).
    #[serde(default)]
    pub workspaces: Vec<PathBuf>,
    /// Auto-detected folders the user removed from the board.
    #[serde(default)]
    pub hidden_workspaces: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    /// Check GitHub for new releases and install them (default on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_update: Option<bool>,
    /// Cursor and fade-in while Claude writes (default on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typing_effect: Option<bool>,
    /// Desktop notifications when a session finishes or needs you (default on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outfit: Option<bool>,
}

pub fn load_config() -> Config {
    std::fs::read_to_string(config_dir().join("config.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_config(c: &Config) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(c) {
        let _ = std::fs::write(dir.join("config.json"), json);
    }
}

#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Lifetime {
    #[serde(default)]
    pub usage: crate::session::Usage,
    #[serde(default)]
    pub sessions: u64,
    #[serde(default)]
    pub since: Option<String>,
    /// Latest plan usage windows (5-hour, weekly, …) from Claude Code.
    #[serde(default)]
    pub limits: Vec<Limit>,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
pub struct Limit {
    pub key: String,
    pub utilization: f32,
    pub resets_at: u64,
}

pub fn load_lifetime() -> Lifetime {
    std::fs::read_to_string(config_dir().join("stats.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_lifetime(l: &Lifetime) {
    let dir = config_dir();
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(json) = serde_json::to_string_pretty(l) {
        let _ = std::fs::write(dir.join("stats.json"), json);
    }
}

/// Everything needed to come back exactly where we were after `/reload`.
#[derive(Serialize, Deserialize, Default)]
pub struct Snapshot {
    pub sessions: Vec<Saved>,
    pub active: usize,
    pub draft: String,
    #[serde(default)]
    pub history: Vec<String>,
}

fn snapshot_file() -> PathBuf {
    config_dir().join("reload.json")
}

pub fn save_snapshot(s: &Snapshot) -> bool {
    let _ = std::fs::create_dir_all(config_dir());
    serde_json::to_string(s).ok().is_some_and(|j| std::fs::write(snapshot_file(), j).is_ok())
}

pub fn take_snapshot() -> Option<Snapshot> {
    let text = std::fs::read_to_string(snapshot_file()).ok()?;
    let _ = std::fs::remove_file(snapshot_file());
    serde_json::from_str(&text).ok()
}

pub fn load() -> Vec<Saved> {
    std::fs::read_to_string(state_file())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(sessions: &[Saved]) {
    let path = state_file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(tmp, path);
        }
    }
}

/// Finds Claude Code's saved transcript for a session id.
pub fn transcript(claude_id: &str) -> Option<String> {
    let projects = home().join(".claude").join("projects");
    let file = format!("{claude_id}.jsonl");
    for dir in std::fs::read_dir(projects).ok()?.flatten() {
        let p = dir.path().join(&file);
        if p.is_file() {
            return std::fs::read_to_string(p).ok();
        }
    }
    None
}
