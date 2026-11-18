use crate::claude::{self, AppEvent, Proc, Spawn};
use crate::files;
use crate::input::Editor;
use crate::session::{Item, NoteKind, Session};
use crate::store::{self, Saved};
use crate::tools;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use ratatui::layout::Rect;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Focus {
    Input,
    Transcript,
}

pub enum Modal {
    Project { query: Editor, sel: usize },
    Tutorial { step: usize },
    Switcher { query: Editor, sel: usize },
    Help,
    Stats,
    Agents,
    Board(Board),
    Model { sel: usize, effort: usize },
    /// `prev` is the theme to restore on esc.
    Theme { sel: usize, prev: usize },
    Settings { sel: usize },
    Files { sel: usize, scroll: u16 },
    Search { query: Editor, sel: usize },
}

/// A model from Claude Code's catalog.
#[derive(Clone, Debug)]
pub struct ModelOpt {
    pub value: String,
    pub name: String,
    pub desc: String,
    pub resolved: String,
    pub efforts: Vec<String>,
}

fn parse_models(v: &Value) -> Vec<ModelOpt> {
    v.get("models")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|m| {
            let value = m.get("value")?.as_str()?.to_string();
            Some(ModelOpt {
                name: tools::s(m, "displayName").to_string(),
                desc: tools::s(m, "description").to_string(),
                resolved: tools::s(m, "resolvedModel").to_string(),
                efforts: m
                    .get("supportedEffortLevels")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default(),
                value,
            })
        })
        .collect()
}

fn fallback_models() -> Vec<ModelOpt> {
    let levels: Vec<String> = ["low", "medium", "high", "xhigh", "max"].iter().map(|s| s.to_string()).collect();
    [("opus", "Opus"), ("sonnet", "Sonnet"), ("haiku", "Haiku"), ("fable", "Fable")]
        .iter()
        .map(|(v, n)| ModelOpt {
            value: v.to_string(),
            name: n.to_string(),
            desc: String::new(),
            resolved: String::new(),
            efforts: levels.clone(),
        })
        .collect()
}

/// A search match: session index, entry index, and the matching line.
pub struct Hit {
    pub session: usize,
    pub entry: usize,
    pub line: String,
    pub role: &'static str,
}

pub fn search_hits(sessions: &[Session], q: &str) -> Vec<Hit> {
    let q = q.trim().to_lowercase();
    let mut out = Vec::new();
    if q.len() < 2 {
        return out;
    }
    for (si, s) in sessions.iter().enumerate() {
        for (ei, e) in s.entries.iter().enumerate().rev().filter(|(_, e)| !crate::session::quiet(&e.item)) {
            let (text, role): (String, &'static str) = match &e.item {
                Item::User(t) => (t.clone(), "you"),
                Item::Assistant(t) => (t.clone(), "claude"),
                Item::Note(t, _) => (t.clone(), "note"),
                Item::Tool(t) => {
                    let (verb, target) = tools::summary(&t.name, &t.input, &s.cwd);
                    (format!("{verb} {target}"), "tool")
                }
            };
            if let Some(line) = text.lines().find(|l| l.to_lowercase().contains(&q)) {
                out.push(Hit { session: si, entry: ei, line: line.trim().to_string(), role });
                if out.len() >= 200 {
                    return out;
                }
            }
        }
    }
    out
}

