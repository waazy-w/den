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
    /// Waiting out a usage limit: when den sends "continue" (epoch seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<String>,
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

/// den used to be called ctui: bring its settings and sessions over once.
/// When both folders exist, ctui's sessions are merged into den's list (and
/// into a pending reload snapshot), so no conversation goes missing.
pub fn migrate_legacy() {
    migrate_from(&config_root().join("ctui"), &config_dir());
}

fn migrate_from(old: &std::path::Path, new: &std::path::Path) {
    use serde_json::Value;
    if !old.is_dir() {
        return;
    }
    if !new.exists() {
        let _ = std::fs::rename(old, new);
        return;
    }
    if old.join("reload.json").exists() && !new.join("reload.json").exists() {
        let _ = std::fs::rename(old.join("reload.json"), new.join("reload.json"));
    }
    let read = |p: &std::path::Path| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let old_sessions: Vec<Value> = read(&old.join("sessions.json")).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let id = |s: &Value| s.get("claude_id").and_then(Value::as_str).map(String::from);
    let merge = |list: &mut Vec<Value>| {
        let have: Vec<String> = list.iter().filter_map(id).collect();
        for s in &old_sessions {
            if id(s).is_some_and(|i| !have.contains(&i)) {
                list.push(s.clone());
            }
        }
    };
    let mut sessions: Vec<Value> = read(&new.join("sessions.json")).and_then(|v| v.as_array().cloned()).unwrap_or_default();
    merge(&mut sessions);
    if let Ok(json) = serde_json::to_string_pretty(&sessions) {
        let _ = std::fs::write(new.join("sessions.json"), json);
    }
    if let Some(mut snap) = read(&new.join("reload.json"))
        && let Some(list) = snap.get_mut("sessions").and_then(Value::as_array_mut)
    {
        merge(list);
        if let Ok(json) = serde_json::to_string(&snap) {
            let _ = std::fs::write(new.join("reload.json"), json);
        }
    }
    // Done once: keep the old folder around, renamed, in case it's needed.
    let _ = std::fs::rename(old, old.with_file_name("ctui.migrated"));
}

fn state_file() -> PathBuf {
    config_dir().join("sessions.json")
}

#[derive(Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub onboarded: bool,
    /// Which version of the tour was last seen (a new version shows once).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tour_version: Option<u32>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squeeze: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resume: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub smart_compact: Option<bool>,
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
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
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
    match serde_json::from_str(&text) {
        Ok(s) => {
            let _ = std::fs::remove_file(snapshot_file());
            Some(s)
        }
        Err(_) => {
            // Unreadable (e.g. written by another version): keep it, never lose it.
            let _ = std::fs::rename(snapshot_file(), config_dir().join("reload.unread.json"));
            None
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_sessions_merge_into_den() {
        let base = std::env::temp_dir().join(format!("den-migrate-{}", std::process::id()));
        let (old, new) = (base.join("ctui"), base.join("den"));
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("sessions.json"), r#"[{"name":"big chat","cwd":"/x","claude_id":"A"},{"name":"dup","cwd":"/x","claude_id":"B"}]"#).unwrap();
        std::fs::write(new.join("sessions.json"), r#"[{"name":"small","cwd":"/x","claude_id":"B"}]"#).unwrap();
        std::fs::write(new.join("reload.json"), r#"{"sessions":[{"name":"small","cwd":"/x","claude_id":"B"}],"active":0,"draft":""}"#).unwrap();
        migrate_from(&old, &new);
        let s: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(new.join("sessions.json")).unwrap()).unwrap();
        let ids: Vec<&str> = s.as_array().unwrap().iter().map(|x| x["claude_id"].as_str().unwrap()).collect();
        assert_eq!(ids, vec!["B", "A"]);
        let r: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(new.join("reload.json")).unwrap()).unwrap();
        assert_eq!(r["sessions"].as_array().unwrap().len(), 2);
        assert!(!old.exists() && base.join("ctui.migrated").exists());
        let _ = std::fs::remove_dir_all(&base);
    }
}
