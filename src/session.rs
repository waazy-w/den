use crate::tools;
use ratatui::text::Line;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Idle,
    Starting,
    Working,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ToolState {
    Running,
    Ok,
    Err,
    Denied,
    Stopped,
}

#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub input: Value,
    pub result: Option<String>,
    pub state: ToolState,
    pub started: Instant,
    pub took: Option<Duration>,
    pub sub_steps: usize,
    pub waiting: bool,
    pub finished_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteKind {
    Info,
    Error,
    Command,
}

#[derive(Debug, Clone)]
pub enum Item {
    User(String),
    Assistant(String),
    Tool(Tool),
    Note(String, NoteKind),
}

pub struct Entry {
    pub item: Item,
    pub rev: u32,
    pub expanded: bool,
    pub cache: Option<RenderCache>,
    /// When the entry appeared (None for replayed history).
    pub born: Option<Instant>,
    /// Bytes of streamed text revealed so far; usize::MAX = everything.
    pub shown: usize,
}

impl Entry {
    pub fn item_len(&self) -> usize {
        match &self.item {
            Item::Assistant(t) | Item::User(t) => t.len(),
            _ => 0,
        }
    }
}

pub struct RenderCache {
    pub key: (u16, u32, bool, bool),
    pub lines: Vec<Line<'static>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Permission {
    pub request_id: String,
    pub tool_name: String,
    pub input: Value,
    pub suggestions: Vec<Value>,
    pub tool_use_id: Option<String>,
}

/// Usage counted for one session (and, summed, for the lifetime stats).
#[derive(Debug, Clone, Copy, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub turns: u64,
    #[serde(default)]
    pub tok_in: u64,
    #[serde(default)]
    pub tok_out: u64,
    #[serde(default)]
    pub working_ms: u64,
    #[serde(default)]
    pub cost: f64,
}

impl Usage {
    pub fn add(&mut self, o: &Usage) {
        self.turns += o.turns;
        self.tok_in += o.tok_in;
        self.tok_out += o.tok_out;
        self.working_ms += o.working_ms;
        self.cost += o.cost;
    }
}

pub const MODES: [&str; 4] = ["default", "acceptEdits", "plan", "auto"];

pub fn mode_label(m: &str) -> &'static str {
    match m {
        "acceptEdits" => "accept edits",
        "plan" => "plan mode",
        "auto" => "auto mode",
        "bypassPermissions" => "bypass permissions",
        "dontAsk" => "don't ask",
        _ => "default",
    }
}

/// Command list from an `initialize` response: (name, short description).
pub fn parse_commands(resp: &Value) -> Vec<(String, String)> {
    let Some(cmds) = resp.get("commands").and_then(Value::as_array) else { return vec![] };
    cmds.iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.to_string();
            let mut d = tools::s(c, "description").trim().to_string();
            if let Some(i) = d.rfind(" (") && d.ends_with(')') {
                d.truncate(i);
            }
            if let Some(i) = d.find(". ") {
                d.truncate(i);
            }
            let d = d.trim_end_matches('.').to_string();
            let d = if d.is_empty() { tools::s(c, "argumentHint").to_string() } else { d };
            Some((name, d))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    Waiting,
    Busy,
    Idle,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// "2h 10m", "4m", "30s".
pub fn dur_words(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    }
}

/// Did this reply say the usage limit was hit? Returns the reset time when
/// the message carries one ("…usage limit reached|1736000000"), else 0.
/// "/hooks isn't available in this environment." → "/hooks" (with its arguments).
pub fn screen_only(result: &str) -> Option<String> {
    let cmd = result.strip_suffix(" isn't available in this environment.")?;
    (cmd.starts_with('/') && !cmd.contains('\n') && cmd.len() < 200).then(|| cmd.to_string())
}

pub fn limit_reset(text: &str) -> Option<u64> {
    let low = text.to_lowercase();
    let hit = low.contains("usage limit reached") || low.contains("hit your limit") || low.contains("you've reached your limit");
    if !hit || text.len() > 400 {
        return None;
    }
    let epoch = text
        .rsplit('|')
        .next()
        .and_then(|t| t.trim().parse::<u64>().ok())
        .filter(|_| text.contains('|'))
        .unwrap_or(0);
    Some(epoch)
}

fn ago(secs: u64) -> String {
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{}m ago", secs / 60),
        3600..86400 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86400),
    }
}

/// Bookkeeping tools shown elsewhere (the sidebar task list) or not worth a row.
pub fn quiet(item: &Item) -> bool {
    matches!(item, Item::Tool(t) if matches!(t.name.as_str(), "TaskCreate" | "TaskUpdate" | "TaskList" | "TaskGet" | "TodoWrite" | "ToolSearch"))
}

pub fn is_test_command(cmd: &str) -> bool {
    let c = cmd.to_lowercase();
    ["cargo test", "cargo nextest", "pytest", "npm test", "npm run test", "yarn test", "pnpm test", "go test", "jest", "vitest", "rspec", "mix test", "swift test", "make test"]
        .iter()
        .any(|t| c.contains(t))
}

fn task_number(result: &str) -> Option<String> {
    let rest = &result[result.find('#')? + 1..];
    let n: String = rest.chars().take_while(char::is_ascii_digit).collect();
    (!n.is_empty()).then_some(n)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AgentStatus {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum AgentLine {
    Tool { id: String, name: String, input: Value, state: ToolState },
    Text(String),
}

/// An agent card carried across /reload (instants become ages in ms).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentSnap {
    pub id: String,
    pub kind: String,
    pub desc: String,
    pub activity: String,
    pub log: Vec<AgentLine>,
    pub status: AgentStatus,
    pub age_ms: u64,
    pub finished_ago_ms: Option<u64>,
    pub tools: usize,
    pub tokens: u64,
    pub background: bool,
}

impl Agent {
    pub fn snap(&self) -> AgentSnap {
        AgentSnap {
            id: self.id.clone(),
            kind: self.kind.clone(),
            desc: self.desc.clone(),
            activity: self.activity.clone(),
            log: self.log.clone(),
            status: self.status,
            age_ms: self.started.elapsed().as_millis() as u64,
            finished_ago_ms: self.finished.map(|f| f.elapsed().as_millis() as u64),
            tools: self.tools,
            tokens: self.tokens,
            background: self.background,
        }
    }

    pub fn from_snap(s: AgentSnap) -> Agent {
        let now = Instant::now();
        let ago = |ms: u64| now.checked_sub(Duration::from_millis(ms)).unwrap_or(now);
        Agent {
            id: s.id,
            kind: s.kind,
            desc: s.desc,
            activity: s.activity,
            log: s.log,
            status: s.status,
            started: ago(s.age_ms),
            finished: s.finished_ago_ms.map(ago),
            tools: s.tools,
            tokens: s.tokens,
            background: s.background,
        }
    }
}

/// Seconds since the epoch for an RFC 3339 UTC stamp like "2026-11-17T15:10:52.112Z".
fn stamp_secs(t: &str) -> Option<u64> {
    let n = |r: std::ops::Range<usize>| t.get(r)?.parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(5..7)?, n(8..10)?);
    let (hh, mm, ss) = (n(11..13)?, n(14..16)?, n(17..19)?);
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    u64::try_from(days * 86_400 + hh * 3600 + mm * 60 + ss).ok()
}