/// Shows a desktop notification (no-op with DEN_NO_NOTIFY set).
fn notify(title: &str, body: &str) {
    if std::env::var_os("DEN_NO_NOTIFY").is_none() {
        crate::notify::send(title, body);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoardFocus {
    Folders,
    Projects,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BoardInput {
    AddFolder,
    NewProject,
    Filter,
}

pub struct Board {
    pub folder: usize,
    pub sel: usize,
    pub focus: BoardFocus,
    pub input: Option<(BoardInput, Editor)>,
    pub filter: String,
}

/// Click targets inside the projects board.
#[derive(Clone)]
pub enum BoardHit {
    Folder(usize),
    Project(usize),
    AddFolder,
    NewProject,
    RemoveFolder,
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum PopupKind {
    Slash,
    File,
    Dir,
}

/// What a click in the sidebar does.
#[derive(Clone)]
pub enum SideHit {
    Session(usize),
    Close(usize),
    NewIn(PathBuf),
    NewButton,
    Board,
    Files,
    Reload,
    Settings,
}

/// Clicks on split-view panes.
#[derive(Clone, Copy)]
pub enum PaneHit {
    Split,
    Close(usize),
    Focus(usize),
    SendQueued(usize),
    DropQueued(usize),
}

/// Narrowest main area that still splits into two panes.
pub const SPLIT_MIN_W: u16 = 90;

pub struct Popup {
    pub kind: PopupKind,
    pub start: usize,
    pub items: Vec<(String, String)>,
    pub sel: usize,
}

/// Commands meant for programs driving Claude Code, not for people.
const HIDDEN_COMMANDS: &[&str] = &["auto-mode-setup"];

pub const LOCAL_COMMANDS: &[(&str, &str)] = &[
    ("cd", "jump into a folder  <path>"),
    ("workspace", "add a folder of projects  add|remove|list <path>"),
    ("style", "how much Claude explains  default|Explanatory|Concise"),
    ("reload", "restart den, keep every session"),
    ("update", "install the newest den release"),
    ("new", "new session  [path]"),
    ("close", "close this session"),
    ("rename", "rename this session"),
    ("clear", "start fresh in this directory"),
    ("model", "pick model and effort  [name]"),
    ("effort", "thinking effort  low|medium|high|xhigh|max|auto"),
    ("files", "files Claude changed, with diffs"),
    ("settings", "theme, typing effect, notifications, updates"),
    ("theme", "pick a color theme  [name]"),
    ("plugin", "install and manage Claude Code plugins  list|install|marketplace add …"),
    ("split", "two sessions side by side  [off]"),
    ("search", "search every conversation"),
    ("mode", "permission mode  default|acceptEdits|plan|auto"),
    ("stats", "tokens, cost and time"),
    ("agents", "see this session's subagents"),
    ("copy", "copy the last reply"),
    ("help", "keys and commands"),
    ("tutorial", "replay the intro tour"),
    ("quit", "exit den"),
];

pub struct App {
    pub sessions: Vec<Session>,
    procs: HashMap<usize, (u64, Proc)>,
    pub active: usize,
    pub focus: Focus,
    pub modal: Option<Modal>,
    pub editor: Editor,
    pub popup: Option<Popup>,
    pub sidebar: bool,
    pub expand_all: bool,
    pub spinner: usize,
    pub toast: Option<(String, Instant)>,
    pub quit: bool,
    pub page: usize,
    pub sidebar_rows: Vec<(Rect, SideHit)>,
    pub board_hits: Vec<(Rect, BoardHit)>,
    pub agent_hits: Vec<Rect>,
    pub sidebar_visible: bool,
    files: HashMap<PathBuf, Option<Arc<Vec<String>>>>,
    tx: UnboundedSender<AppEvent>,
    next_sid: usize,
    next_gen: u64,
    launch_dir: PathBuf,
    last_ctrl_c: Option<Instant>,
    pub recent_projects: Vec<PathBuf>,
    pub catalog: Vec<(String, String)>,
    pub lifetime: store::Lifetime,
    /// Mouse selection in screen cells: (anchor, cursor), plus copy-on-release.
    pub selection: Option<((u16, u16), (u16, u16))>,
    pub copy_pending: bool,
    pub text_rect: Rect,
    pub modal_at: Option<Instant>,
    pub step_at: Option<Instant>,
    pub switched_at: Option<Instant>,
    pub last_tick: Instant,
    pub last_activity: Instant,
    pub reload_pending: bool,
    pub workspaces: Vec<PathBuf>,
    /// Last mouse position, for hover highlights.
    pub hover: Option<(u16, u16)>,
    available_styles: Vec<String>,
    pub models: Vec<ModelOpt>,
    pub reaction: Option<(crate::ui::companion::Reaction, Instant)>,
    /// Whether the terminal window has focus (for notifications).
    pub term_focused: bool,
    pub model_hit: Rect,
    /// Split view: (left, right) session ids, and which side has focus.
    pub split: Option<(usize, usize)>,
    split_side: usize,
    pub pane_hits: Vec<(Rect, PaneHit)>,
    /// Highest warning level (1 = 80%, 2 = 95%) already shown per limit window.
    limit_warned: HashMap<String, u8>,
    /// A newer den was installed on disk; /reload switches to it.
    pub update_ready: Option<String>,
    pub prefs: Prefs,
    outfit_today: Option<crate::ui::companion::Outfit>,
    /// den's own path, for the output-squeezing hook (None when RTK does that job).
    squeeze_hook: Option<String>,
    pub settings_hits: Vec<Rect>,
}

/// Settings read on every frame, mirrored from config.json.
#[derive(Clone, Copy)]
pub struct Prefs {
    pub typing_effect: bool,
    pub notifications: bool,
    pub auto_update: bool,
    pub outfit: bool,
    pub squeeze: bool,
    pub auto_resume: bool,
}

impl Prefs {
    fn load() -> Self {
        let c = store::load_config();
        Prefs {
            typing_effect: c.typing_effect.unwrap_or(true),
            notifications: c.notifications.unwrap_or(true),
            auto_update: c.auto_update.unwrap_or(true),
            outfit: c.outfit.unwrap_or(true),
            squeeze: c.squeeze.unwrap_or(true),
            auto_resume: c.auto_resume.unwrap_or(true),
        }
    }

    fn save(&self) {
        let mut c = store::load_config();
        c.typing_effect = Some(self.typing_effect);
        c.notifications = Some(self.notifications);
        c.auto_update = Some(self.auto_update);
        c.outfit = Some(self.outfit);
        c.squeeze = Some(self.squeeze);
        c.auto_resume = Some(self.auto_resume);
        store::save_config(&c);
    }
}

/// Rows of the settings panel: (label, hint).
pub const SETTINGS: &[(&str, &str)] = &[
    ("Theme", "colors for the whole app"),
    ("Typing effect", "cursor and fade-in while Claude writes"),
    ("Notifications", "when Claude finishes or needs you (silent)"),
    ("Auto-update", "install new den releases in the background"),
    ("Seasonal outfit", "Santa hat in December, shades in summer…"),
    ("Squeeze output", "trim noisy command output to save tokens"),
    ("Auto-resume", "continue by itself when a usage limit resets"),
];


impl App {
    pub fn new(tx: UnboundedSender<AppEvent>, launch_dir: PathBuf, open_new: bool, reloaded: bool) -> Self {
        let mut app = Self {
            sessions: Vec::new(),
            procs: HashMap::new(),
            active: 0,
            focus: Focus::Input,
            modal: None,
            editor: Editor::default(),
            popup: None,
            sidebar: true,
            expand_all: false,
            spinner: 0,
            toast: None,
            quit: false,
            page: 20,
            sidebar_rows: Vec::new(),
            board_hits: Vec::new(),
            agent_hits: Vec::new(),
            sidebar_visible: true,
            files: HashMap::new(),
            tx,
            next_sid: 0,
            next_gen: 0,
            launch_dir: launch_dir.clone(),
            last_ctrl_c: None,
            recent_projects: Vec::new(),
            catalog: Vec::new(),
            lifetime: store::load_lifetime(),
            selection: None,
            copy_pending: false,
            text_rect: Rect::default(),
            modal_at: None,
            step_at: None,
            switched_at: None,
            last_tick: Instant::now(),
            last_activity: Instant::now(),
            reload_pending: false,
            workspaces: {
                let c = store::load_config();
                if let Some(t) = &c.theme {
                    crate::theme::set(t);
                }
                c.workspaces
            },
            hover: None,
            available_styles: Vec::new(),
            models: Vec::new(),
            reaction: None,
            term_focused: true,
            model_hit: Rect::default(),
            split: None,
            split_side: 0,
            pane_hits: Vec::new(),
            limit_warned: HashMap::new(),
            update_ready: None,
            prefs: Prefs::load(),
            outfit_today: crate::ui::companion::Outfit::today(),
            squeeze_hook: (!crate::squeeze::rtk_active())
                .then(|| std::env::current_exe().ok())
                .flatten()
                .map(|p| p.to_string_lossy().into_owned()),
            settings_hits: Vec::new(),
        };
        {
            let tx = app.tx.clone();
            tokio::task::spawn_blocking(move || {
                let _ = tx.send(AppEvent::Projects(crate::projects::discover()));
            });
            claude::fetch_catalog(launch_dir.clone(), app.tx.clone());
            if app.prefs.auto_update && crate::update::due() {
                app.check_update(false);
            }
        }
        if app.lifetime.since.is_none() {
            app.lifetime.since = Some(today());
        }
        let snapshot = if reloaded { store::take_snapshot() } else { None };
        let saved_list = match &snapshot {
            Some(snap) => snap.sessions.clone(),
            None => store::load(),
        };
        for saved in saved_list {
            if !saved.cwd.is_dir() {
                continue;
            }
            let mut s = app.make_session(saved.cwd.clone());
            s.name = saved.name;
            s.auto_named = saved.auto_named;
            s.model = saved.model;
            s.mode = saved.mode.unwrap_or_else(|| "default".into());
            s.style = saved.style.unwrap_or_else(|| "default".into());
            s.ctx_window = saved.ctx_window;
            s.effort = saved.effort;
            s.resume_at = saved.resume_at;
            s.usage = saved.usage;
            let live = saved.live.clone();
            let claude_id = saved.claude_id.clone().or_else(|| live.as_ref().and_then(|l| l.claude_id.clone()));
            if let Some(id) = &claude_id {
                if let Some(h) = store::transcript(id) {
                    s.load_history_opts(&h, live.as_ref().is_some_and(|l| l.busy));
                }
                s.claude_id = Some(id.clone());
            }
            if let Some(l) = live {
                app.next_gen += 1;
                let generation = app.next_gen;
                match claude::adopt(s.sid, generation, l.handoff, app.tx.clone()) {
                    Ok(p) => {
                        app.procs.insert(s.sid, (generation, p));
                        if l.busy {
                            s.status = crate::session::Status::Working;
                            s.turn_started = Instant::now().checked_sub(Duration::from_millis(l.turn_ms));
                        }
                        if !l.permissions.is_empty() {
                            s.perm_since = Some(Instant::now());
                        }
                        s.permissions = l.permissions.into();
                        s.queue = l.queue.into();
                    }
                    Err(e) => s.note(format!("couldn't reattach: {e:#}"), NoteKind::Error),
                }
            }
            if saved.ctx_used > 0 {
                s.ctx_used = saved.ctx_used;
            }
            s.ctx_shown = s.ctx_pct();
            s.unread = false;
            app.sessions.push(s);
        }
        let existing = app.sessions.iter().position(|s| s.cwd == launch_dir && s.entries.is_empty());
        match existing {
            Some(i) if !open_new => app.active = i,
            _ if open_new || app.sessions.is_empty() => {
                let s = app.make_session(launch_dir);
                app.sessions.push(s);
                app.active = app.sessions.len() - 1;
            }
            _ => app.active = app.sessions.len() - 1,
        }
        if let Some(snap) = snapshot {
            app.active = snap.active.min(app.sessions.len().saturating_sub(1));
            app.editor.set(&snap.draft);
            app.editor.set_history(snap.history);
            app.toast("reloaded");
        }
        app.regroup();
        // New people get the tour; everyone sees a new version of it once,
        // even when arriving through /reload.
        let seen = store::load_config().tour_version.unwrap_or(0);
        if seen < crate::ui::tutorial::VERSION {
            app.open_modal(Modal::Tutorial { step: 0 });
        }
        app
    }

    /// Keeps sessions of the same project adjacent, in first-seen order.
    fn regroup(&mut self) {
        let active_sid = self.sessions.get(self.active).map(|s| s.sid);
        let mut order: Vec<PathBuf> = Vec::new();
        for s in &self.sessions {
            if !order.contains(&s.cwd) {
                order.push(s.cwd.clone());
            }
        }
        self.sessions.sort_by_key(|s| order.iter().position(|p| *p == s.cwd));
        if let Some(sid) = active_sid {
            self.active = self.sessions.iter().position(|s| s.sid == sid).unwrap_or(0);
        }
    }

    /// Project folders for the picker: current, open, then recent.
    pub fn project_choices(&self) -> Vec<PathBuf> {
        let mut out = vec![self.current().cwd.clone()];
        let ws_children: Vec<PathBuf> = self.all_workspaces().iter().flat_map(|w| crate::projects::children(w)).collect();
        for p in self.sessions.iter().map(|s| &s.cwd).chain(self.recent_projects.iter()).chain(ws_children.iter()) {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }

    fn add_workspace(&mut self, path: &str) {
        let p = expand(path);
        let p = if p.is_absolute() { p } else { self.current().cwd.join(p) };
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !p.is_dir() {
            self.toast(format!("not a folder: {path}"));
            return;
        }
        if !self.workspaces.contains(&p) {
            self.workspaces.push(p.clone());
            let mut c = store::load_config();
            c.workspaces = self.workspaces.clone();
            c.hidden_workspaces.retain(|h| *h != p);
            store::save_config(&c);
        }
        let n = crate::projects::children(&p).len();
        self.toast(format!("workspace {} · {n} projects · ^N to open one", tools::tilde(&p.to_string_lossy())));
    }

    fn remove_workspace(&mut self, path: &str) {
        let p = expand(path);
        let before = self.workspaces.len();
        self.workspaces.retain(|w| *w != p && !w.ends_with(path.trim_end_matches('/')));
        let mut c = store::load_config();
        c.workspaces = self.workspaces.clone();
        store::save_config(&c);
        self.toast(if self.workspaces.len() < before { "workspace removed" } else { "no such workspace" });
    }

    pub fn finish_tutorial(&mut self) {
        self.modal = None;
        let mut c = store::load_config();
        let first = !c.onboarded;
        c.onboarded = true;
        c.tour_version = Some(crate::ui::tutorial::VERSION);
        let pick_theme = first && c.theme.is_none();
        store::save_config(&c);
        if pick_theme {
            self.open_theme_picker();
        }
    }

    pub fn open_theme_picker(&mut self) {
        let cur = crate::theme::current_index();
        self.open_modal(Modal::Theme { sel: cur, prev: cur });
    }

    fn notify(&self, title: &str, body: &str) {
        if self.prefs.notifications {
            notify(title, body);
        }
    }

    fn toggle_setting(&mut self, row: usize) {
        let p = &mut self.prefs;
        match row {
            0 => return self.open_theme_picker(),
            1 => p.typing_effect = !p.typing_effect,
            2 => p.notifications = !p.notifications,
            3 => p.auto_update = !p.auto_update,
            4 => p.outfit = !p.outfit,
            5 => p.squeeze = !p.squeeze,
            6 => p.auto_resume = !p.auto_resume,
            _ => {}
        }
        self.prefs.save();
    }

    fn save_theme(&mut self) {
        let name = crate::theme::p().name;
        let mut c = store::load_config();
        c.theme = Some(name.to_string());
        store::save_config(&c);
        self.toast(format!("theme: {name}"));
    }

    fn make_session(&mut self, cwd: PathBuf) -> Session {
        let sid = self.next_sid;
        self.next_sid += 1;
        Session::new(sid, cwd)
    }

    pub fn current(&self) -> &Session {
        &self.sessions[self.active]
    }

    pub fn current_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.active]
    }

    pub fn open_modal(&mut self, m: Modal) {
        self.modal = Some(m);
        self.modal_at = Some(Instant::now());
        self.step_at = None;
    }

    /// How soon the next animation frame is needed; None when fully idle.
    pub fn frame_interval(&self) -> Option<Duration> {
        use crate::anim::{FADE_IN, FLASH, OPEN, PULSE, SWITCH, active};
        let s = self.current();
        let recent = s.entries.iter().rev().take(12).any(|e| {
            active(e.born, FADE_IN)
                || e.shown < e.item_len()
                || matches!(&e.item, Item::Tool(t) if active(t.finished_at, FLASH))
        });
        let fast = recent
            || s.scroll_pos != s.scroll as f32
            || s.ctx_shown != s.ctx_pct()
            || active(s.perm_since, OPEN)
            || active(self.modal_at, OPEN)
            || active(self.step_at, SWITCH)
            || active(self.switched_at, SWITCH)
            || self.reaction_now().is_some()
            || s.jump.is_some_and(|(_, at)| at.elapsed() < JUMP_FLASH)
            || self.sessions.iter().any(|x| x.unread && active(x.unread_since, PULSE));
        if fast {
            return Some(Duration::from_millis(16));
        }
        let busy = self.sessions.iter().any(Session::busy)
            || self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() < Duration::from_secs(3));
        if busy {
            return Some(Duration::from_millis(33));
        }
        // Background subagents keep their timers ticking.
        if matches!(self.modal, Some(Modal::Tutorial { .. })) {
            return Some(Duration::from_millis(33));
        }
        // Waiting out a usage limit: tick once a second for the countdown and the resume.
        if self.sessions.iter().any(|s| s.resume_at.is_some()) {
            return Some(Duration::from_secs(1));
        }
        if self.sessions.iter().any(|s| s.agents.iter().any(|a| a.status == crate::session::AgentStatus::Running)) {
            return Some(Duration::from_millis(250));
        }
        // Clawd blinks and glances; stop once he's in deep sleep.
        (self.idle_secs() < crate::ui::companion::DEEP_SLEEP_AFTER).then_some(Duration::from_millis(150))
    }

    pub fn tick(&mut self) {
        self.spinner = self.spinner.wrapping_add(1);
        let dt = self.last_tick.elapsed().as_secs_f32().min(0.1);
        self.last_tick = Instant::now();
        let active = self.active;
        for (i, s) in self.sessions.iter_mut().enumerate() {
            s.advance(dt, i == active);
        }
        if self.toast.as_ref().is_some_and(|(_, t)| t.elapsed() >= Duration::from_secs(3)) {
            self.toast = None;
        }
        self.run_due_resumes();
    }

    /// The running Clawd reaction and its progress.
    pub fn reaction_now(&self) -> Option<(crate::ui::companion::Reaction, f32)> {
        let (r, at) = self.reaction?;
        let p = at.elapsed().as_secs_f32() / r.secs();
        (p < 1.0).then_some((r, p))
    }

    fn react(&mut self, r: crate::ui::companion::Reaction) {
        self.reaction = Some((r, Instant::now()));
        self.last_activity = Instant::now();
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    fn saved(s: &Session) -> Saved {
        Saved {
            name: s.name.clone(),
            auto_named: s.auto_named,
            cwd: s.cwd.clone(),
            claude_id: s.claude_id.clone(),
            model: s.model.clone(),
            mode: Some(s.mode.clone()),
            usage: s.usage,
            style: Some(s.style.clone()),
            ctx_used: s.ctx_used,
            ctx_window: s.ctx_window,
            effort: s.effort.clone(),
            resume_at: s.resume_at,
            live: None,
        }
    }

    pub fn persist(&self) {
        let saved: Vec<Saved> = self.sessions.iter().filter(|s| s.claude_id.is_some()).map(Self::saved).collect();
        store::save(&saved);
    }

    pub fn reload_ready(&self) -> bool {
        self.reload_pending
    }

    /// Hands every running `claude` over to the next den and writes the snapshot.
    /// `rx` is drained after readers stop so no received event is lost.
    pub async fn write_snapshot(&mut self, rx: &mut tokio::sync::mpsc::UnboundedReceiver<AppEvent>) -> bool {
        let mut handoffs = HashMap::new();
        // 1. stop readers; 2. handle everything already received (replies still
        // flow); 3. flush writers and take over the pipes.
        for (_, p) in self.procs.values_mut() {
            p.pause_reader().await;
        }
        while let Ok(ev) = rx.try_recv() {
            self.on_app(ev);
        }
        let sids: Vec<usize> = self.procs.keys().copied().collect();
        for sid in sids {
            if let Some((_, p)) = self.procs.remove(&sid)
                && let Some(h) = p.handoff().await
            {
                handoffs.insert(sid, h);
            }
        }
        self.persist();
        let sessions: Vec<Saved> = self
            .sessions
            .iter()
            .map(|s| {
                let mut sv = Self::saved(s);
                if let Some(h) = handoffs.remove(&s.sid) {
                    sv.live = Some(store::Live {
                        handoff: h,
                        busy: s.busy(),
                        turn_ms: s.turn_started.map_or(0, |t| t.elapsed().as_millis() as u64),
                        permissions: s.permissions.iter().cloned().collect(),
                        queue: s.queue.iter().cloned().collect(),
                        claude_id: s.claude_id.clone(),
                    });
                }
                sv
            })
            .collect();
        store::save_snapshot(&store::Snapshot {
            sessions,
            active: self.active,
            draft: self.editor.text.clone(),
            history: self.editor.history().iter().rev().take(200).rev().cloned().collect(),
        })
    }

    fn idx_of(&self, sid: usize) -> Option<usize> {
        self.sessions.iter().position(|s| s.sid == sid)
    }

    // ---- claude process events ----

    pub fn idle_secs(&self) -> f32 {
        self.last_activity.elapsed().as_secs_f32()
    }

    pub fn outfit(&self) -> Option<crate::ui::companion::Outfit> {
        if self.prefs.outfit { self.outfit_today } else { None }
    }

    pub fn clawd_mood(&self) -> crate::ui::companion::Mood {
        let s = self.current();
        crate::ui::companion::mood(s.busy(), !s.permissions.is_empty(), self.idle_secs())
    }

    pub fn on_app(&mut self, ev: AppEvent) {
        if matches!(ev, AppEvent::Claude(..)) {
            self.last_activity = Instant::now();
        }
        match ev {
            AppEvent::Claude(sid, generation, v) => {
                if self.procs.get(&sid).map(|(g, _)| *g) != Some(generation) {
                    return;
                }
                if v.get("type").and_then(Value::as_str) == Some("rate_limit_event") {
                    let limits = parse_limits(&v);
                    if !limits.is_empty() && limits != self.lifetime.limits {
                        self.warn_limits(&limits);
                        self.lifetime.limits = limits;
                        store::save_lifetime(&self.lifetime);
                    }
                }
                let Some(i) = self.idx_of(sid) else { return };
                let had_id = self.sessions[i].claude_id.clone();
                let was_busy = self.sessions[i].busy();
                let had_perm = !self.sessions[i].permissions.is_empty();
                let reply = self.sessions[i].handle(&v);
                self.after_event(i, was_busy, had_perm);
                if let Some(r) = reply
                    && let Some((_, p)) = self.procs.get(&sid)
                {
                    p.raw(r);
                }
                if i == self.active || self.in_split(i) {
                    self.sessions[i].unread = false;
                }
                if let Some(u) = self.sessions[i].fresh_usage.take() {
                    self.lifetime.usage.add(&u);
                    store::save_lifetime(&self.lifetime);
                    self.persist();
                }
                if !self.sessions[i].busy()
                    && let Some((text, content)) = self.sessions[i].queue.pop_front()
                {
                    self.send_now(i, &text, content);
                }
                if had_id != self.sessions[i].claude_id {
                    self.persist();
                }
            }
            AppEvent::Exit(sid, generation, tail) => {
                if self.procs.get(&sid).map(|(g, _)| *g) != Some(generation) {
                    return;
                }
                self.procs.remove(&sid);
                let Some(i) = self.idx_of(sid) else { return };
                let s = &mut self.sessions[i];
                if s.busy() {
                    let msg = if tail.trim().is_empty() {
                        "claude exited unexpectedly".to_string()
                    } else {
                        format!("claude exited: {}", tail.trim())
                    };
                    s.note(msg, NoteKind::Error);
                }
                s.reset_process_state();
                s.status = crate::session::Status::Idle;
            }
            AppEvent::Projects(list) => self.recent_projects = list,
            AppEvent::Shell(sid, idx, text, code) => {
                if let Some(i) = self.idx_of(sid) {
                    self.sessions[i].finish_shell(idx, &text, code);
                    // A plugin changed: restart claude (conversation kept) so it loads.
                    if self.sessions[i].plugin_cmd.take() == Some(idx) && code == 0 {
                        if self.sessions[i].busy() {
                            self.sessions[i].respawn = true;
                        } else if self.procs.remove(&sid).is_some() {
                            self.sessions[i].reset_process_state();
                        }
                        self.sessions[i].note("plugins updated · Claude picks them up on your next message", NoteKind::Info);
                    }
                }
            }
            AppEvent::Update(outcome, asked) => {
                use crate::update::Outcome;
                match outcome {
                    Outcome::Installed(tag) => {
                        self.toast(format!("den {tag} installed · /reload to switch (nothing stops)"));
                        self.update_ready = Some(tag);
                    }
                    Outcome::UpToDate if asked => {
                        self.toast(format!("den {} is the latest", crate::update::VERSION));
                    }
                    Outcome::Failed(e) if asked => self.toast(format!("update failed: {e}")),
                    _ => {}
                }
            }
            AppEvent::Catalog(v) => {
                self.available_styles = v
                    .get("available_output_styles")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().or_else(|| x.get("name").and_then(Value::as_str)))
                            .map(String::from)
                            .collect()
                    })
                    .unwrap_or_default();
                self.catalog = crate::session::parse_commands(&v);
                let models = parse_models(&v);
                if !models.is_empty() {
                    self.models = models;
                }
                self.refresh_popup();
            }
            AppEvent::Files(root, list) => {
                self.files.insert(root, Some(Arc::new(list)));
                self.refresh_popup();
            }
        }
    }

    /// Notifications, Clawd reactions and pending restarts after a protocol event.
    fn after_event(&mut self, i: usize, was_busy: bool, had_perm: bool) {
        use crate::ui::companion::Reaction;
        let away = !self.term_focused || (i != self.active && !self.in_split(i));
        if let Some(reset) = self.sessions[i].limit_hit.take() {
            self.schedule_resume(i, reset);
        }
        let s = &mut self.sessions[i];
        if std::mem::take(&mut s.tests_passed) && i == self.active {
            self.reaction = Some((Reaction::Wiggle, Instant::now()));
        }
        let s = &self.sessions[i];
        if !had_perm
            && let Some(p) = s.permissions.front()
        {
            if away {
                let (verb, target) = tools::summary(&p.tool_name, &p.input, &s.cwd);
                self.notify(&format!("{} needs you", s.name), &format!("wants to {verb} {target}"));
            }
        } else if was_busy && !s.busy() {
            if away {
                let last = s.entries.iter().rev().find_map(|e| match &e.item {
                    Item::Assistant(t) => t.lines().find(|l| !l.trim().is_empty()).map(str::to_string),
                    Item::Note(t, NoteKind::Error) => Some(t.clone()),
                    _ => None,
                });
                let body: String = last.unwrap_or_else(|| "finished".into()).chars().take(120).collect();
                self.notify(&format!("{} is done", s.name), &body);
            }
            let (respawn, sid) = (s.respawn, s.sid);
            if i == self.active && self.reaction_now().is_none() {
                self.react(Reaction::Hop);
            }
            if respawn {
                self.procs.remove(&sid);
                let s = &mut self.sessions[i];
                s.respawn = false;
                s.reset_process_state();
            }
        }
    }

    /// Toasts (and notifies, if you're away) when a usage window passes 80% or 95%.
    fn warn_limits(&mut self, limits: &[store::Limit]) {
        let now = crate::ui::now_secs();
        for l in limits {
            let pct = l.utilization * 100.0;
            let level = if pct >= 95.0 { 2 } else if pct >= 80.0 { 1 } else { 0 };
            let seen = self.limit_warned.get(&l.key).copied().unwrap_or(0);
            self.limit_warned.insert(l.key.clone(), level);
            if level <= seen {
                continue;
            }
            let msg = format!(
                "⚠ {} usage at {pct:.0}%{}",
                limit_name(&l.key),
                reset_in(l.resets_at, now).map(|r| format!(" · resets in {r}")).unwrap_or_default()
            );
            if !self.term_focused {
                self.notify("usage limit", &msg);
            }
            self.toast(msg);
        }
    }

    fn check_update(&self, asked: bool) {
        let tx = self.tx.clone();
        tokio::task::spawn_blocking(move || {
            let _ = tx.send(AppEvent::Update(crate::update::run(), asked));
        });
    }

    /// A turn stopped on the usage limit: plan to send "continue" once it resets.
    fn schedule_resume(&mut self, i: usize, reset: u64) {
        let now = crate::session::now_secs();
        // Claude Code didn't say when: wait for the fullest window we know of.
        let reset = if reset > now {
            reset
        } else {
            self.lifetime
                .limits
                .iter()
                .filter(|l| l.utilization >= 0.95 && l.resets_at > now)
                .map(|l| l.resets_at)
                .max()
                .unwrap_or(now + 15 * 60)
        };
        // A minute of slack after the reset (DEN_RESUME_SLACK overrides, for tests).
        let slack = std::env::var("DEN_RESUME_SLACK").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let at = reset + slack;
        let when = format!("at {} (in {})", clock(at), crate::session::dur_words(at.saturating_sub(now)));
        let s = &mut self.sessions[i];
        if self.prefs.auto_resume {
            s.resume_at = Some(at);
            s.note(format!("usage limit reached · den continues automatically {when} · esc cancels"), NoteKind::Info);
        } else {
            s.note(format!("usage limit reached · it resets {when} (turn on Auto-resume in settings to continue by itself)"), NoteKind::Info);
        }
        self.persist();
    }

    /// Sends "continue" for every session whose limit has reset.
    fn run_due_resumes(&mut self) {
        let now = crate::session::now_secs();
        for i in 0..self.sessions.len() {
            let due = self.sessions[i].resume_at.is_some_and(|t| now >= t) && !self.sessions[i].busy();
            if !due {
                continue;
            }
            self.sessions[i].resume_at = None;
            let name = self.sessions[i].name.clone();
            self.sessions[i].note("usage limit reset · continuing", NoteKind::Info);
            self.send_now(i, "continue", Value::String("The usage limit has reset. Please continue where you left off.".into()));
            self.notify(&format!("{name} resumed"), "the usage limit reset, so den picked the work back up");
        }
    }

    fn ensure_proc(&mut self, i: usize) -> bool {
        let sid = self.sessions[i].sid;
        if self.procs.contains_key(&sid) {
            return true;
        }
        self.next_gen += 1;
        let generation = self.next_gen;
        let s = &self.sessions[i];
        let res = claude::spawn(
            Spawn {
                sid,
                generation,
                cwd: &s.cwd,
                resume: s.claude_id.as_deref(),
                model: s.model.as_deref(),
                mode: Some(s.mode.as_str()),
                style: Some(s.style.as_str()),
                effort: s.effort.as_deref(),
                squeeze_hook: self.squeeze_hook.as_deref().filter(|_| self.prefs.squeeze),
            },
            self.tx.clone(),
        );
        match res {
            Ok(p) => {
                self.sessions[i].reset_process_state();
                p.initialize();
                self.procs.insert(sid, (generation, p));
                self.lifetime.sessions += 1;
                true
            }
            Err(e) => {
                self.sessions[i].note(format!("{e:#}"), NoteKind::Error);
                false
            }
        }
    }

    fn proc(&self, i: usize) -> Option<&Proc> {
        self.procs.get(&self.sessions[i].sid).map(|(_, p)| p)
    }

    fn kill(&mut self, i: usize) {
        let sid = self.sessions[i].sid;
        self.procs.remove(&sid);
        self.sessions[i].reset_process_state();
        self.sessions[i].status = crate::session::Status::Idle;
    }

    // ---- actions ----

    /// Sends queued messages right away (one, or all). Claude Code reads a
    /// message that arrives mid-turn at its next step, so nothing is interrupted.
    fn send_queued(&mut self, k: Option<usize>) {
        let i = self.active;
        let items: Vec<(String, Value)> = match k {
            Some(k) => self.sessions[i].queue.remove(k).into_iter().collect(),
            None => self.sessions[i].queue.drain(..).collect(),
        };
        if items.is_empty() {
            return;
        }
        let busy = self.sessions[i].busy();
        for (text, content) in items {
            self.send_now(i, &text, content);
        }
        if busy {
            self.toast("sent · Claude reads it at its next step");
        }
    }

    /// Pulls the newest queued message back into the editor.
    fn edit_queued(&mut self) {
        let i = self.active;
        if let Some((text, _)) = self.sessions[i].queue.pop_back() {
            self.editor.set(&text);
        }
    }

    fn submit(&mut self) {
        let raw = self.editor.take();
        let text = raw.trim();
        if text.is_empty() {
            if !self.current().queue.is_empty() {
                self.send_queued(None);
            }
            return;
        }
        self.popup = None;
        if let Some(cmd) = text.strip_prefix('/')
            && self.local_command(cmd)
        {
            return;
        }
        if let Some(cmd) = text.strip_prefix('!').map(str::trim).filter(|c| !c.is_empty()) {
            let i = self.active;
            self.run_shell(i, cmd);
            return;
        }
        let i = self.active;
        let content = match self.build_content(text) {
            Ok(c) => c,
            Err(e) => {
                self.toast(e);
                return;
            }
        };
        if self.sessions[i].busy() {
            self.sessions[i].queue.push_back((text.to_string(), content));
            return;
        }
        self.send_now(i, text, content);
    }

    /// Runs `!command` in the user's shell, in the session's folder.
    fn run_shell(&mut self, i: usize, cmd: &str) {
        let s = &mut self.sessions[i];
        let idx = s.start_shell(cmd);
        let (sid, cwd, cmd) = (s.sid, s.cwd.clone(), cmd.to_string());
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let out = tokio::process::Command::new(shell)
                .arg("-c")
                .arg(&cmd)
                .current_dir(cwd)
                .stdin(std::process::Stdio::null())
                .kill_on_drop(true)
                .output()
                .await;
            let (text, code) = match out {
                Ok(o) => {
                    let mut t = String::from_utf8_lossy(&o.stdout).into_owned();
                    t.push_str(&String::from_utf8_lossy(&o.stderr));
                    (t, o.status.code().unwrap_or(1))
                }
                Err(e) => (format!("couldn't run: {e}"), 127),
            };
            let _ = tx.send(AppEvent::Shell(sid, idx, text, code));
        });
    }

    fn send_now(&mut self, i: usize, text: &str, content: Value) {
        self.sessions[i].resume_at = None;
        // Output of earlier `!commands` rides along with this message.
        let content = if self.sessions[i].shell_context.is_empty() {
            content
        } else {
            let prefix = std::mem::take(&mut self.sessions[i].shell_context).join("\n\n") + "\n\n";
            match content {
                Value::String(t) => Value::String(prefix + &t),
                Value::Array(mut blocks) => {
                    blocks.insert(0, json!({"type": "text", "text": prefix}));
                    Value::Array(blocks)
                }
                other => other,
            }
        };
        let fresh = !self.procs.contains_key(&self.sessions[i].sid);
        self.sessions[i].user_message(text);
        if !self.ensure_proc(i) {
            self.sessions[i].status = crate::session::Status::Idle;
            return;
        }
        if fresh {
            self.sessions[i].status = crate::session::Status::Starting;
        }
        self.sessions[i].turn_started.get_or_insert_with(Instant::now);
        if let Some(p) = self.proc(i) {
            p.user(content);
        }
        self.persist();
    }

    fn local_command(&mut self, cmd: &str) -> bool {
        let (name, arg) = cmd.split_once(' ').map_or((cmd, ""), |(a, b)| (a, b.trim()));
        match name {
            "reload" => self.reload_pending = true,
            "update" => {
                self.toast("checking for updates…");
                self.check_update(true);
            }
            "style" if arg.is_empty() => {
                let list = if self.available_styles.is_empty() {
                    "default, Explanatory, Concise, Learning, minimal".to_string()
                } else {
                    self.available_styles.join(", ")
                };
                let cur = self.current().style.clone();
                self.toast(format!("style: {cur}  ·  options: {list}"));
            }
            "style" => {
                let name = self
                    .available_styles
                    .iter()
                    .find(|s| s.eq_ignore_ascii_case(arg))
                    .cloned()
                    .unwrap_or_else(|| arg.to_string());
                let i = self.active;
                self.sessions[i].style = name.clone();
                if !self.sessions[i].busy() {
                    let sid = self.sessions[i].sid;
                    self.procs.remove(&sid);
                    self.sessions[i].reset_process_state();
                }
                self.sessions[i].note(format!("output style → {name}"), NoteKind::Info);
                self.persist();
            }
            "workspace" | "ws" => {
                let (verb, rest) = arg.split_once(' ').map_or((arg, ""), |(a, b)| (a, b.trim()));
                match verb {
                    "" | "list" => {
                        if self.workspaces.is_empty() {
                            self.toast("no workspaces · /workspace add <folder>");
                        } else {
                            let list: Vec<String> = self.workspaces.iter().map(|w| tools::tilde(&w.to_string_lossy())).collect();
                            self.toast(format!("workspaces: {}", list.join(", ")));
                        }
                    }
                    "add" => self.add_workspace(if rest.is_empty() { "." } else { rest }),
                    "remove" | "rm" => self.remove_workspace(rest),
                    _ => self.add_workspace(arg),
                }
            }
            "cd" if arg.is_empty() => self.open_new_modal(),
            "cd" => self.cd(arg),
            "new" => {
                if arg.is_empty() {
                    self.open_new_modal();
                } else {
                    self.new_session_at(arg);
                }
            }
            "close" => self.close_active(),
            "rename" if !arg.is_empty() => {
                let s = self.current_mut();
                s.name = arg.to_string();
                s.auto_named = false;
                self.persist();
            }
            "rename" => self.toast("usage: /rename <name>"),
            "clear" => {
                let i = self.active;
                self.kill(i);
                let cwd = self.sessions[i].cwd.clone();
                let model = self.sessions[i].model.clone();
                let mode = self.sessions[i].mode.clone();
                let mut s = self.make_session(cwd);
                s.model = model;
                s.mode = mode;
                self.sessions[i] = s;
                self.persist();
            }
            "model" if !arg.is_empty() => {
                let i = self.active;
                self.sessions[i].model = Some(arg.to_string());
                if let Some(p) = self.proc(i) {
                    p.control(json!({"subtype": "set_model", "model": arg}));
                }
                self.sessions[i].note(format!("model → {arg}"), NoteKind::Info);
                self.persist();
            }
            "model" => self.open_model_picker(),
            "effort" if arg.is_empty() => self.open_model_picker(),
            "effort" => {
                let e = arg.to_lowercase();
                let e = if matches!(e.as_str(), "auto" | "default" | "off") { None } else { Some(e) };
                self.set_effort(e);
            }
            "files" | "diff" => self.open_files(),
            "plugin" | "plugins" => {
                // Claude Code's /plugin menu only works in its own screen; its
                // command-line twin does the same job.
                let args = if arg.is_empty() { "list" } else { arg };
                let i = self.active;
                let changes = ["install", "i ", "uninstall", "remove", "enable", "disable", "update", "marketplace add"]
                    .iter()
                    .any(|w| args.starts_with(w));
                self.run_shell(i, &format!("claude plugin {args}"));
                if changes {
                    self.sessions[i].plugin_cmd = self.sessions[i].entries.len().checked_sub(1);
                }
            }
            "split" if matches!(arg, "off" | "close") => {
                let a = self.active;
                self.close_pane(a);
            }
            "split" => self.toggle_split(),
            "unsplit" => {
                let a = self.active;
                self.close_pane(a);
            }
            "settings" | "config" | "prefs" => self.open_modal(Modal::Settings { sel: 0 }),
            "theme" if arg.is_empty() => self.open_theme_picker(),
            "theme" => {
                if crate::theme::set(arg) {
                    self.save_theme();
                } else {
                    let names: Vec<&str> = crate::theme::PALETTES.iter().map(|t| t.name).collect();
                    self.toast(format!("themes: {}", names.join(", ")));
                }
            }
            "search" | "find" => {
                let mut q = Editor::default();
                q.set(arg);
                self.open_modal(Modal::Search { query: q, sel: 0 });
            }
            "help" => self.open_modal(Modal::Help),
            "stats" | "cost" | "usage" => self.open_modal(Modal::Stats),
            "agents" => self.open_modal(Modal::Agents),
            "mode" if arg.is_empty() => self.cycle_mode(),
            "mode" => {
                let m = crate::session::MODES
                    .iter()
                    .find(|m| m.eq_ignore_ascii_case(arg) || crate::session::mode_label(m).starts_with(&arg.to_lowercase()))
                    .copied();
                match m {
                    Some(m) => self.set_mode(m),
                    None => self.toast("modes: default, acceptEdits, plan, auto"),
                }
            }
            "copy" => {
                let last = self.current().entries.iter().rev().find_map(|e| match &e.item {
                    Item::Assistant(t) => Some(t.clone()),
                    _ => None,
                });
                match last {
                    Some(t) => self.copy_text(&t),
                    None => self.toast("nothing to copy yet"),
                }
            }
            "tutorial" | "tour" => self.open_modal(Modal::Tutorial { step: 0 }),
            "quit" | "exit" => self.quit = true,
            _ => return false,
        }
        true
    }

    /// Expands pasted-text tags and attaches images as content blocks.
    fn build_content(&self, text: &str) -> Result<Value, String> {
        use crate::input::Attachment;
        let atts = self.editor.attachments_in(text);
        if atts.is_empty() {
            return Ok(Value::String(text.to_string()));
        }
        let mut expanded = text.to_string();
        let mut images = Vec::new();
        for (tag, att) in atts {
            match att {
                Attachment::Text(t) => expanded = expanded.replacen(&tag, &t, 1),
                Attachment::Image { path, media } => {
                    let bytes = std::fs::read(&path).map_err(|e| format!("{tag}: {e}"))?;
                    if bytes.len() > 5 * 1024 * 1024 {
                        return Err(format!("{tag} is over 5 MB"));
                    }
                    images.push(json!({
                        "type": "image",
                        "source": {"type": "base64", "media_type": media, "data": base64(&bytes)}
                    }));
                }
            }
        }
        if images.is_empty() {
            return Ok(Value::String(expanded));
        }
        let mut blocks = vec![json!({"type": "text", "text": expanded})];
        blocks.extend(images);
        Ok(Value::Array(blocks))
    }

    pub fn model_choices(&self) -> Vec<ModelOpt> {
        if self.models.is_empty() { fallback_models() } else { self.models.clone() }
    }

    /// Effort options for a model: "auto" (Claude Code decides) plus its levels.
    pub fn effort_choices(m: &ModelOpt) -> Vec<String> {
        std::iter::once("auto".to_string()).chain(m.efforts.iter().cloned()).collect()
    }

    fn open_model_picker(&mut self) {
        let models = self.model_choices();
        let s = self.current();
        let cur = s.model.as_deref().unwrap_or("default");
        let sel = models
            .iter()
            .position(|m| m.value == cur || (!m.resolved.is_empty() && m.resolved == cur && m.value != "default"))
            .unwrap_or(0);
        let effort = models
            .get(sel)
            .and_then(|m| Self::effort_choices(m).iter().position(|e| Some(e.as_str()) == s.effort.as_deref()))
            .unwrap_or(0);
        self.open_modal(Modal::Model { sel, effort });
    }

    fn set_model(&mut self, value: &str) {
        let i = self.active;
        if self.sessions[i].model.as_deref() == Some(value) {
            return;
        }
        self.sessions[i].model = Some(value.to_string());
        if let Some(p) = self.proc(i) {
            p.control(json!({"subtype": "set_model", "model": value}));
        }
        self.sessions[i].note(format!("model → {value}"), NoteKind::Info);
        self.persist();
    }

    /// Effort is a launch flag, so the process restarts (now, or after this turn).
    fn set_effort(&mut self, effort: Option<String>) {
        let i = self.active;
        if self.sessions[i].effort == effort {
            return;
        }
        self.sessions[i].effort = effort.clone();
        let sid = self.sessions[i].sid;
        if self.sessions[i].busy() {
            self.sessions[i].respawn = true;
        } else if self.procs.remove(&sid).is_some() {
            self.sessions[i].reset_process_state();
        }
        let label = effort.as_deref().unwrap_or("auto");
        let when = if self.sessions[i].busy() { " (after this reply)" } else { "" };
        self.sessions[i].note(format!("effort → {label}{when}"), NoteKind::Info);
        self.persist();
    }

    fn open_files(&mut self) {
        if self.current().changes.is_empty() {
            self.toast("no files changed in this session yet");
            return;
        }
        self.open_modal(Modal::Files { sel: 0, scroll: 0 });
    }

    /// Sessions on screen, left to right. Keeps the split in step with the
    /// active session: picking another session swaps it into the focused pane.
    pub fn panes(&mut self) -> Vec<usize> {
        let Some((l, r)) = self.split else { return vec![self.active] };
        let (li, ri) = (self.idx_of(l), self.idx_of(r));
        let act = self.sessions[self.active].sid;
        let (Some(_), Some(_)) = (li, ri) else {
            self.split = None;
            return vec![self.active];
        };
        let (l, r) = if act == l {
            self.split_side = 0;
            (l, r)
        } else if act == r {
            self.split_side = 1;
            (l, r)
        } else if self.split_side == 0 {
            (act, r)
        } else {
            (l, act)
        };
        self.split = Some((l, r));
        [l, r].iter().filter_map(|&sid| self.idx_of(sid)).collect()
    }

    pub fn in_split(&self, i: usize) -> bool {
        let sid = self.sessions[i].sid;
        self.split.is_some_and(|(l, r)| l == sid || r == sid)
    }

    /// Opens a second pane (the most recent other session, or a new one
    /// in the same folder), or moves focus to the other pane.
    fn toggle_split(&mut self) {
        if let Some((l, r)) = self.split {
            let act = self.sessions[self.active].sid;
            let other = if act == l { r } else { l };
            if let Some(i) = self.idx_of(other) {
                self.switch_to(i);
                self.switched_at = None;
            }
            return;
        }
        let act = self.active;
        let other = (0..self.sessions.len())
            .filter(|&i| i != act)
            .max_by_key(|&i| self.sessions[i].last_active)
            .unwrap_or_else(|| {
                let s = self.make_session(self.sessions[act].cwd.clone());
                self.sessions.insert(act + 1, s);
                act + 1
            });
        self.split = Some((self.sessions[self.active].sid, self.sessions[other].sid));
        self.split_side = 0;
        self.toast("split view · ^\\ switches pane · × closes one");
    }

    fn close_pane(&mut self, idx: usize) {
        let Some((l, r)) = self.split.take() else { return };
        let sid = self.sessions[idx].sid;
        let keep = if sid == l { r } else { l };
        if let Some(i) = self.idx_of(keep) {
            self.switch_to(i);
            self.switched_at = None;
        }
    }

    fn set_mode(&mut self, mode: &str) {
        let i = self.active;
        self.sessions[i].mode = mode.to_string();
        if let Some(p) = self.proc(i) {
            p.control(json!({"subtype": "set_permission_mode", "mode": mode}));
        }
        self.toast(format!("mode: {}", crate::session::mode_label(mode)));
        self.persist();
    }

    fn cycle_mode(&mut self) {
        let modes = crate::session::MODES;
        let cur = modes.iter().position(|m| *m == self.current().mode).unwrap_or(0);
        self.set_mode(modes[(cur + 1) % modes.len()]);
    }

    pub fn copy_text(&mut self, text: &str) {
        let text = text.trim_end();
        if text.is_empty() {
            return;
        }
        let copied = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().map(|mut i| i.write_all(text.as_bytes())).transpose()?;
                c.wait()
            })
            .is_ok_and(|s| s.success());
        if !copied {
            use std::io::Write;
            let b64 = base64(text.as_bytes());
            let _ = write!(std::io::stdout(), "\x1b]52;c;{b64}\x07");
            let _ = std::io::stdout().flush();
        }
        let lines = text.lines().count();
        self.toast(if lines > 1 { format!("copied {lines} lines") } else { "copied".to_string() });
    }

    pub fn open_board(&mut self) {
        let folder = self
            .workspaces
            .iter()
            .position(|w| self.current().cwd.parent() == Some(w.as_path()))
            .map_or(0, |i| i + 1);
        self.open_modal(Modal::Board(Board {
            folder,
            sel: 0,
            focus: BoardFocus::Projects,
            input: None,
            filter: String::new(),
        }));
    }

    /// Folder list for the board: "Recent" first, then each workspace.
    /// Folders of projects: ones the user added, then parents that hold two or
    /// more known projects (so renamed or never-opened siblings show up too).
    pub fn all_workspaces(&self) -> Vec<PathBuf> {
        let mut out = self.workspaces.clone();
        let hidden = store::load_config().hidden_workspaces;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut counts: Vec<(PathBuf, usize)> = Vec::new();
        for p in self.sessions.iter().map(|s| &s.cwd).chain(self.recent_projects.iter()) {
            let Some(parent) = p.parent() else { continue };
            match counts.iter_mut().find(|(q, _)| q == parent) {
                Some((_, n)) => *n += 1,
                None => counts.push((parent.to_path_buf(), 1)),
            }
        }
        for (parent, n) in counts {
            let skip = n < 2
                || parent.parent().is_none()
                || home.as_deref() == Some(parent.as_path())
                || parent.starts_with("/private/tmp")
                || parent.starts_with("/tmp")
                || parent.starts_with("/var/folders")
                || parent.starts_with("/private/var")
                || hidden.contains(&parent)
                || out.contains(&parent);
            if !skip {
                out.push(parent);
            }
        }
        out
    }

    pub fn board_folders(&self) -> Vec<(String, Option<PathBuf>, usize)> {
        let mut out = vec![("Recent".to_string(), None, self.board_recent().len())];
        for w in &self.all_workspaces() {
            let name = w.file_name().map_or_else(|| tools::tilde(&w.to_string_lossy()), |n| n.to_string_lossy().into_owned());
            out.push((name, Some(w.clone()), crate::projects::children(w).len()));
        }
        out
    }

    fn board_recent(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        for p in self.sessions.iter().map(|s| &s.cwd).chain(self.recent_projects.iter()) {
            if !out.contains(p) {
                out.push(p.clone());
            }
        }
        out
    }

    pub fn board_projects(&self, folder: usize, filter: &str) -> Vec<PathBuf> {
        let all = match folder.checked_sub(1).and_then(|i| self.all_workspaces().get(i).cloned()) {
            Some(w) => crate::projects::children(&w),
            None => self.board_recent(),
        };
        if filter.is_empty() {
            return all;
        }
        let names: Vec<String> = all.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let hits = files::top(filter, names.iter().map(String::as_str), usize::MAX);
        hits.into_iter().map(PathBuf::from).collect()
    }

    fn board_click(&mut self, h: BoardHit) {
        let Some(Modal::Board(b)) = &mut self.modal else { return };
        match h {
            BoardHit::Folder(i) => {
                b.folder = i;
                b.sel = 0;
                b.focus = BoardFocus::Projects;
                b.filter.clear();
            }
            BoardHit::Project(i) => {
                let projects = {
                    let (f, flt) = (b.folder, b.filter.clone());
                    self.board_projects(f, &flt)
                };
                if let Some(p) = projects.get(i) {
                    let p = p.to_string_lossy().into_owned();
                    self.modal = None;
                    self.new_session_at(&p);
                }
            }
            BoardHit::AddFolder => {
                let mut e = Editor::default();
                e.set("~/");
                b.input = Some((BoardInput::AddFolder, e));
            }
            BoardHit::NewProject => {
                if b.folder > 0 {
                    b.input = Some((BoardInput::NewProject, Editor::default()));
                } else {
                    self.toast("pick a folder on the left first, or add one");
                }
            }
            BoardHit::RemoveFolder => self.board_remove_folder(),
        }
    }

    fn board_remove_folder(&mut self) {
        let all = self.all_workspaces();
        let Some(Modal::Board(b)) = &mut self.modal else { return };
        let Some(w) = b.folder.checked_sub(1).and_then(|i| all.get(i)).cloned() else { return };
        b.folder = b.folder.saturating_sub(1);
        b.sel = 0;
        let mut c = store::load_config();
        if let Some(i) = self.workspaces.iter().position(|x| *x == w) {
            self.workspaces.remove(i);
            c.workspaces = self.workspaces.clone();
        } else if !c.hidden_workspaces.contains(&w) {
            c.hidden_workspaces.push(w.clone());
        }
        store::save_config(&c);
        self.toast(format!("removed {} from the board (nothing deleted)", tools::tilde(&w.to_string_lossy())));
    }

    fn board_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let all_ws = self.all_workspaces();
        let Some(Modal::Board(b)) = &mut self.modal else { return };

        if let Some((kind, e)) = &mut b.input {
            let kind = *kind;
            match k.code {
                KeyCode::Esc => {
                    if kind == BoardInput::Filter {
                        b.filter.clear();
                    }
                    b.input = None;
                }
                KeyCode::Tab if kind == BoardInput::AddFolder => complete_dir(e),
                KeyCode::Enter => {
                    let text = e.text.trim().to_string();
                    b.input = None;
                    match kind {
                        BoardInput::Filter => {}
                        BoardInput::AddFolder => {
                            self.add_workspace(&text);
                            let p = expand(&text);
                            let n = self.all_workspaces().iter().position(|w| *w == p || std::fs::canonicalize(&p).ok().as_ref() == Some(w)).map_or(self.workspaces.len(), |i| i + 1);
                            if let Some(Modal::Board(b)) = &mut self.modal {
                                b.folder = n;
                                b.sel = 0;
                                b.focus = BoardFocus::Projects;
                            }
                        }
                        BoardInput::NewProject => {
                            let Some(ws) = b.folder.checked_sub(1).and_then(|i| all_ws.get(i)).cloned() else { return };
                            if text.is_empty() || text.contains('/') {
                                self.toast("type a folder name");
                                return;
                            }
                            let p = ws.join(&text);
                            match std::fs::create_dir_all(&p) {
                                Ok(()) => {
                                    self.modal = None;
                                    self.new_session_at(&p.to_string_lossy());
                                }
                                Err(err) => self.toast(format!("couldn't create folder: {err}")),
                            }
                        }
                    }
                }
                _ => {
                    edit_line(e, k);
                    if kind == BoardInput::Filter {
                        b.filter = e.text.clone();
                        b.sel = 0;
                    }
                }
            }
            return;
        }

        let folders = self.board_folders().len();
        let Some(Modal::Board(b)) = &mut self.modal else { return };
        let (folder, filter) = (b.folder, b.filter.clone());
        let n_proj = self.board_projects(folder, &filter).len();
        let Some(Modal::Board(b)) = &mut self.modal else { return };
        match k.code {
            KeyCode::Esc => {
                if !b.filter.is_empty() {
                    b.filter.clear();
                } else {
                    self.modal = None;
                }
            }
            KeyCode::Char('c') if ctrl => self.modal = None,
            KeyCode::Up | KeyCode::Char('k') => match b.focus {
                BoardFocus::Folders => {
                    b.folder = b.folder.saturating_sub(1);
                    b.sel = 0;
                }
                BoardFocus::Projects => b.sel = b.sel.saturating_sub(1),
            },
            KeyCode::Down | KeyCode::Char('j') => match b.focus {
                BoardFocus::Folders => {
                    b.folder = (b.folder + 1).min(folders.saturating_sub(1));
                    b.sel = 0;
                }
                BoardFocus::Projects => b.sel = (b.sel + 1).min(n_proj.saturating_sub(1)),
            },
            KeyCode::Left | KeyCode::Char('h') => b.focus = BoardFocus::Folders,
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => b.focus = BoardFocus::Projects,
            KeyCode::Enter => match b.focus {
                BoardFocus::Folders => b.focus = BoardFocus::Projects,
                BoardFocus::Projects => {
                    let sel = b.sel;
                    self.board_click(BoardHit::Project(sel));
                }
            },
            KeyCode::Char('n') => self.board_click(BoardHit::NewProject),
            KeyCode::Char('a') => self.board_click(BoardHit::AddFolder),
            KeyCode::Char('d') | KeyCode::Delete if b.focus == BoardFocus::Folders => self.board_remove_folder(),
            KeyCode::Char('/') => {
                let mut e = Editor::default();
                e.set(&b.filter);
                b.input = Some((BoardInput::Filter, e));
            }
            _ => {}
        }
    }

    fn open_new_modal(&mut self) {
        self.open_modal(Modal::Project { query: Editor::default(), sel: 0 });
    }

    fn new_session_at(&mut self, path: &str) {
        let p = expand(path);
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !p.is_dir() {
            self.toast(format!("not a directory: {path}"));
            return;
        }
        let s = self.make_session(p.clone());
        let at = self.sessions.iter().rposition(|x| x.cwd == p).map_or(self.sessions.len(), |i| i + 1);
        self.sessions.insert(at, s);
        self.switch_to(at);
        self.react(crate::ui::companion::Reaction::Wave);
    }

    /// Moves an untouched session into `path`, or opens a new one there.
    fn cd(&mut self, path: &str) {
        let base = self.current().cwd.clone();
        let p = expand(path);
        let p = if p.is_absolute() { p } else { base.join(p) };
        let p = std::fs::canonicalize(&p).unwrap_or(p);
        if !p.is_dir() {
            self.toast(format!("not a folder: {path}"));
            return;
        }
        let i = self.active;
        let pristine = self.sessions[i].entries.is_empty()
            && self.sessions[i].claude_id.is_none()
            && !self.procs.contains_key(&self.sessions[i].sid);
        if pristine {
            self.sessions[i].cwd = p.clone();
            self.regroup();
            self.toast(format!("→ {}", tools::tilde(&p.to_string_lossy())));
        } else {
            self.new_session_at(&p.to_string_lossy());
        }
    }

    fn close_active(&mut self) {
        self.close_session(self.active);
    }

    pub fn close_session(&mut self, i: usize) {
        if i >= self.sessions.len() {
            return;
        }
        let name = self.sessions[i].name.clone();
        self.kill(i);
        self.sessions.remove(i);
        if self.sessions.is_empty() {
            let s = self.make_session(self.launch_dir.clone());
            self.sessions.push(s);
        }
        if i < self.active || self.active >= self.sessions.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.sessions[self.active].unread = false;
        self.switched_at = Some(Instant::now());
        self.toast(format!("closed “{name}”"));
        self.persist();
    }

    pub fn switch_to(&mut self, i: usize) {
        if i < self.sessions.len() {
            if i != self.active {
                self.switched_at = Some(Instant::now());
                // Each session keeps its own unsent draft.
                if let Some(old) = self.sessions.get_mut(self.active) {
                    old.draft = self.editor.text.clone();
                }
                let draft = std::mem::take(&mut self.sessions[i].draft);
                self.editor.set(&draft);
            }
            self.active = i;
            self.sessions[i].unread = false;
            self.popup = None;
            self.focus = Focus::Input;
        }
    }

    fn cycle(&mut self, d: isize) {
        let n = self.sessions.len() as isize;
        self.switch_to(((self.active as isize + d).rem_euclid(n)) as usize);
    }

    fn answer_permission(&mut self, choice: char) {
        let i = self.active;
        let Some(perm) = self.sessions[i].permissions.pop_front() else { return };
        if let Some(id) = &perm.tool_use_id {
            self.sessions[i].set_waiting(id, false);
        }
        let response = match choice {
            'y' => json!({"behavior": "allow", "updatedInput": perm.input}),
            'a' => {
                let rules: Vec<Value> = perm
                    .suggestions
                    .iter()
                    .filter(|s| matches!(tools::s(s, "type"), "addRules" | "setMode"))
                    .cloned()
                    .collect();
                let mut r = json!({"behavior": "allow", "updatedInput": perm.input});
                if !rules.is_empty() {
                    r["updatedPermissions"] = Value::Array(rules);
                }
                r
            }
            _ => {
                if let Some(id) = &perm.tool_use_id {
                    self.sessions[i].mark_denied(id);
                }
                json!({"behavior": "deny", "message": "The user declined this tool use. Stop and wait for their next instruction."})
            }
        };
        if let Some(p) = self.proc(i) {
            p.permission(&perm.request_id, response);
        }
    }

    fn interrupt(&mut self) {
        let i = self.active;
        if !self.sessions[i].busy() {
            return;
        }
        while !self.sessions[i].permissions.is_empty() {
            self.answer_permission('n');
        }
        self.sessions[i].queue.clear();
        if let Some(p) = self.proc(i) {
            p.interrupt();
            self.toast("interrupting…");
        }
    }

    // ---- popups ----

    fn refresh_popup(&mut self) {
        if let Some(arg) = self.editor.text.strip_prefix("/cd ")
            && self.editor.cursor == self.editor.text.len()
        {
            let items = dir_completions(arg, &self.current().cwd);
            let prev = self.popup.as_ref().filter(|p| p.kind == PopupKind::Dir).map_or(0, |p| p.sel);
            self.set_popup(PopupKind::Dir, 4, items, prev);
            return;
        }
        let Some((start, tok)) = self.editor.token_at_cursor() else {
            self.popup = None;
            return;
        };
        let tok = tok.to_string();
        let prev_sel = self.popup.as_ref().map_or(0, |p| p.sel);
        if let Some(q) = tok.strip_prefix('/') {
            let s = self.current();
            let mut cands: Vec<(String, String)> = LOCAL_COMMANDS
                .iter()
                .map(|(c, d)| (c.to_string(), d.to_string()))
                .collect();
            let remote: Vec<(String, String)> = if !s.commands.is_empty() {
                s.commands.clone()
            } else if !self.catalog.is_empty() {
                self.catalog.clone()
            } else {
                s.slash_commands.iter().map(|c| (c.clone(), String::new())).collect()
            };
            for (c, d) in remote {
                if HIDDEN_COMMANDS.contains(&c.as_str()) {
                    continue;
                }
                if !cands.iter().any(|(n, _)| *n == c) {
                    cands.push((c, d));
                }
            }
            let names: Vec<&str> = cands.iter().map(|(n, _)| n.as_str()).collect();
            let hits = files::top(q, names, usize::MAX);
            let items: Vec<(String, String)> = hits
                .into_iter()
                .filter_map(|h| cands.iter().find(|(n, _)| n == h).cloned())
                .collect();
            self.set_popup(PopupKind::Slash, start, items, prev_sel);
        } else if let Some(q) = tok.strip_prefix('@') {
            let root = self.current().cwd.clone();
            match self.files.get(&root) {
                Some(Some(list)) => {
                    let hits = files::top(q, list.iter().map(String::as_str), 8);
                    let items = hits.into_iter().map(|h| (h.to_string(), String::new())).collect();
                    self.set_popup(PopupKind::File, start, items, prev_sel);
                }
                Some(None) => {
                    self.set_popup(PopupKind::File, start, vec![("indexing…".into(), String::new())], 0)
                }
                None => {
                    self.files.insert(root.clone(), None);
                    let tx = self.tx.clone();
                    tokio::task::spawn_blocking(move || {
                        let list = files::index(&root);
                        let _ = tx.send(AppEvent::Files(root, list));
                    });
                    self.set_popup(PopupKind::File, start, vec![("indexing…".into(), String::new())], 0)
                }
            }
        }
    }

    fn set_popup(&mut self, kind: PopupKind, start: usize, items: Vec<(String, String)>, sel: usize) {
        if items.is_empty() {
            self.popup = None;
            return;
        }
        let sel = sel.min(items.len() - 1);
        self.popup = Some(Popup { kind, start, items, sel });
    }

    fn accept_popup(&mut self) -> bool {
        let Some(p) = self.popup.take() else { return false };
        let Some((item, _)) = p.items.get(p.sel) else { return false };
        if item == "indexing…" {
            return true;
        }
        let with = match p.kind {
            PopupKind::Slash => format!("/{item} "),
            PopupKind::File => format!("@{item} "),
            PopupKind::Dir => {
                self.editor.set(&format!("/cd {item}/"));
                self.refresh_popup();
                return true;
            }
        };
        self.editor.replace_token(p.start, &with);
        true
    }

    // ---- input events ----

    pub fn on_term(&mut self, ev: Event) {
        if matches!(ev, Event::Key(_) | Event::Paste(_)) {
            self.last_activity = Instant::now();
        }
        match ev {
            Event::Key(k) if k.kind != KeyEventKind::Release => self.on_key(k),
            Event::FocusGained => self.term_focused = true,
            Event::FocusLost => self.term_focused = false,
            Event::Paste(s) => {
                let one_line = s.trim().replace(['\r', '\n'], " ");
                if let Some(
                    Modal::Project { query, .. } | Modal::Switcher { query, .. } | Modal::Search { query, .. },
                ) = &mut self.modal
                {
                    query.insert(&one_line);
                } else if let Some(Modal::Board(b)) = &mut self.modal {
                    match &mut b.input {
                        Some((_, e)) => e.insert(&one_line),
                        None => {
                            // Pasting a folder straight onto the board adds it.
                            let mut e = Editor::default();
                            e.set(&one_line);
                            b.input = Some((BoardInput::AddFolder, e));
                        }
                    }
                } else if self.modal.is_some() {
                } else {
                    self.focus = Focus::Input;
                    self.editor.paste(&s);
                    self.refresh_popup();
                }
            }
            Event::Mouse(m) => match m.kind {
                MouseEventKind::Moved => self.hover = Some((m.column, m.row)),
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                    if matches!(self.modal, Some(Modal::Files { .. })) =>
                {
                    if let Some(Modal::Files { scroll, .. }) = &mut self.modal {
                        *scroll = if m.kind == MouseEventKind::ScrollUp {
                            scroll.saturating_sub(3)
                        } else {
                            scroll.saturating_add(3)
                        };
                    }
                }
                MouseEventKind::ScrollUp => self.scroll(3),
                MouseEventKind::ScrollDown => self.scroll(-3),
                MouseEventKind::Down(_)
                    if self.modal.is_none() && self.model_hit.contains((m.column, m.row).into()) =>
                {
                    self.open_model_picker();
                }
                MouseEventKind::Down(_)
                    if self.modal.is_none() && self.pane_hits.iter().any(|(r, _)| r.contains((m.column, m.row).into())) =>
                {
                    // Buttons come before the whole-pane focus targets.
                    let mut hits: Vec<PaneHit> = self
                        .pane_hits
                        .iter()
                        .filter(|(r, _)| r.contains((m.column, m.row).into()))
                        .map(|(_, h)| *h)
                        .collect();
                    hits.sort_by_key(|h| matches!(h, PaneHit::Focus(_)));
                    match hits[0] {
                        PaneHit::Split => self.toggle_split(),
                        PaneHit::Close(i) => self.close_pane(i),
                        PaneHit::Focus(i) => {
                            self.switch_to(i);
                            self.switched_at = None;
                        }
                        PaneHit::SendQueued(k) => self.send_queued(Some(k)),
                        PaneHit::DropQueued(k) => {
                            let i = self.active;
                            self.sessions[i].queue.remove(k);
                        }
                    }
                }
                MouseEventKind::Down(crossterm::event::MouseButton::Left)
                    if self.text_rect.contains((m.column, m.row).into()) =>
                {
                    self.selection = Some(((m.column, m.row), (m.column, m.row)));
                }
                MouseEventKind::Drag(crossterm::event::MouseButton::Left) => {
                    if let Some((_, cur)) = &mut self.selection {
                        let r = self.text_rect;
                        *cur = (
                            m.column.clamp(r.x, r.right().saturating_sub(1)),
                            m.row.clamp(r.y, r.bottom().saturating_sub(1)),
                        );
                    }
                }
                MouseEventKind::Up(crossterm::event::MouseButton::Left) => {
                    if let Some((a, b)) = self.selection {
                        if a == b {
                            self.selection = None;
                        } else {
                            self.copy_pending = true;
                        }
                    }
                }
                MouseEventKind::Down(_) if matches!(self.modal, Some(Modal::Settings { .. })) => {
                    if let Some(row) = self.settings_hits.iter().position(|r| r.contains((m.column, m.row).into())) {
                        self.modal = Some(Modal::Settings { sel: row });
                        self.toggle_setting(row);
                    }
                }
                MouseEventKind::Down(_) if matches!(self.modal, Some(Modal::Board(_))) => {
                    let hit = self
                        .board_hits
                        .iter()
                        .find(|(r, _)| r.contains((m.column, m.row).into()))
                        .map(|(_, h)| h.clone());
                    if let Some(h) = hit {
                        self.board_click(h);
                    }
                }
                MouseEventKind::Down(_)
                    if self.modal.is_none() && self.agent_hits.iter().any(|r| r.contains((m.column, m.row).into())) =>
                {
                    self.open_modal(Modal::Agents);
                }
                MouseEventKind::Down(_) => {
                    let hit = self
                        .sidebar_rows
                        .iter()
                        .find(|(r, _)| r.contains((m.column, m.row).into()))
                        .map(|(_, h)| h.clone());
                    match hit {
                        Some(SideHit::Session(i)) => self.switch_to(i),
                        Some(SideHit::Close(i)) => self.close_session(i),
                        Some(SideHit::NewIn(p)) => self.new_session_at(&p.to_string_lossy()),
                        Some(SideHit::NewButton) => self.open_new_modal(),
                        Some(SideHit::Board) => self.open_board(),
                        Some(SideHit::Files) => self.open_files(),
                        Some(SideHit::Reload) => self.reload_pending = true,
                        Some(SideHit::Settings) => self.open_modal(Modal::Settings { sel: 0 }),
                        None => {}
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn scroll(&mut self, by: isize) {
        let s = self.current_mut();
        s.scroll = (s.scroll as isize + by).max(0) as usize;
    }

    fn on_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);

        if self.modal.is_some() {
            self.on_modal_key(k);
            return;
        }

        // global
        match k.code {
            KeyCode::Char('q') if ctrl => {
                self.quit = true;
                return;
            }
            KeyCode::Char('n') if ctrl => return self.open_new_modal(),
            KeyCode::Char('p') if ctrl => {
                self.open_modal(Modal::Switcher { query: Editor::default(), sel: self.active });
                return;
            }
            KeyCode::Char('b') if ctrl => {
                self.sidebar = !self.sidebar;
                return;
            }
            KeyCode::BackTab => return self.cycle_mode(),
            KeyCode::Char('g') if ctrl => return self.open_board(),
            KeyCode::Char('x') if ctrl => return self.open_modal(Modal::Agents),
            KeyCode::Char('\\' | '4') if ctrl => return self.toggle_split(),
            KeyCode::Char('f') if ctrl => {
                return self.open_modal(Modal::Search { query: Editor::default(), sel: 0 });
            }
            KeyCode::Char('m') if alt => return self.open_model_picker(),
            KeyCode::Char('s') if ctrl => return self.open_modal(Modal::Settings { sel: 0 }),
            KeyCode::Char('µ') => return self.open_model_picker(),
            KeyCode::Char('t') if ctrl => {
                if self.focus == Focus::Transcript {
                    self.focus = Focus::Input;
                } else {
                    self.focus_transcript();
                }
                return;
            }
            KeyCode::Char('o') if ctrl => {
                self.expand_all = !self.expand_all;
                return;
            }
            KeyCode::Char('c') if ctrl => return self.ctrl_c(),
            KeyCode::Char(c) if alt && c.is_ascii_digit() && c != '0' => {
                return self.switch_to(c as usize - '1' as usize);
            }
            KeyCode::Up if alt => return self.cycle(-1),
            KeyCode::Down if alt => return self.cycle(1),
            KeyCode::Char('[') if alt => return self.cycle(-1),
            KeyCode::Char(']') if alt => return self.cycle(1),
            KeyCode::PageUp => return self.scroll(self.page.saturating_sub(2) as isize),
            KeyCode::PageDown => return self.scroll(-(self.page.saturating_sub(2) as isize)),
            KeyCode::Up if shift => return self.scroll(3),
            KeyCode::Down if shift => return self.scroll(-3),
            _ => {}
        }

        if !self.current().permissions.is_empty() && !ctrl && !alt {
            match k.code {
                KeyCode::Char(c @ ('y' | 'a' | 'n')) if self.editor.is_empty() => {
                    return self.answer_permission(c);
                }
                KeyCode::Esc => return self.answer_permission('n'),
                _ => {}
            }
        }

        if self.focus == Focus::Transcript {
            return self.on_transcript_key(k);
        }
        self.on_input_key(k);
    }

    fn ctrl_c(&mut self) {
        if self.popup.take().is_some() {
            return;
        }
        if !self.editor.is_empty() {
            self.editor.clear();
            return;
        }
        if self.current().busy() {
            self.interrupt();
            return;
        }
        if self.last_ctrl_c.is_some_and(|t| t.elapsed() < Duration::from_millis(1200)) {
            self.quit = true;
        } else {
            self.last_ctrl_c = Some(Instant::now());
            self.toast("press ctrl+c again to quit");
        }
    }

    fn on_input_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        let alt = k.modifiers.contains(KeyModifiers::ALT);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);

        if let Some(p) = &mut self.popup {
            match k.code {
                KeyCode::Up => {
                    p.sel = p.sel.checked_sub(1).unwrap_or(p.items.len() - 1);
                    return;
                }
                KeyCode::Down => {
                    p.sel = (p.sel + 1) % p.items.len();
                    return;
                }
                KeyCode::Tab => {
                    self.accept_popup();
                    return;
                }
                KeyCode::Enter if !shift && !alt && p.kind == PopupKind::Dir => {
                    if let Some((item, _)) = p.items.get(p.sel).cloned() {
                        self.editor.set(&format!("/cd {item}"));
                    }
                    self.popup = None;
                    self.submit();
                    return;
                }
                KeyCode::Enter if !shift && !alt => {
                    let slash = p.kind == PopupKind::Slash;
                    let exact = p.items.get(p.sel).is_some_and(|(n, _)| {
                        self.editor.token_at_cursor().is_some_and(|(_, t)| t.get(1..) == Some(n.as_str()))
                    });
                    if slash && exact {
                        self.popup = None;
                        self.submit();
                    } else {
                        self.accept_popup();
                    }
                    return;
                }
                KeyCode::Esc => {
                    self.popup = None;
                    return;
                }
                _ => {}
            }
        }

        if k.code == KeyCode::Up && self.editor.is_empty() && !self.current().queue.is_empty() {
            self.edit_queued();
            return;
        }
        let e = &mut self.editor;
        match k.code {
            KeyCode::Enter if shift || alt => e.insert("\n"),
            KeyCode::Enter if e.text[..e.cursor].ends_with('\\') => {
                e.backspace();
                e.insert("\n");
            }
            KeyCode::Enter => return self.submit(),
            KeyCode::Char('j') if ctrl => e.insert("\n"),
            KeyCode::Char('v') if ctrl => {
                match clipboard_image() {
                    Some(path) => e.attach_image(path),
                    None => self.toast("no image on the clipboard — ⌘V pastes text"),
                }
                return;
            }
            KeyCode::Esc => {
                if self.current().busy() {
                    self.interrupt();
                } else if self.current_mut().resume_at.take().is_some() {
                    self.toast("auto-resume cancelled");
                    self.persist();
                }
                return;
            }
            KeyCode::Tab => return,
            KeyCode::Char('d') if ctrl && e.is_empty() => return self.close_active(),
            KeyCode::Char('d') if ctrl => e.delete(),
            KeyCode::Char('a') if ctrl => e.home(),
            KeyCode::Char('e') if ctrl => e.end(),
            KeyCode::Char('u') if ctrl => e.kill_line_back(),
            KeyCode::Char('k') if ctrl => e.kill_line_fwd(),
            KeyCode::Char('w') if ctrl => e.delete_word(),
            KeyCode::Char('l') if ctrl => self.current_mut().scroll = 0,
            KeyCode::Backspace if alt || ctrl => e.delete_word(),
            KeyCode::Backspace => e.backspace(),
            KeyCode::Delete => e.delete(),
            KeyCode::Left if alt || ctrl => e.word_left(),
            KeyCode::Right if alt || ctrl => e.word_right(),
            KeyCode::Char('b') if alt => e.word_left(),
            KeyCode::Char('f') if alt => e.word_right(),
            KeyCode::Left => e.left(),
            KeyCode::Right => e.right(),
            KeyCode::Home => e.home(),
            KeyCode::End => e.end(),
            KeyCode::Up => e.up(),
            KeyCode::Down => e.down(),
            KeyCode::Char(c) if !ctrl => {
                let mut buf = [0u8; 4];
                e.insert(c.encode_utf8(&mut buf));
            }
            _ => return,
        }
        self.refresh_popup();
    }

    fn selectable(&self) -> Vec<usize> {
        self.current()
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match &e.item {
                Item::Tool(_) => !crate::session::quiet(&e.item),
                Item::Assistant(t) => t.contains("```"),
                _ => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn focus_transcript(&mut self) {
        let sel = self.selectable();
        if sel.is_empty() {
            self.toast("nothing to expand yet");
            return;
        }
        self.focus = Focus::Transcript;
        let s = self.current_mut();
        if s.selected.is_none_or(|i| !sel.contains(&i)) {
            s.selected = sel.last().copied();
        }
        s.ensure_visible = true;
    }

    fn on_transcript_key(&mut self, k: KeyEvent) {
        let sel = self.selectable();
        let s = &mut self.sessions[self.active];
        let pos = s.selected.and_then(|i| sel.iter().position(|&x| x == i));
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(p) = pos {
                    s.selected = Some(sel[p.saturating_sub(1)]);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(p) = pos {
                    s.selected = Some(sel[(p + 1).min(sel.len() - 1)]);
                }
            }
            KeyCode::Char('g') | KeyCode::Home => s.selected = sel.first().copied(),
            KeyCode::Char('G') | KeyCode::End => s.selected = sel.last().copied(),
            KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('o') => {
                if let Some(i) = s.selected {
                    let e = &mut s.entries[i];
                    e.expanded = !e.expanded;
                }
            }
            KeyCode::Esc | KeyCode::Char('i') => {
                self.focus = Focus::Input;
                return;
            }
            KeyCode::Char(c) => {
                self.focus = Focus::Input;
                let mut buf = [0u8; 4];
                self.editor.insert(c.encode_utf8(&mut buf));
                self.refresh_popup();
                return;
            }
            _ => return,
        }
        s.ensure_visible = true;
    }

    fn on_modal_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if let Some(Modal::Tutorial { step }) = &mut self.modal {
            let last = crate::ui::tutorial::STEPS - 1;
            match k.code {
                KeyCode::Esc => self.finish_tutorial(),
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Backspace if *step > 0 => {
                    *step -= 1;
                    self.step_at = Some(Instant::now());
                }
                KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Tab => {
                    if *step >= last {
                        self.finish_tutorial();
                    } else {
                        *step += 1;
                        self.step_at = Some(Instant::now());
                    }
                }
                _ => {}
            }
            return;
        }
        if matches!(self.modal, Some(Modal::Board(_))) {
            return self.board_key(k);
        }
        if matches!(
            self.modal,
            Some(
                Modal::Model { .. }
                    | Modal::Files { .. }
                    | Modal::Search { .. }
                    | Modal::Theme { .. }
                    | Modal::Settings { .. }
            )
        ) {
            return self.panel_key(k);
        }
        let choices = matches!(self.modal, Some(Modal::Project { .. })).then(|| self.project_choices());
        let all_ws = if choices.is_some() { self.all_workspaces() } else { Vec::new() };
        let Some(modal) = &mut self.modal else { return };
        if k.code == KeyCode::Esc || (ctrl && k.code == KeyCode::Char('c')) {
            self.modal = None;
            return;
        }
        match modal {
            Modal::Board(_)
            | Modal::Model { .. }
            | Modal::Files { .. }
            | Modal::Search { .. }
            | Modal::Theme { .. }
            | Modal::Settings { .. } => {}
            Modal::Help | Modal::Stats | Modal::Agents | Modal::Tutorial { .. } => {
                self.modal = None;
                if matches!(k.code, KeyCode::Char(_)) && !ctrl {
                    self.on_key(k);
                }
            }
            Modal::Project { query, sel } => {
                let picks =
                    crate::projects::picker_with(&query.text, choices.as_deref().unwrap_or(&[]), &all_ws);
                match k.code {
                    KeyCode::Up => *sel = sel.saturating_sub(1),
                    KeyCode::Down => *sel = (*sel + 1).min(picks.len().saturating_sub(1)),
                    KeyCode::Enter => {
                        let pick = picks.get(*sel).cloned();
                        self.modal = None;
                        match pick {
                            Some(crate::projects::Pick::Project(p) | crate::projects::Pick::Open(p)) => {
                                self.new_session_at(&p.to_string_lossy())
                            }
                            Some(crate::projects::Pick::Create(p)) => match std::fs::create_dir_all(&p) {
                                Ok(()) => self.new_session_at(&p.to_string_lossy()),
                                Err(e) => self.toast(format!("couldn't create folder: {e}")),
                            },
                            None => self.toast("no matching project"),
                        }
                    }
                    KeyCode::Tab => {
                        if crate::projects::looks_like_path(&query.text) {
                            complete_dir(query);
                        } else if let Some(crate::projects::Pick::Project(p)) = picks.get(*sel) {
                            query.set(&format!("{}/", tools::tilde(&p.to_string_lossy())));
                        }
                        *sel = 0;
                    }
                    _ => {
                        edit_line(query, k);
                        *sel = 0;
                    }
                }
            }
            Modal::Switcher { query, sel } => {
                let hits = switcher_hits(&self.sessions, &query.text);
                match k.code {
                    KeyCode::Up => {
                        let p = hits.iter().position(|h| h == sel).unwrap_or(0);
                        *sel = hits[p.saturating_sub(1).min(hits.len().saturating_sub(1))];
                    }
                    KeyCode::Down => {
                        let p = hits.iter().position(|h| h == sel).map_or(0, |p| p + 1);
                        if let Some(&h) = hits.get(p.min(hits.len().saturating_sub(1))) {
                            *sel = h;
                        }
                    }
                    KeyCode::Enter => {
                        let target = if hits.contains(sel) { Some(*sel) } else { hits.first().copied() };
                        self.modal = None;
                        if let Some(t) = target {
                            self.switch_to(t);
                        }
                    }
                    KeyCode::Char('n') if ctrl => self.open_new_modal(),
                    _ => {
                        edit_line(query, k);
                        let hits = switcher_hits(&self.sessions, &query.text);
                        if !hits.contains(sel)
                            && let Some(&h) = hits.first()
                        {
                            *sel = h;
                        }
                    }
                }
            }
        }
    }
}

