use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Messages waiting to be sent, so a restart never drops them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queue: Vec<(String, serde_json::Value)>,
    /// Only in reload snapshots: the chat as den showed it, used when Claude
    /// Code's own transcript can't be found or read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chat: Vec<(bool, String)>,
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
    /// Subagent cards, so they reappear as they were after /reload.
    #[serde(default)]
    pub agents: Vec<crate::session::AgentSnap>,
}

fn home() -> PathBuf {
    crate::platform::home()
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

/// Writes via a synced temp file and a rename, so a crash mid-write leaves the
/// old file whole. With `backup`, the previous version is kept as `<name>.bak`.
fn write_atomic(path: &Path, data: &[u8], backup: bool) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(data)?;
    f.sync_all()?;
    drop(f);
    if backup && path.is_file() {
        let _ = std::fs::copy(path, path.with_extension("json.bak"));
    }
    std::fs::rename(&tmp, path)
}

pub fn save_snapshot(s: &Snapshot) -> bool {
    let Ok(json) = serde_json::to_vec(s) else { return false };
    // Read it back before trusting it: the next den must be able to load it.
    if serde_json::from_slice::<Snapshot>(&json).is_err() {
        return false;
    }
    write_atomic(&snapshot_file(), &json, false).is_ok()
}

/// Parses a snapshot, salvaging what it can from one written by another
/// version: sessions one by one, dropping only a part that no longer fits.
fn parse_snapshot(text: &str) -> Option<(Snapshot, bool)> {
    if let Ok(s) = serde_json::from_str::<Snapshot>(text) {
        return Some((s, true));
    }
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let mut whole = true;
    let sessions = v
        .get("sessions")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    serde_json::from_value::<Saved>(x.clone()).ok().or_else(|| {
                        whole = false;
                        let mut x = x.clone();
                        let queue = x.pointer("/live/queue").cloned();
                        let o = x.as_object_mut()?;
                        o.remove("live");
                        if let Some(q) = queue {
                            o.entry("queue").or_insert(q);
                        }
                        serde_json::from_value(x).ok()
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let str_of = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let snap = Snapshot {
        sessions,
        active: v.get("active").and_then(|x| x.as_u64()).unwrap_or(0) as usize,
        draft: str_of("draft"),
        history: serde_json::from_value(v.get("history").cloned().unwrap_or_default()).unwrap_or_default(),
    };
    Some((snap, false))
}

/// Takes the snapshot left by `/reload` (or by a reload that never finished).
/// The file is kept as `reload.prev.json`, never just deleted.
pub fn take_snapshot() -> Option<Snapshot> {
    let file = snapshot_file();
    let text = std::fs::read_to_string(&file).ok()?;
    let parsed = parse_snapshot(&text);
    let keep = match &parsed {
        Some((_, true)) => "reload.prev.json",
        _ => "reload.unread.json",
    };
    let _ = std::fs::rename(&file, config_dir().join(keep));
    parsed.map(|(s, _)| s)
}

fn read_sessions(path: &Path) -> Option<Vec<Saved>> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn load() -> Vec<Saved> {
    let path = state_file();
    read_sessions(&path)
        .or_else(|| read_sessions(&path.with_extension("json.bak")))
        .unwrap_or_default()
}

/// Adds saved sessions the snapshot doesn't have, so nothing goes missing
/// when the two disagree.
pub fn merge_missing(snap: &mut Vec<Saved>, saved: Vec<Saved>) {
    for sv in saved {
        let known = sv.claude_id.is_some()
            && snap.iter().any(|x| x.claude_id == sv.claude_id || x.live.as_ref().is_some_and(|l| l.claude_id == sv.claude_id));
        if !known {
            snap.push(sv);
        }
    }
}

pub fn save(sessions: &[Saved]) {
    if let Ok(json) = serde_json::to_string_pretty(sessions) {
        let _ = write_atomic(&state_file(), json.as_bytes(), true);
    }
}

/// Claude Code's most recent conversation started in `cwd` (its session id),
/// like `claude --continue` picks.
pub fn latest_conversation(cwd: &Path) -> Option<String> {
    latest_in(&home().join(".claude").join("projects"), cwd)
}

fn latest_in(projects: &Path, cwd: &Path) -> Option<String> {
    let slug: String = cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(projects.join(slug))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("agent-")))
        .filter_map(|p| Some((std::fs::metadata(&p).ok()?.modified().ok()?, p)))
        .collect();
    files.sort_by_key(|a| std::cmp::Reverse(a.0));
    // Different folders can share a slug; take the newest that really is this one
    // and has an actual message in it.
    files.into_iter().find_map(|(_, p)| {
        let text = std::fs::read_to_string(&p).ok()?;
        let mine = text.lines().take(50).any(|l| {
            serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("cwd").and_then(|c| c.as_str()).map(|c| Path::new(c) == cwd))
                .unwrap_or(false)
        });
        let talked = text.contains("\"type\":\"assistant\"");
        if mine && talked { Some(p.file_stem()?.to_string_lossy().into_owned()) } else { None }
    })
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
    #[test]
    fn snapshot_salvage_keeps_sessions_when_live_part_is_unreadable() {
        let text = r#"{"sessions":[{"name":"a","cwd":"/x","claude_id":"A","live":{"handoff":"new format","queue":[["hi",null]]}},{"name":"b","cwd":"/y","claude_id":"B"}],"active":1,"draft":"half typed","future_field":3}"#;
        let (s, whole) = parse_snapshot(text).unwrap();
        assert!(!whole);
        assert_eq!(s.sessions.len(), 2);
        assert!(s.sessions[0].live.is_none());
        assert_eq!(s.sessions[0].queue.len(), 1);
        assert_eq!((s.active, s.draft.as_str()), (1, "half typed"));
    }

    #[test]
    fn merge_adds_only_missing_sessions() {
        let mk = |id: &str| Saved { claude_id: Some(id.into()), ..serde_json::from_str(r#"{"name":"n","cwd":"/x","claude_id":null}"#).unwrap() };
        let mut snap = vec![mk("A")];
        merge_missing(&mut snap, vec![mk("A"), mk("B")]);
        let ids: Vec<_> = snap.iter().map(|s| s.claude_id.clone().unwrap()).collect();
        assert_eq!(ids, ["A", "B"]);
    }

    #[test]
    fn atomic_write_keeps_a_backup() {
        let dir = std::env::temp_dir().join(format!("den-atomic-{}", std::process::id()));
        let f = dir.join("sessions.json");
        write_atomic(&f, b"[1]", true).unwrap();
        write_atomic(&f, b"[2]", true).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "[2]");
        assert_eq!(std::fs::read_to_string(dir.join("sessions.json.bak")).unwrap(), "[1]");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latest_conversation_picks_newest_real_one_for_this_folder() {
        let base = std::env::temp_dir().join(format!("den-latest-{}", std::process::id()));
        let dir = base.join("-w-app");
        std::fs::create_dir_all(&dir).unwrap();
        let line = |cwd: &str| format!("{{\"type\":\"user\",\"cwd\":\"{cwd}\"}}\n{{\"type\":\"assistant\",\"cwd\":\"{cwd}\"}}\n");
        std::fs::write(dir.join("old.jsonl"), line("/w/app")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("new.jsonl"), line("/w/app")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("other.jsonl"), line("/w.app")).unwrap();
        std::fs::write(dir.join("agent-x.jsonl"), line("/w/app")).unwrap();
        assert_eq!(latest_in(&base, Path::new("/w/app")).as_deref(), Some("new"));
        assert_eq!(latest_in(&base, Path::new("/nope")), None);
        let _ = std::fs::remove_dir_all(&base);
    }

}