/// Subagents still running according to a transcript: launched, and neither
/// answered nor (for background ones) reported done by a task notification.
/// Lets cards come back after /reload even without a snapshot of them.
pub fn running_agents(jsonl: &str) -> Vec<Agent> {
    let mut found: Vec<(String, Value, Option<u64>)> = Vec::new();
    let mut ended: HashSet<String> = HashSet::new();
    let mut background: HashSet<String> = HashSet::new();
    for line in jsonl.lines() {
        if !line.contains("toolu_") {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let at = v.get("timestamp").and_then(Value::as_str).and_then(stamp_secs);
        let text = match &v["message"]["content"] {
            Value::String(s) => s.as_str(),
            _ => v.get("content").and_then(Value::as_str).unwrap_or(""),
        };
        if text.contains("<task-notification>") {
            for part in text.split("<tool-use-id>").skip(1) {
                if let Some(id) = part.split("</tool-use-id>").next() {
                    ended.insert(id.trim().to_string());
                }
            }
        }
        let Some(blocks) = v["message"]["content"].as_array() else { continue };
        for b in blocks {
            match tools::s(b, "type") {
                "tool_use" if matches!(tools::s(b, "name"), "Agent" | "Task") => {
                    found.push((tools::s(b, "id").to_string(), b.get("input").cloned().unwrap_or(Value::Null), at));
                }
                "tool_result" => {
                    let id = tools::s(b, "tool_use_id").to_string();
                    let out = tools::result_text(b.get("content").unwrap_or(&Value::Null));
                    if out.starts_with("Async agent launched") {
                        background.insert(id);
                    } else {
                        ended.insert(id);
                    }
                }
                _ => {}
            }
        }
    }
    let now = Instant::now();
    let wall = now_secs();
    found
        .into_iter()
        .filter(|(id, _, _)| !ended.contains(id))
        .map(|(id, input, at)| Agent {
            kind: tools::s(&input, "subagent_type").to_string(),
            desc: tools::s(&input, "description").to_string(),
            activity: String::new(),
            log: Vec::new(),
            status: AgentStatus::Running,
            started: at
                .and_then(|t| now.checked_sub(Duration::from_secs(wall.saturating_sub(t))))
                .unwrap_or(now),
            finished: None,
            tools: 0,
            tokens: 0,
            background: background.contains(&id),
            id,
        })
        .collect()
}

/// A subagent launched by the Agent/Task tool, with its own activity log.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: String,
    pub kind: String,
    pub desc: String,
    /// Latest progress line from Claude Code, e.g. "Reading README.md".
    pub activity: String,
    pub log: Vec<AgentLine>,
    pub status: AgentStatus,
    pub started: Instant,
    pub finished: Option<Instant>,
    pub tools: usize,
    pub tokens: u64,
    /// Launched in the background: its tool result only means "started", and
    /// Claude Code's task notification says when it's really done.
    pub background: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoState {
    Pending,
    Active,
    Done,
}

/// One item of Claude's task list (TaskCreate/TaskUpdate or TodoWrite).
#[derive(Debug, Clone)]
pub struct Todo {
    pub id: String,
    pub text: String,
    pub active_form: String,
    pub state: TodoState,
}

fn todo_state(s: &str) -> TodoState {
    match s {
        "in_progress" => TodoState::Active,
        "completed" => TodoState::Done,
        _ => TodoState::Pending,
    }
}

/// Edits Claude made to one file this session.
#[derive(Debug, Clone, Default)]
pub struct FileChange {
    pub path: PathBuf,
    /// (old, new) pairs; Write has an empty old side.
    pub edits: Vec<(String, String)>,
    pub added: usize,
    pub removed: usize,
}

/// Lines added and removed going from `old` to `new`, ignoring shared ends.
pub fn line_delta(old: &str, new: &str) -> (usize, usize) {
    let o: Vec<&str> = old.lines().collect();
    let n: Vec<&str> = new.lines().collect();
    let pre = o.iter().zip(&n).take_while(|(a, b)| a == b).count();
    let suf = o[pre..].iter().rev().zip(n[pre..].iter().rev()).take_while(|(a, b)| a == b).count();
    (n.len() - pre - suf, o.len() - pre - suf)
}

pub struct Session {
    pub sid: usize,
    pub name: String,
    pub auto_named: bool,
    pub cwd: PathBuf,
    pub claude_id: Option<String>,
    pub model: Option<String>,
    pub entries: Vec<Entry>,
    pub status: Status,
    pub permissions: VecDeque<Permission>,
    pub cost: f64,
    cost_base: f64,
    pub ctx_used: u64,
    pub ctx_window: u64,
    pub slash_commands: Vec<String>,
    pub commands: Vec<(String, String)>,
    /// Messages typed while Claude was busy, sent when the turn ends.
    pub queue: VecDeque<(String, Value)>,
    pub agents: Vec<Agent>,
    pub mode: String,
    /// Claude Code output style for this session ("default" talks normally).
    pub style: String,
    pub usage: Usage,
    /// Usage from the latest finished turn, for the app's lifetime totals.
    pub fresh_usage: Option<Usage>,
    proc_cost: f64,
    pub turn_started: Option<Instant>,
    pub thinking: bool,
    pub unread: bool,
    pub last_active: Instant,
    /// Lines scrolled up from the bottom; 0 = follow output.
    pub scroll: usize,
    pub last_total: usize,
    pub selected: Option<usize>,
    pub ensure_visible: bool,
    /// Eased display position chasing `scroll`.
    pub scroll_pos: f32,
    pub ctx_shown: f32,
    pub unread_since: Option<Instant>,
    pub perm_since: Option<Instant>,
    tool_index: HashMap<String, usize>,
    stream_blocks: HashMap<(String, u64), usize>,
    streamed_msgs: HashSet<String>,
    /// Text blocks joined mid-stream, keyed by message id → entry index.
    partial_msgs: HashMap<String, usize>,
    replaying: bool,
    pub todos: Vec<Todo>,
    /// TaskCreate tool_use id → todo index, until its result names the task id.
    pending_tasks: HashMap<String, usize>,
    pub changes: Vec<FileChange>,
    /// Thinking effort passed to `claude --effort`; None = Claude Code's default.
    pub effort: Option<String>,
    /// Restart the process once the turn ends (an effort/style change).
    pub respawn: bool,
    /// A command Claude Code can only show in its own screen ("/hooks isn't
    /// available in this environment"); den opens it there.
    pub needs_screen: Option<String>,
    /// A test command just passed (Clawd celebrates).
    pub tests_passed: bool,
    pub finished_at: Option<Instant>,
    /// Entry to scroll to (search result), and when it was picked.
    pub jump: Option<(usize, Instant)>,
    pub jump_pending: bool,
    /// Unsent input, kept while another session has the editor.
    pub draft: String,
    /// Output of `!` shell commands, attached to the next message to Claude.
    pub shell_context: Vec<String>,
    /// Entry of a running `/plugin` command; a success restarts claude.
    pub plugin_cmd: Option<usize>,
    /// The turn stopped on a usage limit: Some(reset time in epoch seconds,
    /// or 0 when Claude Code didn't say). The app turns it into `resume_at`.
    pub limit_hit: Option<u64>,
    /// When den will send "continue" by itself (epoch seconds).
    pub resume_at: Option<u64>,
    /// Notes the user pinned; they survive compaction (re-sent after it).
    pub pins: Vec<String>,
    /// A compaction just happened: attach the pins to the next message.
    pub repin: bool,
    /// Highest context alert shown (70 / 85); resets when context drops.
    pub ctx_alerted: u8,
    /// den's own auto-compact already ran at this fill; re-armed below 60%.
    pub auto_compacted: bool,
}

impl Session {
    pub fn new(sid: usize, cwd: PathBuf) -> Self {
        Self {
            sid,
            name: "new session".into(),
            auto_named: true,
            cwd,
            claude_id: None,
            model: None,
            entries: Vec::new(),
            status: Status::Idle,
            permissions: VecDeque::new(),
            cost: 0.0,
            cost_base: 0.0,
            ctx_used: 0,
            ctx_window: 0,
            slash_commands: Vec::new(),
            commands: Vec::new(),
            queue: VecDeque::new(),
            agents: Vec::new(),
            mode: "default".into(),
            style: "default".into(),
            usage: Usage::default(),
            fresh_usage: None,
            proc_cost: 0.0,
            turn_started: None,
            thinking: false,
            unread: false,
            last_active: Instant::now(),
            scroll: 0,
            last_total: 0,
            selected: None,
            ensure_visible: false,
            scroll_pos: 0.0,
            ctx_shown: 0.0,
            unread_since: None,
            perm_since: None,
            tool_index: HashMap::new(),
            stream_blocks: HashMap::new(),
            streamed_msgs: HashSet::new(),
            partial_msgs: HashMap::new(),
            replaying: false,
            todos: Vec::new(),
            pending_tasks: HashMap::new(),
            changes: Vec::new(),
            effort: None,
            respawn: false,
            needs_screen: None,
            tests_passed: false,
            finished_at: None,
            jump: None,
            jump_pending: false,
            draft: String::new(),
            shell_context: Vec::new(),
            plugin_cmd: None,
            limit_hit: None,
            resume_at: None,
            pins: Vec::new(),
            repin: false,
            ctx_alerted: 0,
            auto_compacted: false,
        }
    }

    pub fn busy(&self) -> bool {
        matches!(self.status, Status::Working | Status::Starting)
    }

    pub fn push(&mut self, item: Item) -> usize {
        let born = (!self.replaying).then(Instant::now);
        self.entries.push(Entry { item, rev: 0, expanded: false, cache: None, born, shown: usize::MAX });
        self.entries.len() - 1
    }

    /// Adds a running `!command` row; returns its entry index.
    pub fn start_shell(&mut self, cmd: &str) -> usize {
        let i = self.push(Item::Tool(Tool {
            name: "Shell".into(),
            input: json!({"command": cmd}),
            result: None,
            state: ToolState::Running,
            started: Instant::now(),
            took: None,
            sub_steps: 0,
            waiting: false,
            finished_at: None,
        }));
        self.entries[i].expanded = true;
        self.scroll = 0;
        i
    }

    /// Fills in a finished `!command` and queues its output for Claude.
    pub fn finish_shell(&mut self, idx: usize, output: &str, code: i32) {
        let Some(Item::Tool(t)) = self.entries.get_mut(idx).map(|e| &mut e.item) else { return };
        let cmd = tools::s(&t.input, "command").to_string();
        let (out, _) = crate::squeeze::squeeze(output, "(not saved)");
        t.result = Some(if out.is_empty() { "(no output)".into() } else { out.clone() });
        t.state = if code == 0 { ToolState::Ok } else { ToolState::Err };
        t.took = Some(t.started.elapsed());
        t.finished_at = Some(Instant::now());
        self.touch(idx);
        if self.plugin_cmd == Some(idx) {
            return;
        }
        self.shell_context.push(format!("I ran `{cmd}` in my shell (exit code {code}):\n```\n{out}\n```"));
    }

    fn touch(&mut self, idx: usize) {
        if let Some(e) = self.entries.get_mut(idx) {
            e.rev = e.rev.wrapping_add(1);
        }
    }

    pub fn ctx_pct(&self) -> f32 {
        if self.ctx_window == 0 { 0.0 } else { self.ctx_used as f32 / self.ctx_window as f32 * 100.0 }
    }

    /// Steps reveal/scroll/meter animations. Returns true while any is running.
    pub fn advance(&mut self, dt: f32, visible: bool) -> bool {
        let mut busy = false;
        let mut touched = Vec::new();
        for (i, e) in self.entries.iter_mut().enumerate() {
            let Item::Assistant(text) = &e.item else { continue };
            if e.shown >= text.len() {
                continue;
            }
            if !visible {
                e.shown = usize::MAX;
                touched.push(i);
                continue;
            }
            let remaining = text.len() - e.shown;
            let speed = (remaining as f32 * 9.0).max(160.0);
            let mut next = (e.shown + (speed * dt).ceil() as usize).min(text.len());
            while !text.is_char_boundary(next) {
                next += 1;
            }
            if let Some(ws) = text[next..].find(char::is_whitespace).filter(|w| *w < 16) {
                next += ws;
            }
            e.shown = next;
            touched.push(i);
            busy |= next < text.len();
        }
        for i in touched {
            self.touch(i);
        }
        let target = self.scroll as f32;
        if (self.scroll_pos - target).abs() > 0.0 {
            self.scroll_pos = if visible { crate::anim::approach(self.scroll_pos, target, dt, 22.0) } else { target };
            busy |= self.scroll_pos != target;
        }
        let pct = self.ctx_pct();
        if self.ctx_shown != pct {
            let next = self.ctx_shown + (pct - self.ctx_shown) * (1.0 - (-8.0 * dt).exp());
            self.ctx_shown = if (pct - next).abs() < 0.2 || !visible { pct } else { next };
            busy |= self.ctx_shown != pct;
        }
        busy
    }

    pub fn note(&mut self, text: impl Into<String>, kind: NoteKind) {
        self.push(Item::Note(text.into(), kind));
    }

    pub fn user_message(&mut self, text: &str) {
        if self.auto_named && !text.starts_with('/') {
            self.name = auto_name(text);
            self.auto_named = false;
        }
        if text.starts_with('/') {
            self.note(text, NoteKind::Command);
        } else {
            self.push(Item::User(text.to_string()));
        }
        self.status = Status::Working;
        self.turn_started.get_or_insert_with(Instant::now);
        self.scroll = 0;
        self.last_active = Instant::now();
    }

    pub fn reset_process_state(&mut self) {
        self.cost_base = self.cost;
        self.proc_cost = 0.0;
        self.permissions.clear();
        self.stream_blocks.clear();
        self.thinking = false;
        self.turn_started = None;
        self.stop_running_tools(ToolState::Stopped);
        for a in &mut self.agents {
            if a.status == AgentStatus::Running {
                a.status = AgentStatus::Failed;
                a.finished = Some(Instant::now());
            }
        }
    }

    fn stop_running_tools(&mut self, state: ToolState) {
        let mut touched = Vec::new();
        for (i, e) in self.entries.iter_mut().enumerate() {
            if let Item::Tool(t) = &mut e.item
                && t.state == ToolState::Running
            {
                t.state = state;
                t.waiting = false;
                t.took = Some(t.started.elapsed());
                touched.push(i);
            }
        }
        for i in touched {
            self.touch(i);
        }
    }

    pub fn set_waiting(&mut self, tool_use_id: &str, waiting: bool) {
        if let Some(&i) = self.tool_index.get(tool_use_id) {
            if let Item::Tool(t) = &mut self.entries[i].item {
                t.waiting = waiting;
            }
            self.touch(i);
        }
    }

    pub fn mark_denied(&mut self, tool_use_id: &str) {
        if let Some(&i) = self.tool_index.get(tool_use_id) {
            if let Item::Tool(t) = &mut self.entries[i].item {
                t.state = ToolState::Denied;
                t.took = Some(t.started.elapsed());
            }
            self.touch(i);
        }
    }

    /// Applies one protocol message. Returns a reply to write back, if any.
    pub fn handle(&mut self, v: &Value) -> Option<Value> {
        let ty = tools::s(v, "type");
        let parent = v.get("parent_tool_use_id").and_then(Value::as_str);
        // A message sent mid-turn can start another turn after the result; any
        // new output means Claude is working again.
        if self.status == Status::Idle && !self.replaying && matches!(ty, "stream_event" | "assistant") {
            self.status = Status::Working;
            self.turn_started.get_or_insert_with(Instant::now);
        }
        match ty {
            "system" => self.on_system(v),
            "stream_event" if parent.is_none() => self.on_stream(v),
            "assistant" => match parent {
                None => self.on_assistant(v),
                Some(p) => self.on_subagent(p, v),
            },
            "user" => match parent {
                None => self.on_user(v),
                Some(p) => self.on_subagent_result(p, v),
            },
            "result" => self.on_result(v),
            "control_request" => return self.on_control(v),
            "control_response" => {
                let r = &v["response"];
                let rid = tools::s(r, "request_id");
                if rid.starts_with("den-mcp") {
                    let ok = tools::s(r, "subtype") != "error";
                    let text = match (ok, rid.starts_with("den-mcp-reconnect")) {
                        (true, false) => mcp_report(&r["response"]),
                        (true, true) => "reconnected · /mcp shows the status".to_string(),
                        (false, _) => format!("MCP: {}", tools::s(r, "error")),
                    };
                    self.note(text, if ok { NoteKind::Info } else { NoteKind::Error });
                }
                if tools::s(r, "request_id").starts_with("den-init") || tools::s(r, "request_id").starts_with("ctui-init") {
                    let cmds = parse_commands(&r["response"]);
                    if !cmds.is_empty() {
                        self.commands = cmds;
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn on_system(&mut self, v: &Value) {
        match tools::s(v, "subtype") {
            "init" => {
                if let Some(id) = v.get("session_id").and_then(Value::as_str) {
                    self.claude_id = Some(id.to_string());
                }
                if let Some(m) = v.get("model").and_then(Value::as_str) {
                    self.model = Some(m.to_string());
                }
                if let Some(cmds) = v.get("slash_commands").and_then(Value::as_array) {
                    self.slash_commands =
                        cmds.iter().filter_map(Value::as_str).map(String::from).collect();
                }
                if self.status == Status::Starting {
                    self.status = Status::Working;
                }
            }
            "compact_boundary" => {
                let kept = if self.pins.is_empty() { String::new() } else { format!(" · your {} pin(s) go along with your next message", self.pins.len()) };
                self.note(format!("context compacted{kept}"), NoteKind::Info);
                self.repin = !self.pins.is_empty() && !self.replaying;
            }
            "task_started" | "task_progress" | "task_notification" => self.on_task(v),
            _ => {}
        }
    }

    fn on_stream(&mut self, v: &Value) {
        let Some(ev) = v.get("event") else { return };
        let msg_id = tools::s(v, "api_message_id").to_string();
        let index = ev.get("index").and_then(Value::as_u64).unwrap_or(0);
        match tools::s(ev, "type") {
            "message_start" => self.thinking = false,
            "content_block_start" => {
                let block = &ev["content_block"];
                match tools::s(block, "type") {
                    "text" => {
                        self.thinking = false;
                        let idx = self.push(Item::Assistant(String::new()));
                        self.entries[idx].shown = 0;
                        self.stream_blocks.insert((msg_id.clone(), index), idx);
                        self.streamed_msgs.insert(msg_id);
                    }
                    "thinking" | "redacted_thinking" => self.thinking = true,
                    _ => self.thinking = false,
                }
            }
            "content_block_delta" => {
                let delta = &ev["delta"];
                if tools::s(delta, "type") != "text_delta" {
                    return;
                }
                let key = (msg_id.clone(), index);
                let idx = match self.stream_blocks.get(&key) {
                    Some(&i) => i,
                    None => {
                        // Joined mid-block (after /reload): show what's still coming and
                        // swap in the full text when the finished message arrives.
                        let i = self.push(Item::Assistant(String::new()));
                        self.entries[i].shown = 0;
                        self.stream_blocks.insert(key, i);
                        self.streamed_msgs.insert(msg_id.clone());
                        self.partial_msgs.insert(msg_id, i);
                        i
                    }
                };
                if let Item::Assistant(t) = &mut self.entries[idx].item {
                    t.push_str(tools::s(delta, "text"));
                }
                self.touch(idx);
            }
            "content_block_stop" => self.thinking = false,
            _ => {}
        }
    }

    fn on_assistant(&mut self, v: &Value) {
        let msg = &v["message"];
        let msg_id = tools::s(msg, "id").to_string();
        if let Some(idx) = self.partial_msgs.remove(&msg_id) {
            let full: String = msg
                .get("content")
                .and_then(Value::as_array)
                .map(|bs| bs.iter().filter(|b| tools::s(b, "type") == "text").map(|b| tools::s(b, "text")).collect())
                .unwrap_or_default();
            if !full.is_empty()
                && let Some(e) = self.entries.get_mut(idx)
            {
                e.item = Item::Assistant(full);
                e.shown = usize::MAX;
                e.rev = e.rev.wrapping_add(1);
            }
        }
        let streamed = self.streamed_msgs.contains(&msg_id);
        if let Some(blocks) = msg.get("content").and_then(Value::as_array) {
            for b in blocks {
                match tools::s(b, "type") {
                    "text" if !streamed => {
                        let t = tools::s(b, "text");
                        if !t.trim().is_empty() {
                            self.push(Item::Assistant(t.to_string()));
                        }
                    }
                    "tool_use" => {
                        let id = tools::s(b, "id").to_string();
                        if self.tool_index.contains_key(&id) {
                            continue;
                        }
                        let tool = Tool {
                            name: tools::s(b, "name").to_string(),
                            input: b.get("input").cloned().unwrap_or(Value::Null),
                            result: None,
                            state: ToolState::Running,
                            started: Instant::now(),
                            took: None,
                            sub_steps: 0,
                            waiting: false,
                            finished_at: None,
                        };
                        let name = tool.name.clone();
                        let input = tool.input.clone();
                        let idx = self.push(Item::Tool(tool));
                        self.tool_index.insert(id.clone(), idx);
                        self.track_todos(&id, &name, &input);
                    }
                    _ => {}
                }
            }
        }
        if let Some(u) = msg.get("usage") {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            let used = n("input_tokens")
                + n("cache_read_input_tokens")
                + n("cache_creation_input_tokens")
                + n("output_tokens");
            if used > 0 {
                self.ctx_used = used;
            }
        }
        if let Some(m) = msg.get("model").and_then(Value::as_str) {
            self.model = Some(m.to_string());
        }
    }

    fn agent_mut(&mut self, id: &str) -> &mut Agent {
        if let Some(i) = self.agents.iter().position(|a| a.id == id) {
            return &mut self.agents[i];
        }
        let input = self.tool_index.get(id).and_then(|&i| match &self.entries[i].item {
            Item::Tool(t) => Some(t.input.clone()),
            _ => None,
        });
        let input = input.unwrap_or(Value::Null);
        self.agents.push(Agent {
            id: id.to_string(),
            kind: tools::s(&input, "subagent_type").to_string(),
            desc: tools::s(&input, "description").to_string(),
            activity: String::new(),
            log: Vec::new(),
            status: AgentStatus::Running,
            started: Instant::now(),
            finished: None,
            tools: 0,
            tokens: 0,
            background: false,
        });
        self.agents.last_mut().unwrap()
    }

    fn on_task(&mut self, v: &Value) {
        let id = tools::s(v, "tool_use_id");
        if id.is_empty() {
            return;
        }
        // Background Bash commands report through task events too; only
        // Agent/Task tools are subagents.
        let in_subagent = self.agents.iter().any(|a| a.log.iter().any(|l| matches!(l, AgentLine::Tool { id: t, .. } if t == id)));
        let is_agent = !in_subagent
            && self.tool_index.get(id).is_none_or(|&i| {
                matches!(&self.entries[i].item, Item::Tool(t) if matches!(t.name.as_str(), "Agent" | "Task"))
            });
        if !is_agent && !self.agents.iter().any(|a| a.id == id) {
            return;
        }
        let sub = tools::s(v, "subtype");
        let a = self.agent_mut(id);
        let desc = tools::s(v, "description");
        match sub {
            "task_started" => {
                if !desc.is_empty() {
                    a.desc = desc.to_string();
                }
                let kind = tools::s(v, "subagent_type");
                if !kind.is_empty() {
                    a.kind = kind.to_string();
                }
            }
            "task_progress" => a.activity = desc.to_string(),
            _ => {
                if a.status == AgentStatus::Running {
                    a.status = if tools::s(v, "status") == "completed" { AgentStatus::Done } else { AgentStatus::Failed };
                    a.finished = Some(Instant::now());
                }
            }
        }
    }

    fn on_subagent(&mut self, parent: &str, v: &Value) {
        if let Some(&i) = self.tool_index.get(parent) {
            if let Item::Tool(t) = &mut self.entries[i].item {
                t.sub_steps += 1;
            }
            self.touch(i);
        }
        let msg = &v["message"];
        let out_tokens = msg.get("usage").and_then(|u| u.get("output_tokens")).and_then(Value::as_u64).unwrap_or(0);
        let a = self.agent_mut(parent);
        a.tokens += out_tokens;
        for b in msg.get("content").and_then(Value::as_array).into_iter().flatten() {
            match tools::s(b, "type") {
                "tool_use" => {
                    a.tools += 1;
                    a.log.push(AgentLine::Tool {
                        id: tools::s(b, "id").to_string(),
                        name: tools::s(b, "name").to_string(),
                        input: b.get("input").cloned().unwrap_or(Value::Null),
                        state: ToolState::Running,
                    });
                }
                "text" => {
                    let t = tools::s(b, "text").trim();
                    if !t.is_empty() {
                        a.log.push(AgentLine::Text(t.to_string()));
                    }
                }
                _ => {}
            }
        }
    }

    fn on_subagent_result(&mut self, parent: &str, v: &Value) {
        let Some(blocks) = v["message"]["content"].as_array() else { return };
        let a = self.agent_mut(parent);
        for b in blocks {
            if tools::s(b, "type") != "tool_result" {
                continue;
            }
            let id = tools::s(b, "tool_use_id");
            let err = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
            for line in a.log.iter_mut().rev() {
                if let AgentLine::Tool { id: tid, state, .. } = line
                    && tid == id
                {
                    *state = if err { ToolState::Err } else { ToolState::Ok };
                    break;
                }
            }
        }
    }

    fn on_user(&mut self, v: &Value) {
        let content = &v["message"]["content"];
        let Some(blocks) = content.as_array() else {
            return;
        };
        for b in blocks {
            if tools::s(b, "type") != "tool_result" {
                continue;
            }
            let id = tools::s(b, "tool_use_id");
            let Some(&i) = self.tool_index.get(id) else { continue };
            if let Item::Tool(t) = &mut self.entries[i].item {
                let text = tools::result_text(b.get("content").unwrap_or(&Value::Null));
                let is_err = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                if t.state != ToolState::Denied {
                    t.state = if is_err { ToolState::Err } else { ToolState::Ok };
                    if !self.replaying {
                        t.finished_at = Some(Instant::now());
                    }
                }
                if !self.replaying {
                    t.took.get_or_insert_with(|| t.started.elapsed());
                }
                let ok = t.state == ToolState::Ok;
                let (name, input) = (t.name.clone(), t.input.clone());
                if ok && name == "Bash" && !self.replaying && is_test_command(tools::s(&input, "command")) {
                    self.tests_passed = true;
                }
                if let Some(ti) = self.pending_tasks.remove(id)
                    && let Some(n) = task_number(&text)
                    && let Some(todo) = self.todos.get_mut(ti)
                {
                    todo.id = n;
                }
                let launched = !self.replaying
                    && matches!(name.as_str(), "Agent" | "Task")
                    && text.starts_with("Async agent launched");
                let Item::Tool(t) = &mut self.entries[i].item else { continue };
                t.result = Some(text);
                let is_err = t.state == ToolState::Err;
                if ok {
                    self.record_change(&name, &input);
                }
                if launched {
                    // Still running in the background; the task notification ends it.
                    self.agent_mut(id).background = true;
                } else if let Some(a) = self.agents.iter_mut().find(|a| a.id == id)
                    && a.status == AgentStatus::Running
                    && !a.background
                {
                    a.status = if is_err { AgentStatus::Failed } else { AgentStatus::Done };
                    a.finished = Some(Instant::now());
                }
            }
            self.touch(i);
        }
    }

    fn on_result(&mut self, v: &Value) {
        let mut turn = Usage { turns: 1, ..Usage::default() };
        if let Some(c) = v.get("total_cost_usd").and_then(Value::as_f64) {
            turn.cost = (c - self.proc_cost).max(0.0);
            self.proc_cost = c;
            self.cost = self.cost_base + c;
        }
        if let Some(u) = v.get("usage") {
            let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
            turn.tok_in = n("input_tokens") + n("cache_read_input_tokens") + n("cache_creation_input_tokens");
            turn.tok_out = n("output_tokens");
        }
        turn.working_ms = v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0);
        self.usage.add(&turn);
        self.fresh_usage = Some(turn);
        if let Some(mu) = v.get("modelUsage").and_then(Value::as_object) {
            let key = self.model.clone().unwrap_or_default();
            let entry = mu.get(&key).or_else(|| mu.values().next());
            if let Some(w) = entry.and_then(|e| e.get("contextWindow")).and_then(Value::as_u64) {
                self.ctx_window = w;
            }
        }
        let is_err = v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
        let subtype = tools::s(v, "subtype");
        // Only Claude Code's own error result counts, never text Claude wrote.
        if !self.replaying
            && is_err
            && let Some(reset) = limit_reset(tools::s(v, "result"))
        {
            self.limit_hit = Some(reset);
        }
        // Commands like /usage answer only in the result; show it if nothing else did.
        let answered = self
            .entries
            .iter()
            .rev()
            .take_while(|e| !matches!(e.item, Item::User(_) | Item::Note(_, NoteKind::Command)))
            .any(|e| matches!(e.item, Item::Assistant(_)));
        let result_text = tools::s(v, "result").trim();
        if !self.replaying
            && let Some(cmd) = screen_only(result_text)
        {
            if let Some(pos) = self.entries.iter().rposition(|e| matches!(&e.item, Item::Assistant(t) if t.trim() == result_text)) {
                self.entries.remove(pos);
            }
            self.needs_screen = Some(cmd);
        } else if !is_err && !answered && !result_text.is_empty() {
            self.push(Item::Assistant(result_text.to_string()));
        }
        if is_err || (subtype != "success" && !subtype.is_empty()) {
            let msg = match subtype {
                "error_during_execution" => "interrupted".to_string(),
                "error_max_turns" => "stopped: max turns reached".to_string(),
                _ => {
                    let r = tools::s(v, "result");
                    if r.is_empty() { format!("error: {subtype}") } else { r.to_string() }
                }
            };
            let kind = if msg == "interrupted" { NoteKind::Info } else { NoteKind::Error };
            // A usage limit gets den's own friendlier note instead of the raw error.
            if limit_reset(&msg).is_none() {
                self.note(msg, kind);
            }
        }
        self.stop_running_tools(ToolState::Stopped);
        self.permissions.clear();
        self.thinking = false;
        self.turn_started = None;
        self.status = Status::Idle;
        self.unread = true;
        self.unread_since = Some(Instant::now());
        self.finished_at = Some(Instant::now());
    }

    fn track_todos(&mut self, use_id: &str, name: &str, input: &Value) {
        match name {
            "TodoWrite" => {
                let Some(list) = input.get("todos").and_then(Value::as_array) else { return };
                self.todos = list
                    .iter()
                    .enumerate()
                    .map(|(k, t)| Todo {
                        id: (k + 1).to_string(),
                        text: tools::s(t, "content").to_string(),
                        active_form: tools::s(t, "activeForm").to_string(),
                        state: todo_state(tools::s(t, "status")),
                    })
                    .collect();
            }
            "TaskCreate" => {
                // Finished lists make way for a fresh one.
                if !self.todos.is_empty() && self.todos.iter().all(|t| t.state == TodoState::Done) {
                    self.todos.clear();
                }
                let next = self.todos.iter().filter_map(|t| t.id.parse::<u32>().ok()).max().unwrap_or(0) + 1;
                self.todos.push(Todo {
                    id: next.to_string(),
                    text: tools::s(input, "subject").to_string(),
                    active_form: tools::s(input, "activeForm").to_string(),
                    state: TodoState::Pending,
                });
                self.pending_tasks.insert(use_id.to_string(), self.todos.len() - 1);
            }
            "TaskUpdate" => {
                let id = match input.get("taskId") {
                    Some(Value::String(s)) => s.clone(),
                    Some(v) => v.to_string(),
                    None => return,
                };
                let Some(k) = self.todos.iter().position(|t| t.id == id) else { return };
                let status = tools::s(input, "status");
                if status == "deleted" {
                    self.todos.remove(k);
                    return;
                }
                let t = &mut self.todos[k];
                if !status.is_empty() {
                    t.state = todo_state(status);
                }
                let subject = tools::s(input, "subject");
                if !subject.is_empty() {
                    t.text = subject.to_string();
                }
                let af = tools::s(input, "activeForm");
                if !af.is_empty() {
                    t.active_form = af.to_string();
                }
            }
            _ => {}
        }
    }

    fn record_change(&mut self, name: &str, input: &Value) {
        let path = tools::s(input, "file_path");
        let path = if path.is_empty() { tools::s(input, "notebook_path") } else { path };
        if path.is_empty() {
            return;
        }
        let pairs: Vec<(String, String)> = match name {
            "Edit" => vec![(tools::s(input, "old_string").to_string(), tools::s(input, "new_string").to_string())],
            "MultiEdit" => input
                .get("edits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|e| (tools::s(e, "old_string").to_string(), tools::s(e, "new_string").to_string()))
                .collect(),
            "Write" => vec![(String::new(), tools::s(input, "content").to_string())],
            "NotebookEdit" => vec![(String::new(), tools::s(input, "new_source").to_string())],
            _ => return,
        };
        let path = PathBuf::from(path);
        let k = match self.changes.iter().position(|c| c.path == path) {
            Some(k) => k,
            None => {
                self.changes.push(FileChange { path, ..FileChange::default() });
                self.changes.len() - 1
            }
        };
        let c = &mut self.changes[k];
        for (old, new) in pairs {
            let (a, r) = line_delta(&old, &new);
            c.added += a;
            c.removed += r;
            c.edits.push((old, new));
        }
    }

    /// Instructions for `/compact` that keep what matters: the user's pins,
    /// open tasks, files changed and the current goal.
    pub fn compact_instructions(&self, focus: &str) -> String {
        let mut keep = vec!["Keep the current goal and any decisions we made.".to_string()];
        if let Some(goal) = self.entries.iter().rev().find_map(|e| match &e.item {
            Item::User(t) => Some(t.lines().next().unwrap_or("").chars().take(200).collect::<String>()),
            _ => None,
        }) {
            keep.push(format!("Latest request: {goal}"));
        }
        if !self.pins.is_empty() {
            keep.push(format!("Keep these pinned notes verbatim: {}", self.pins.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join("; ")));
        }
        let open: Vec<&str> = self.todos.iter().filter(|t| t.state != TodoState::Done).map(|t| t.text.as_str()).collect();
        if !open.is_empty() {
            keep.push(format!("Open tasks: {}", open.join("; ")));
        }
        if !self.changes.is_empty() {
            let files: Vec<String> = self.changes.iter().map(|c| c.path.to_string_lossy().into_owned()).collect();
            keep.push(format!("Files changed so far: {}", files.join(", ")));
        }
        keep.push("Drop raw tool output and superseded attempts.".into());
        if !focus.trim().is_empty() {
            keep.push(format!("Also: {}", focus.trim()));
        }
        keep.join(" ")
    }

    /// Context-fill alert to show now, if the fill just crossed 70% or 85%.
    pub fn context_alert(&mut self) -> Option<u8> {
        let pct = self.ctx_pct();
        if pct < 50.0 {
            self.ctx_alerted = 0;
        }
        if pct < 60.0 {
            self.auto_compacted = false;
        }
        let level = if pct >= 85.0 { 85 } else if pct >= 70.0 { 70 } else { 0 };
        (level > self.ctx_alerted).then(|| {
            self.ctx_alerted = level;
            level
        })
    }

    /// One short line on what the session is up to, for the sidebar.
    pub fn activity(&self) -> Option<(String, ActivityKind)> {
        if !self.permissions.is_empty() {
            return Some(("waiting for you".into(), ActivityKind::Waiting));
        }
        match self.status {
            Status::Starting => Some(("starting…".into(), ActivityKind::Busy)),
            Status::Working => {
                let running = self.agents.iter().filter(|a| a.status == AgentStatus::Running).count();
                if running > 1 {
                    return Some((format!("{running} agents running"), ActivityKind::Busy));
                }
                let tool = self.entries.iter().rev().take(30).find_map(|e| match &e.item {
                    Item::Tool(t) if t.state == ToolState::Running => Some(t),
                    _ => None,
                });
                if let Some(t) = tool {
                    let (verb, target) = tools::summary(&t.name, &t.input, &self.cwd);
                    let file = target.rsplit('/').next().unwrap_or(&target).to_string();
                    let line = match t.name.as_str() {
                        "Read" => format!("reading {file}"),
                        "Edit" | "MultiEdit" | "NotebookEdit" => format!("editing {file}"),
                        "Write" => format!("writing {file}"),
                        "Bash" => format!("running {target}"),
                        "Grep" | "Glob" => "searching files".into(),
                        "WebFetch" | "WebSearch" => "browsing the web".into(),
                        "Task" | "Agent" => format!("agent: {target}"),
                        "TaskCreate" | "TaskUpdate" | "TodoWrite" => "planning".into(),
                        _ => format!("{verb} {target}"),
                    };
                    return Some((line.trim().to_string(), ActivityKind::Busy));
                }
                Some((if self.thinking { "thinking…" } else { "writing…" }.into(), ActivityKind::Busy))
            }
            Status::Idle if self.resume_at.is_some() => {
                let left = self.resume_at.unwrap_or(0).saturating_sub(now_secs());
                Some((format!("limit · resumes in {}", dur_words(left)), ActivityKind::Waiting))
            }
            Status::Idle => {
                let at = self.finished_at?;
                let word = if self.unread { "done" } else { "idle" };
                Some((format!("{word} · {}", ago(at.elapsed().as_secs())), ActivityKind::Idle))
            }
        }
    }

    fn on_control(&mut self, v: &Value) -> Option<Value> {
        let rid = tools::s(v, "request_id").to_string();
        let req = &v["request"];
        if tools::s(req, "subtype") == "can_use_tool" {
            if let Some(id) = req.get("tool_use_id").and_then(Value::as_str) {
                self.set_waiting(id, true);
            }
            if self.permissions.is_empty() {
                self.perm_since = Some(Instant::now());
            }
            self.permissions.push_back(Permission {
                request_id: rid,
                tool_name: tools::s(req, "tool_name").to_string(),
                input: req.get("input").cloned().unwrap_or(Value::Null),
                suggestions: req
                    .get("permission_suggestions")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                tool_use_id: req.get("tool_use_id").and_then(Value::as_str).map(String::from),
            });
            return None;
        }
        Some(json!({
            "type": "control_response",
            "response": {"subtype": "error", "request_id": rid, "error": "not supported by den"}
        }))
    }

    /// Replays a saved Claude Code transcript (~/.claude/projects/*/<id>.jsonl).
    #[cfg(test)]
    pub fn load_history(&mut self, jsonl: &str) {
        self.load_history_opts(jsonl, false);
    }

    /// Replays a transcript; with `live`, unfinished tools stay running because
    /// the same `claude` process is still working on them.
    /// The last few hundred user and assistant messages, as a fallback copy
    /// for /reload. `true` marks the user's own lines.
    pub fn chat_backup(&self) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = self
            .entries
            .iter()
            .rev()
            .filter_map(|e| match &e.item {
                Item::User(t) => Some((true, t.clone())),
                Item::Assistant(t) => Some((false, t.clone())),
                _ => None,
            })
            .take(400)
            .collect();
        out.reverse();
        out
    }

    pub fn restore_chat(&mut self, chat: &[(bool, String)]) {
        self.replaying = true;
        for (user, text) in chat {
            self.push(if *user { Item::User(text.clone()) } else { Item::Assistant(text.clone()) });
        }
        self.replaying = false;
    }

    pub fn load_history_opts(&mut self, jsonl: &str, live: bool) {
        self.replaying = true;
        for line in jsonl.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            if v.get("isSidechain").and_then(Value::as_bool) == Some(true)
                || v.get("isMeta").and_then(Value::as_bool) == Some(true)
            {
                continue;
            }
            match tools::s(&v, "type") {
                "assistant" => self.on_assistant(&v),
                "user" => {
                    let c = &v["message"]["content"];
                    let text = match c {
                        Value::String(s) => Some(s.clone()),
                        Value::Array(a) => a
                            .iter()
                            .find(|b| tools::s(b, "type") == "text")
                            .map(|b| tools::s(b, "text").to_string()),
                        _ => None,
                    };
                    match text {
                        Some(t) if !t.trim_start().starts_with('<') && !t.trim().is_empty() => {
                            self.push(Item::User(t));
                        }
                        _ => self.on_user(&v),
                    }
                }
                _ => {}
            }
        }
        if !live {
            self.stop_running_tools(ToolState::Stopped);
        }
        for e in &mut self.entries {
            if let Item::Tool(t) = &mut e.item {
                t.took = None;
                if live && t.state == ToolState::Running {
                    t.started = Instant::now();
                }
            }
        }
        self.replaying = false;
    }
}

/// "MCP servers" listing from Claude Code's mcp_status reply. Only names,
/// status, scope and tool counts: a server's config can hold tokens.
pub fn mcp_report(v: &Value) -> String {
    let servers = v.get("mcpServers").and_then(Value::as_array).cloned().unwrap_or_default();
    if servers.is_empty() {
        return "no MCP servers configured · add one with `claude mcp add`, then /restart".into();
    }
    let mut lines = vec![format!("MCP servers ({})", servers.len())];
    for sv in &servers {
        let status = tools::s(sv, "status");
        let mark = match status {
            "connected" => "●",
            "pending" => "◌",
            "needs-auth" => "◐",
            _ => "○",
        };
        let mut line = format!("  {mark} {}  {}", tools::s(sv, "name"), status.replace('-', " "));
        if let Some(n) = sv.get("tools").and_then(Value::as_array).map(Vec::len).filter(|n| *n > 0) {
            line.push_str(&format!(" · {n} tools"));
        }
        let scope = tools::s(sv, "scope");
        if !scope.is_empty() && scope != "dynamic" {
            line.push_str(&format!(" · {scope}"));
        }
        if let Some(e) = sv.get("error").and_then(Value::as_str) {
            line.push_str(&format!(" · {}", e.lines().next().unwrap_or("")));
        }
        lines.push(line);
    }
    lines.push("/mcp opens the menu (sign in, enable, add) · /mcp reconnect <name> retries one · /mcp list shows this".into());
    lines.join("\n")
}

pub fn auto_name(text: &str) -> String {
    let mut out = String::new();
    for w in text.split_whitespace() {
        if out.chars().count() + w.chars().count() > 22 {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(w);
    }
    if out.is_empty() {
        out = text.chars().take(22).collect();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sess() -> Session {
        Session::new(0, PathBuf::from("/p"))
    }

    #[test]
    fn command_result_shown_once() {
        let mut s = sess();
        s.user_message("/usage");
        s.handle(&json!({"type":"result","subtype":"success","result":"5h: 18% used","is_error":false}));
        assert_eq!(texts(&s), vec!["N:/usage", "A:5h: 18% used"]);
        s.user_message("hi");
        s.handle(&json!({"type":"assistant","message":{"id":"x","content":[{"type":"text","text":"hello"}]}}));
        s.handle(&json!({"type":"result","subtype":"success","result":"hello","is_error":false}));
        assert_eq!(texts(&s).len(), 4);
    }

    fn texts(s: &Session) -> Vec<String> {
        s.entries
            .iter()
            .map(|e| match &e.item {
                Item::User(t) => format!("U:{t}"),
                Item::Assistant(t) => format!("A:{t}"),
                Item::Tool(t) => format!("T:{}:{:?}", t.name, t.state),
                Item::Note(t, _) => format!("N:{t}"),
            })
            .collect()
    }

    #[test]
    fn streamed_text_not_duplicated() {
        let mut s = sess();
        s.user_message("hi");
        s.handle(&json!({"type":"stream_event","api_message_id":"m1","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}));
        s.handle(&json!({"type":"stream_event","api_message_id":"m1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hel"}}}));
        s.handle(&json!({"type":"stream_event","api_message_id":"m1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"lo"}}}));
        s.handle(&json!({"type":"assistant","message":{"id":"m1","content":[{"type":"text","text":"Hello"}]}}));
        assert_eq!(texts(&s), vec!["U:hi", "A:Hello"]);
    }

    #[test]
    fn joins_stream_midway_then_fills_full_text() {
        let mut s = sess();
        s.handle(&json!({"type":"stream_event","api_message_id":"m","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"…end of story"}}}));
        assert_eq!(texts(&s), vec!["A:…end of story"]);
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"Start of story …end of story"}]}}));
        assert_eq!(texts(&s), vec!["A:Start of story …end of story"]);
    }

    #[test]
    fn tool_lifecycle_and_permission() {
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]}}));
        let reply = s.handle(&json!({"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"tool_use_id":"t1"}}));
        assert!(reply.is_none());
        assert_eq!(s.permissions.len(), 1);
        s.handle(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"a\nb","is_error":false}]}}));
        assert_eq!(texts(&s), vec!["U:go", "T:Bash:Ok"]);
        s.handle(&json!({"type":"result","subtype":"success","total_cost_usd":0.5,"is_error":false}));
        assert_eq!(s.status, Status::Idle);
        assert!((s.cost - 0.5).abs() < 1e-9);
    }

    #[test]
    fn unknown_control_gets_error_reply() {
        let mut s = sess();
        let r = s.handle(&json!({"type":"control_request","request_id":"x","request":{"subtype":"mystery"}})).unwrap();
        assert_eq!(r["response"]["subtype"], "error");
    }

    #[test]
    fn reveal_advances_on_char_boundaries() {
        let mut s = sess();
        s.handle(&json!({"type":"stream_event","api_message_id":"m","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}));
        s.handle(&json!({"type":"stream_event","api_message_id":"m","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"héllo wörld 👋 ünïcode"}}}));
        for _ in 0..200 {
            s.advance(0.001, true);
        }
        assert_eq!(s.entries[0].shown, s.entries[0].item_len());
    }

    #[test]
    fn subagents_tracked_from_events() {
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"A1","name":"Agent","input":{"description":"Read README","subagent_type":"general-purpose"}}]}}));
        s.handle(&json!({"type":"system","subtype":"task_started","tool_use_id":"A1","description":"Read README.md","subagent_type":"general-purpose"}));
        s.handle(&json!({"type":"assistant","parent_tool_use_id":"A1","message":{"id":"x","content":[{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"/p/README.md"}}],"usage":{"output_tokens":40}}}));
        s.handle(&json!({"type":"system","subtype":"task_progress","tool_use_id":"A1","description":"Reading README.md"}));
        s.handle(&json!({"type":"user","parent_tool_use_id":"A1","message":{"content":[{"type":"tool_result","tool_use_id":"r1","content":"hi"}]}}));
        s.handle(&json!({"type":"assistant","parent_tool_use_id":"A1","message":{"id":"y","content":[{"type":"text","text":"It says hi."}]}}));
        let a = &s.agents[0];
        assert_eq!((a.desc.as_str(), a.kind.as_str(), a.tools, a.tokens), ("Read README.md", "general-purpose", 1, 40));
        assert!(matches!(a.log[0], AgentLine::Tool { state: ToolState::Ok, .. }));
        assert!(matches!(&a.log[1], AgentLine::Text(t) if t == "It says hi."));
        assert_eq!(a.activity, "Reading README.md");
        s.handle(&json!({"type":"system","subtype":"task_notification","tool_use_id":"A1","status":"completed"}));
        assert_eq!(s.agents[0].status, AgentStatus::Done);
    }

    fn tool(s: &mut Session, id: &str, name: &str, input: Value, result: &str) {
        s.handle(&json!({"type":"assistant","message":{"id":id,"content":[{"type":"tool_use","id":id,"name":name,"input":input}]}}));
        s.handle(&json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":id,"content":result}]}}));
    }

    #[test]
    fn task_list_follows_create_and_update() {
        let mut s = sess();
        s.user_message("go");
        tool(&mut s, "c1", "TaskCreate", json!({"subject":"Scaffold","activeForm":"Scaffolding"}), "Task #4 created successfully: Scaffold");
        tool(&mut s, "c2", "TaskCreate", json!({"subject":"Test"}), "Task #5 created successfully: Test");
        assert_eq!(s.todos.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["4", "5"]);
        tool(&mut s, "u1", "TaskUpdate", json!({"taskId":"4","status":"in_progress"}), "Updated task #4 status");
        assert_eq!(s.todos[0].state, TodoState::Active);
        tool(&mut s, "u2", "TaskUpdate", json!({"taskId":"4","status":"completed"}), "ok");
        tool(&mut s, "u3", "TaskUpdate", json!({"taskId":"5","status":"deleted"}), "ok");
        assert_eq!(s.todos.len(), 1);
        assert_eq!(s.todos[0].state, TodoState::Done);
        tool(&mut s, "w", "TodoWrite", json!({"todos":[{"content":"a","status":"pending"},{"content":"b","status":"in_progress","activeForm":"Doing b"}]}), "ok");
        assert_eq!(s.todos.len(), 2);
        assert_eq!(s.todos[1].active_form, "Doing b");
    }

    #[test]
    fn file_changes_counted() {
        let mut s = sess();
        s.user_message("go");
        tool(&mut s, "e1", "Edit", json!({"file_path":"/p/a.rs","old_string":"x\ny\nz","new_string":"x\nY\nY2\nz"}), "ok");
        tool(&mut s, "w1", "Write", json!({"file_path":"/p/b.rs","content":"1\n2\n3"}), "ok");
        tool(&mut s, "e2", "Edit", json!({"file_path":"/p/a.rs","old_string":"q","new_string":""}), "ok");
        assert_eq!(s.changes.len(), 2);
        assert_eq!((s.changes[0].added, s.changes[0].removed, s.changes[0].edits.len()), (2, 2, 2));
        assert_eq!((s.changes[1].added, s.changes[1].removed), (3, 0));
        assert_eq!(line_delta("a\nb", "a\nb"), (0, 0));
    }

    #[test]
    fn activity_and_test_detection() {
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"t","name":"Edit","input":{"file_path":"/p/src/fetch.rs"}}]}}));
        assert_eq!(s.activity().unwrap().0, "editing fetch.rs");
        tool(&mut s, "b", "Bash", json!({"command":"cargo test --quiet"}), "ok");
        assert!(s.tests_passed);
        s.handle(&json!({"type":"result","subtype":"success","is_error":false}));
        assert!(s.activity().unwrap().0.starts_with("done"));
    }

    #[test]
    fn background_agents_run_until_notified() {
        let mut s = sess();
        s.user_message("go");
        tool(&mut s, "A1", "Agent", json!({"description":"Build video","subagent_type":"fork"}), "Async agent launched successfully. (internal)");
        assert_eq!(s.agents[0].status, AgentStatus::Running);
        s.handle(&json!({"type":"system","subtype":"task_progress","tool_use_id":"A1","description":"Rendering"}));
        assert_eq!(s.agents[0].status, AgentStatus::Running);
        s.handle(&json!({"type":"system","subtype":"task_notification","tool_use_id":"A1","status":"completed"}));
        assert_eq!(s.agents[0].status, AgentStatus::Done);
    }

    #[test]
    fn subagent_background_bash_is_not_an_agent() {
        let mut s = sess();
        s.user_message("go");
        tool(&mut s, "A1", "Agent", json!({"description":"x"}), "Async agent launched successfully.");
        s.handle(&json!({"type":"assistant","parent_tool_use_id":"A1","message":{"id":"x","content":[{"type":"tool_use","id":"sb","name":"Bash","input":{"command":"sleep 12","run_in_background":true}}]}}));
        s.handle(&json!({"type":"system","subtype":"task_started","tool_use_id":"sb","description":"Sleep for 12"}));
        assert_eq!(s.agents.len(), 1);
    }

    #[test]
    fn background_bash_is_not_an_agent() {
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"tool_use","id":"b1","name":"Bash","input":{"command":"sleep 5","run_in_background":true}}]}}));
        s.handle(&json!({"type":"system","subtype":"task_started","tool_use_id":"b1","description":"Sleep for 5"}));
        assert!(s.agents.is_empty());
    }

    #[test]
    fn output_after_result_means_working_again() {
        let mut s = sess();
        s.user_message("one");
        s.handle(&json!({"type":"result","subtype":"success","is_error":false}));
        assert_eq!(s.status, Status::Idle);
        s.handle(&json!({"type":"stream_event","api_message_id":"m2","event":{"type":"message_start"}}));
        assert_eq!(s.status, Status::Working);
    }

    #[test]
    fn usage_limit_replies_are_detected() {
        assert_eq!(limit_reset("Claude AI usage limit reached|1736000000"), Some(1736000000));
        assert_eq!(limit_reset("You've hit your limit · resets 3pm (Europe/London)"), Some(0));
        assert_eq!(limit_reset("All tests pass."), None);
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"result","subtype":"success","is_error":true,"result":"Claude AI usage limit reached|1736000000"}));
        assert_eq!(s.limit_hit, Some(1736000000));
        // Claude merely saying the words is not a limit.
        let mut s = sess();
        s.user_message("go");
        s.handle(&json!({"type":"assistant","message":{"id":"m","content":[{"type":"text","text":"Claude AI usage limit reached|1"}]}}));
        s.handle(&json!({"type":"result","subtype":"success","is_error":false,"result":"Claude AI usage limit reached|1"}));
        assert_eq!(s.limit_hit, None);
    }

    #[test]
    fn compaction_keeps_pins_and_alerts_once() {
        let mut s = sess();
        s.user_message("ship the release");
        s.pins.push("never touch prod config".into());
        let ins = s.compact_instructions("");
        assert!(ins.contains("never touch prod config") && ins.contains("ship the release"));
        s.handle(&json!({"type":"system","subtype":"compact_boundary"}));
        assert!(s.repin);
        s.ctx_window = 100;
        s.ctx_used = 72;
        assert_eq!(s.context_alert(), Some(70));
        assert_eq!(s.context_alert(), None);
        s.ctx_used = 90;
        assert_eq!(s.context_alert(), Some(85));
        s.ctx_used = 10;
        assert_eq!(s.context_alert(), None);
        assert_eq!(s.ctx_alerted, 0);
    }

    #[test]
    fn agent_cards_survive_a_snapshot() {
        let mut s = sess();
        s.user_message("go");
        tool(&mut s, "A1", "Agent", json!({"description":"Security review"}), "Async agent launched successfully.");
        s.handle(&json!({"type":"assistant","parent_tool_use_id":"A1","message":{"id":"x","content":[{"type":"tool_use","id":"r1","name":"Read","input":{"file_path":"/p/a.ts"}}]}}));
        let json = serde_json::to_string(&s.agents[0].snap()).unwrap();
        let back = Agent::from_snap(serde_json::from_str(&json).unwrap());
        assert_eq!((back.desc.as_str(), back.status, back.log.len(), back.background), ("Security review", AgentStatus::Running, 1, true));
    }

    #[test]
    fn auto_name_truncates_on_words() {
        assert_eq!(auto_name("add retry logic to the fetcher module please"), "add retry logic to the");
    }

    #[test]
    fn history_skips_meta_and_commands() {
        let mut s = sess();
        let h = [
            r#"{"type":"user","message":{"content":"<command-name>/x</command-name>"}}"#,
            r#"{"type":"user","isMeta":true,"message":{"content":"meta"}}"#,
            r#"{"type":"user","message":{"content":"real question"}}"#,
            r#"{"type":"assistant","message":{"id":"a","content":[{"type":"text","text":"answer"}]}}"#,
        ]
        .join("\n");
        s.load_history(&h);
        assert_eq!(texts(&s), vec!["U:real question", "A:answer"]);
    }
    #[test]
    fn rebuilds_running_agents_from_transcript() {
        let j = [
            r#"{"type":"assistant","timestamp":"2026-11-17T15:00:00.000Z","message":{"content":[{"type":"tool_use","id":"toolu_A","name":"Agent","input":{"description":"fg","subagent_type":"Explore"}},{"type":"tool_use","id":"toolu_B","name":"Agent","input":{"description":"bg"}},{"type":"tool_use","id":"toolu_C","name":"Agent","input":{"description":"done"}},{"type":"tool_use","id":"toolu_D","name":"Agent","input":{"description":"bg done"}}]}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"toolu_B","content":"Async agent launched successfully"},{"type":"tool_result","tool_use_id":"toolu_C","content":"all good"},{"type":"tool_result","tool_use_id":"toolu_D","content":"Async agent launched successfully"}]}}"#,
            r#"{"type":"user","message":{"content":"<task-notification>\n<tool-use-id>toolu_D</tool-use-id>\n</task-notification>"}}"#,
        ]
        .join("\n");
        let a = running_agents(&j);
        let ids: Vec<&str> = a.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, ["toolu_A", "toolu_B"]);
        assert_eq!(a[0].kind, "Explore");
        assert!(!a[0].background && a[1].background);
        assert!(a[0].started.elapsed().as_secs() > 60);
        assert_eq!(stamp_secs("1970-01-02T00:00:01Z"), Some(86_401));
    }

    #[test]
    fn spots_commands_that_need_claude_codes_screen() {
        assert_eq!(super::screen_only("/hooks isn't available in this environment."), Some("/hooks".into()));
        assert_eq!(super::screen_only("/add-dir ../x isn't available in this environment."), Some("/add-dir ../x".into()));
        assert_eq!(super::screen_only("Sure, hooks isn't available in this environment."), None);
        assert_eq!(super::screen_only("Done."), None);
    }

    #[test]
    fn mcp_report_lists_servers_without_config() {
        let v = json!({"mcpServers": [
            {"name": "github", "status": "connected", "scope": "user", "tools": [{"name": "a"}, {"name": "b"}], "config": {"headers": {"Authorization": "Bearer SECRET"}}},
            {"name": "plugin:x", "status": "needs-auth", "scope": "dynamic"}
        ]});
        let r = mcp_report(&v);
        assert!(r.contains("● github  connected · 2 tools · user"));
        assert!(r.contains("◐ plugin:x  needs auth"));
        assert!(!r.contains("SECRET"));
        assert!(mcp_report(&json!({})).contains("no MCP servers"));
    }

}