impl App {
    fn panel_key(&mut self, k: KeyEvent) {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if k.code == KeyCode::Esc || (ctrl && k.code == KeyCode::Char('c')) {
            if let Some(Modal::Theme { prev, .. }) = self.modal {
                crate::theme::set_index(prev);
            }
            self.modal = None;
            return;
        }
        if let Some(Modal::Settings { sel }) = &mut self.modal {
            let n = SETTINGS.len();
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => *sel = (*sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => *sel = (*sel + 1) % n,
                KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Left | KeyCode::Right => {
                    let row = *sel;
                    self.toggle_setting(row);
                }
                KeyCode::Char('q') => self.modal = None,
                _ => {}
            }
            return;
        }
        if let Some(Modal::Theme { sel, .. }) = &mut self.modal {
            let n = crate::theme::PALETTES.len();
            match k.code {
                KeyCode::Up | KeyCode::Char('k') | KeyCode::Left => *sel = (*sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Right | KeyCode::Tab => *sel = (*sel + 1) % n,
                KeyCode::Enter => {
                    self.modal = None;
                    self.save_theme();
                    return;
                }
                _ => return,
            }
            // Live preview.
            crate::theme::set_index(*sel);
            return;
        }
        let models = self.model_choices();
        let n_files = self.current().changes.len();
        match &mut self.modal {
            Some(Modal::Model { sel, effort }) => {
                let n = models.len().max(1);
                let efforts = models.get(*sel).map(Self::effort_choices).unwrap_or_default();
                match k.code {
                    KeyCode::Up | KeyCode::Char('k') => *sel = (*sel + n - 1) % n,
                    KeyCode::Down | KeyCode::Char('j') => *sel = (*sel + 1) % n,
                    KeyCode::Left | KeyCode::Char('h') => *effort = effort.saturating_sub(1),
                    KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                        *effort = (*effort + 1).min(efforts.len().saturating_sub(1))
                    }
                    KeyCode::Enter => {
                        let (sel, effort) = (*sel, *effort);
                        self.modal = None;
                        if let Some(m) = models.get(sel) {
                            self.set_model(&m.value);
                            let e = Self::effort_choices(m).get(effort).cloned().filter(|e| e != "auto");
                            self.set_effort(e);
                        }
                    }
                    _ => {}
                }
                if let Some(Modal::Model { sel, effort }) = &mut self.modal {
                    let len = models.get(*sel).map_or(1, |m| Self::effort_choices(m).len());
                    *effort = (*effort).min(len - 1);
                }
            }
            Some(Modal::Files { sel, scroll }) => match k.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    *sel = sel.saturating_sub(1);
                    *scroll = 0;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    *sel = (*sel + 1).min(n_files.saturating_sub(1));
                    *scroll = 0;
                }
                KeyCode::PageDown | KeyCode::Char(' ') => *scroll = scroll.saturating_add(10),
                KeyCode::PageUp => *scroll = scroll.saturating_sub(10),
                KeyCode::Char('J') => *scroll = scroll.saturating_add(1),
                KeyCode::Char('K') => *scroll = scroll.saturating_sub(1),
                KeyCode::Char('q') | KeyCode::Enter => self.modal = None,
                _ => {}
            },
            Some(Modal::Search { query, sel }) => {
                let n = search_hits(&self.sessions, &query.text).len();
                match k.code {
                    KeyCode::Up => *sel = sel.saturating_sub(1),
                    KeyCode::Down => *sel = (*sel + 1).min(n.saturating_sub(1)),
                    KeyCode::Enter => {
                        let hit = search_hits(&self.sessions, &query.text).into_iter().nth(*sel);
                        self.modal = None;
                        if let Some(h) = hit {
                            self.switch_to(h.session);
                            let s = &mut self.sessions[h.session];
                            s.jump = Some((h.entry, Instant::now()));
                            s.jump_pending = true;
                        }
                    }
                    _ => {
                        edit_line(query, k);
                        *sel = 0;
                    }
                }
            }
            _ => {}
        }
    }
}

pub const JUMP_FLASH: Duration = Duration::from_millis(1600);

pub fn switcher_hits(sessions: &[Session], q: &str) -> Vec<usize> {
    let mut scored: Vec<(i32, usize)> = sessions
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let hay = format!("{} {}", s.name, s.cwd.display());
            files::score(q, &hay).map(|sc| (sc, i))
        })
        .collect();
    if !q.is_empty() {
        scored.sort_by_key(|x| std::cmp::Reverse(x.0));
    }
    scored.into_iter().map(|(_, i)| i).collect()
}

fn edit_line(e: &mut Editor, k: KeyEvent) {
    let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
    let alt = k.modifiers.contains(KeyModifiers::ALT);
    match k.code {
        KeyCode::Backspace if alt || ctrl => e.delete_word(),
        KeyCode::Char('w') if ctrl => e.delete_word(),
        KeyCode::Char('u') if ctrl => e.kill_line_back(),
        KeyCode::Backspace => e.backspace(),
        KeyCode::Left => e.left(),
        KeyCode::Right => e.right(),
        KeyCode::Home => e.home(),
        KeyCode::End => e.end(),
        KeyCode::Char(c) if !ctrl => {
            let mut buf = [0u8; 4];
            e.insert(c.encode_utf8(&mut buf));
        }
        _ => {}
    }
}

use crate::projects::expand;

fn complete_dir(e: &mut Editor) {
    let text = e.text.clone();
    let (dir_part, prefix) = match text.rfind('/') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("./", text.as_str()),
    };
    let Ok(rd) = std::fs::read_dir(expand(dir_part)) else { return };
    let mut matches: Vec<String> = rd
        .flatten()
        .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && (!n.starts_with('.') || prefix.starts_with('.')))
        .collect();
    matches.sort();
    let Some(first) = matches.first() else { return };
    let common = matches.iter().fold(first.clone(), |acc, m| {
        acc.chars().zip(m.chars()).take_while(|(a, b)| a == b).map(|(a, _)| a).collect()
    });
    let base = if text.contains('/') { dir_part } else { "" };
    let suffix = if matches.len() == 1 { "/" } else { "" };
    e.set(&format!("{base}{common}{suffix}"));
}

pub fn display_dir(p: &Path) -> String {
    tools::tilde(&p.to_string_lossy())
}

/// Saves an image on the macOS clipboard to a temp PNG.
fn clipboard_image() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join("den-images");
    std::fs::create_dir_all(&dir).ok()?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    let path = dir.join(format!("clipboard-{stamp}.png"));
    let script = format!(
        "set f to open for access POSIX file \"{}\" with write permission\n\
         try\n write (the clipboard as «class PNGf») to f\n close access f\n\
         on error\n close access f\n error \"none\"\n end try",
        path.display()
    );
    let ok = std::process::Command::new("osascript")
        .args(["-e", &script])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    (ok && std::fs::metadata(&path).is_ok_and(|m| m.len() > 0)).then_some(path)
}

/// Sub-folders matching a partially typed path, shown as `~/…` paths.
fn dir_completions(arg: &str, cwd: &Path) -> Vec<(String, String)> {
    let arg = arg.trim_start();
    let (dir_part, prefix) = match arg.rfind('/') {
        Some(i) => (&arg[..=i], &arg[i + 1..]),
        None => ("", arg),
    };
    let base = if dir_part.is_empty() {
        cwd.to_path_buf()
    } else {
        let d = expand(dir_part);
        if d.is_absolute() { d } else { cwd.join(d) }
    };
    let Ok(rd) = std::fs::read_dir(&base) else { return vec![] };
    let mut names: Vec<String> = rd
        .flatten()
        .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
        .map(|d| d.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            (!n.starts_with('.') || prefix.starts_with('.')) && n.to_lowercase().starts_with(&prefix.to_lowercase())
        })
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names
        .into_iter()
        .take(200)
        .map(|n| (format!("{dir_part}{n}"), String::new()))
        .collect()
}

/// Plan usage windows from a `rate_limit_event`.
pub fn limit_name(key: &str) -> &'static str {
    match key {
        "five_hour" => "5-hour",
        "seven_day" => "weekly",
        "seven_day_opus" => "weekly Opus",
        _ => "plan",
    }
}

/// "2h 10m" until `resets_at` (epoch seconds), if it's in the future.
pub fn reset_in(resets_at: u64, now: u64) -> Option<String> {
    let left = resets_at.checked_sub(now).filter(|s| *s > 0)?;
    let (d, h, m) = (left / 86400, left % 86400 / 3600, left % 3600 / 60);
    Some(if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{}m", m.max(1))
    })
}

fn parse_limits(v: &Value) -> Vec<store::Limit> {
    let info = &v["rate_limit_info"];
    let mut out = Vec::new();
    if let Some(windows) = info.get("unifiedWindows").and_then(Value::as_object) {
        for (key, w) in windows {
            out.push(store::Limit {
                key: key.clone(),
                utilization: w.get("utilization").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                resets_at: w.get("resetsAt").and_then(Value::as_u64).unwrap_or(0),
            });
        }
    } else if let Some(u) = info.get("utilization").and_then(Value::as_f64) {
        out.push(store::Limit {
            key: tools::s(info, "rateLimitType").to_string(),
            utilization: u as f32,
            resets_at: info.get("resetsAt").and_then(Value::as_u64).unwrap_or(0),
        });
    }
    out.sort_by_key(|l| match l.key.as_str() {
        "five_hour" => 0,
        "seven_day" => 1,
        _ => 2,
    });
    out
}

/// Local wall-clock time like "3:40 PM".
fn clock(epoch: u64) -> String {
    let t = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let (h, m) = (tm.tm_hour, tm.tm_min);
    let h12 = if h % 12 == 0 { 12 } else { h % 12 };
    format!("{h12}:{m:02} {}", if h < 12 { "AM" } else { "PM" })
}

fn today() -> String {
    std::process::Command::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{base64, reset_in};

    #[test]
    fn reset_countdown() {
        assert_eq!(reset_in(1000 + 2 * 3600 + 600, 1000).as_deref(), Some("2h 10m"));
        assert_eq!(reset_in(1000 + 3 * 86400 + 3600, 1000).as_deref(), Some("3d 1h"));
        assert_eq!(reset_in(10, 1000), None);
    }

    #[test]
    fn limits_parse_windows() {
        let v = serde_json::json!({"type":"rate_limit_event","rate_limit_info":{"unifiedWindows":{
            "seven_day":{"utilization":0.05,"resetsAt":20},"five_hour":{"utilization":0.18,"resetsAt":10}}}});
        let l = super::parse_limits(&v);
        assert_eq!(l[0].key, "five_hour");
        assert!((l[0].utilization - 0.18).abs() < 1e-6);
        assert_eq!(l[1].resets_at, 20);
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b"hi"), "aGk=");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
    }
}

