use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;

pub enum AppEvent {
    Claude(usize, u64, Value),
    Exit(usize, u64, String),
    Files(std::path::PathBuf, Vec<String>),
    Projects(Vec<std::path::PathBuf>),
    Catalog(Value),
    /// Result of an update check; `bool` = the user asked for it.
    Update(crate::update::Outcome, bool),
    /// A `!command` finished: session, entry, output, exit code.
    Shell(usize, usize, String, i32),
    /// Results of the setup check; `bool` = it ran by itself at startup.
    Doctor(Vec<crate::app::Check>, bool),
}

/// A running `claude` process and the tasks pumping its pipes.
#[cfg_attr(not(unix), allow(dead_code))]
pub struct Proc {
    tx: Option<mpsc::UnboundedSender<String>>,
    pid: u32,
    stdin_fd: i32,
    stdout_fd: i32,
    log: PathBuf,
    /// Bytes read from stdout that don't yet form a complete line.
    leftover: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
    writer: Option<JoinHandle<()>>,
    /// Set for processes we spawned; the waiter task kills the child on signal.
    kill: Option<oneshot::Sender<()>>,
    /// Inherited across `/reload`: no waiter task, so kill by pid.
    adopted: bool,
    /// Duplicate of stdout taken before the reader (which owns the original) stops.
    stdout_dup: i32,
}

/// What a reloaded den needs to keep talking to a still-running `claude`.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Handoff {
    pub pid: u32,
    pub stdin_fd: i32,
    pub stdout_fd: i32,
    pub log: PathBuf,
    #[serde(default)]
    pub leftover: Vec<u8>,
}

impl Drop for Proc {
    fn drop(&mut self) {
        if let Some(k) = self.kill.take() {
            let _ = k.send(());
        } else if self.adopted && self.pid > 0 {
            crate::platform::terminate(self.pid);
        }
    }
}

static REQ: AtomicU64 = AtomicU64::new(1);

pub struct Spawn<'a> {
    pub sid: usize,
    pub generation: u64,
    pub cwd: &'a Path,
    pub resume: Option<&'a str>,
    pub model: Option<&'a str>,
    pub mode: Option<&'a str>,
    pub style: Option<&'a str>,
    pub effort: Option<&'a str>,
    /// den binary to run as Claude Code's Bash hook (output squeezer).
    pub squeeze_hook: Option<&'a str>,
}

/// Per-user, owner-only log folder (`$XDG_CACHE_HOME/den/logs` or ~/.cache/den/logs).
fn log_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).unwrap_or_else(|| {
        crate::platform::home().join(".cache")
    });
    let dir = base.join("den").join("logs");
    #[cfg(not(unix))]
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let _ = std::fs::DirBuilder::new().recursive(true).mode(0o700).create(&dir);
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

fn log_path(sid: usize) -> PathBuf {
    let dir = log_dir();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    dir.join(format!("claude-{sid}-{stamp}.log"))
}

fn log_tail(path: &Path) -> String {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let lines: Vec<&str> = text.lines().rev().take(6).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

/// Splits complete lines off `buf`, leaving any partial line behind.
fn take_lines(buf: &mut Vec<u8>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    while let Some(i) = buf.iter().position(|b| *b == b'\n') {
        let mut line: Vec<u8> = buf.drain(..=i).collect();
        line.pop();
        out.push(line);
    }
    out
}

/// Reads stdout into `leftover` and emits each complete JSON line. There is no
/// await between buffering and emitting, so aborting the task never loses data.
async fn read_loop<R: AsyncRead + Unpin>(
    mut r: R,
    leftover: Arc<Mutex<Vec<u8>>>,
    sid: usize,
    generation: u64,
    events: mpsc::UnboundedSender<AppEvent>,
) {
    let mut tmp = vec![0u8; 1 << 16];
    loop {
        let n = match r.read(&mut tmp).await {
            Ok(0) | Err(_) => return,
            Ok(n) => n,
        };
        let lines = {
            let mut buf = leftover.lock().unwrap();
            buf.extend_from_slice(&tmp[..n]);
            take_lines(&mut buf)
        };
        for line in lines {
            if let Ok(v) = serde_json::from_slice::<Value>(&line)
                && events.send(AppEvent::Claude(sid, generation, v)).is_err()
            {
                return;
            }
        }
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(mut w: W, mut rx: mpsc::UnboundedReceiver<String>) {
    while let Some(line) = rx.recv().await {
        if w.write_all(line.as_bytes()).await.is_err() || w.write_all(b"\n").await.is_err() || w.flush().await.is_err() {
            break;
        }
    }
}

/// The `claude` to run: $DEN_CLAUDE_BIN, or `claude` found on PATH.
pub fn claude_bin() -> PathBuf {
    std::env::var_os("DEN_CLAUDE_BIN").map(PathBuf::from).unwrap_or_else(|| crate::platform::find_program("claude"))
}

pub fn spawn(opts: Spawn, events: mpsc::UnboundedSender<AppEvent>) -> Result<Proc> {
    let mut cmd = Command::new(claude_bin());
    cmd.args([
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
        "--verbose",
        "--include-partial-messages",
        "--permission-prompt-tool",
        "stdio",
        "--forward-subagent-text",
        "--disallowed-tools",
        "AskUserQuestion",
    ]);
    if let Some(id) = opts.resume {
        cmd.args(["--resume", id]);
    }
    if let Some(m) = opts.model {
        cmd.args(["--model", m]);
    }
    let mut settings = serde_json::Map::new();
    if let Some(st) = opts.style {
        settings.insert("outputStyle".into(), json!(st));
    }
    if let Some(exe) = opts.squeeze_hook {
        let quoted = if cfg!(windows) {
            format!("\"{}\"", exe.replace('\\', "/"))
        } else {
            format!("'{}'", exe.replace('\'', "'\\''"))
        };
        settings.insert(
            "hooks".into(),
            json!({"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": format!("{quoted} hook")}]}]}),
        );
    }
    if !settings.is_empty() {
        cmd.args(["--settings", &Value::Object(settings).to_string()]);
    }
    if let Some(e) = opts.effort {
        cmd.args(["--effort", e]);
    }
    if let Some(m) = opts.mode.filter(|m| *m != "default") {
        cmd.args(["--permission-mode", m]);
    }
    // Headless Claude Code hides its task list tools; den shows the list.
    if std::env::var_os("CLAUDE_CODE_ENABLE_TODO_TOOLS").is_none() {
        cmd.env("CLAUDE_CODE_ENABLE_TODO_TOOLS", "1");
    }
    let log = log_path(opts.sid);
    #[cfg(not(unix))]
    let err_file = std::fs::OpenOptions::new().write(true).create_new(true).open(&log).context("couldn't create claude log file")?;
    #[cfg(unix)]
    let err_file = {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&log)
            .context("couldn't create claude log file")?
    };
    cmd.current_dir(opts.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::from(err_file))
        .kill_on_drop(true);
    let mut child = cmd.spawn().context("failed to start `claude` (is it on PATH?)")?;
    let stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    #[cfg(unix)]
    let (stdin_fd, stdout_fd) = (stdin.as_raw_fd(), stdout.as_raw_fd());
    #[cfg(not(unix))]
    let (stdin_fd, stdout_fd) = (-1, -1);
    let pid = child.id().unwrap_or(0);
    let (sid, generation) = (opts.sid, opts.generation);

    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(write_loop(stdin, rx));
    let leftover = Arc::new(Mutex::new(Vec::new()));
    let ev = events.clone();
    let lo = leftover.clone();
    let log2 = log.clone();
    let reader = tokio::spawn(async move {
        read_loop(stdout, lo, sid, generation, ev.clone()).await;
        let _ = ev.send(AppEvent::Exit(sid, generation, log_tail(&log2)));
    });

    let (kill_tx, kill_rx) = oneshot::channel();
    tokio::spawn(async move {
        tokio::select! {
            _ = child.wait() => {}
            r = kill_rx => if r.is_ok() { let _ = child.kill().await; },
        }
    });

    Ok(Proc {
        tx: Some(tx),
        pid,
        stdin_fd,
        stdout_fd,
        log,
        leftover,
        reader: Some(reader),
        writer: Some(writer),
        kill: Some(kill_tx),
        adopted: false,
        stdout_dup: -1,
    })
}

/// Windows has no exec: `/reload` restarts claude with --resume instead.
#[cfg(not(unix))]
pub fn adopt(_sid: usize, _generation: u64, h: Handoff, _events: mpsc::UnboundedSender<AppEvent>) -> Result<Proc> {
    anyhow::bail!("claude process {} can't be handed over on this system", h.pid)
}

/// Reattaches to a `claude` that survived a `/reload` (same pid, inherited pipes).
#[cfg(unix)]
pub fn adopt(sid: usize, generation: u64, h: Handoff, events: mpsc::UnboundedSender<AppEvent>) -> Result<Proc> {
    use tokio::net::unix::pipe;
    let alive = crate::platform::alive(h.pid);
    anyhow::ensure!(alive, "claude process {} is gone", h.pid);
    // Don't leak these pipe ends into processes we spawn later.
    for fd in [h.stdin_fd, h.stdout_fd] {
        unsafe {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }
    let receiver = pipe::Receiver::from_owned_fd(unsafe { OwnedFd::from_raw_fd(h.stdout_fd) })?;
    let sender = pipe::Sender::from_owned_fd(unsafe { OwnedFd::from_raw_fd(h.stdin_fd) })?;
    let (tx, rx) = mpsc::unbounded_channel::<String>();
    let writer = tokio::spawn(write_loop(sender, rx));
    let leftover = Arc::new(Mutex::new(h.leftover));
    let (lo, ev, log2, pid) = (leftover.clone(), events.clone(), h.log.clone(), h.pid);
    let reader = tokio::spawn(async move {
        read_loop(receiver, lo, sid, generation, ev.clone()).await;
        // We are still its parent after exec, so reap it to avoid a zombie.
        let _ = tokio::task::spawn_blocking(move || unsafe {
            let mut status = 0;
            libc::waitpid(pid as i32, &mut status, 0);
        })
        .await;
        let _ = ev.send(AppEvent::Exit(sid, generation, log_tail(&log2)));
    });
    Ok(Proc {
        tx: Some(tx),
        pid: h.pid,
        stdin_fd: h.stdin_fd,
        stdout_fd: h.stdout_fd,
        log: h.log,
        leftover,
        reader: Some(reader),
        writer: Some(writer),
        kill: None,
        adopted: true,
        stdout_dup: -1,
    })
}

impl Proc {
    /// Stops reading (keeping any partial line), flushes pending writes, and
    /// returns duplicated pipe fds that survive exec. The process keeps running.
    /// Stops the stdout reader; unread bytes stay in `leftover`.
    pub async fn pause_reader(&mut self) {
        #[cfg(unix)]
        if self.stdout_dup < 0 {
            self.stdout_dup = unsafe { libc::dup(self.stdout_fd) };
        }
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
    }

    #[cfg(not(unix))]
    pub async fn handoff(self) -> Option<Handoff> {
        None
    }

    #[cfg(unix)]
    pub async fn handoff(mut self) -> Option<Handoff> {
        if self.stdout_dup < 0 {
            self.stdout_dup = unsafe { libc::dup(self.stdout_fd) };
        }
        let r = std::mem::replace(&mut self.stdout_dup, -1);
        let w = unsafe { libc::dup(self.stdin_fd) };
        if r < 0 || w < 0 {
            return None;
        }
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
        self.tx.take();
        if let Some(writer) = self.writer.take() {
            let _ = writer.await;
        }
        let leftover = std::mem::take(&mut *self.leftover.lock().unwrap());
        // Never signal the waiter (that would kill the child) and never kill by pid.
        if let Some(k) = self.kill.take() {
            std::mem::forget(k);
        }
        self.adopted = false;
        Some(Handoff { pid: self.pid, stdin_fd: w, stdout_fd: r, log: self.log.clone(), leftover })
    }

    fn send(&self, v: Value) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(v.to_string());
        }
    }

    pub fn user(&self, content: Value) {
        self.send(json!({"type": "user", "message": {"role": "user", "content": content}}));
    }

    pub fn control(&self, request: Value) {
        let id = format!("den-{}", REQ.fetch_add(1, Ordering::Relaxed));
        self.send(json!({"type": "control_request", "request_id": id, "request": request}));
    }

    /// Asks for the command/model catalog; answered with a `den-init` control_response.
    pub fn initialize(&self) {
        let id = format!("den-init-{}", REQ.fetch_add(1, Ordering::Relaxed));
        self.send(json!({"type": "control_request", "request_id": id, "request": {"subtype": "initialize"}}));
    }

    pub fn interrupt(&self) {
        self.control(json!({"subtype": "interrupt"}));
    }

    pub fn raw(&self, v: Value) {
        self.send(v);
    }

    pub fn permission(&self, request_id: &str, response: Value) {
        self.send(json!({
            "type": "control_response",
            "response": {"subtype": "success", "request_id": request_id, "response": response}
        }));
    }
}

/// Starts a short-lived `claude` just to read its command catalog.
pub fn fetch_catalog(cwd: std::path::PathBuf, events: mpsc::UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let Ok(mut child) = Command::new(claude_bin())
            .args(["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"])
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
        else {
            return;
        };
        let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else { return };
        let req = json!({"type": "control_request", "request_id": "den-init-catalog", "request": {"subtype": "initialize"}});
        if stdin.write_all(format!("{req}\n").as_bytes()).await.is_err() {
            return;
        }
        let read = async {
            use tokio::io::{AsyncBufReadExt, BufReader};
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&line)
                    && v.get("type").and_then(Value::as_str) == Some("control_response")
                {
                    return Some(v["response"]["response"].clone());
                }
            }
            None
        };
        if let Ok(Some(v)) = tokio::time::timeout(std::time::Duration::from_secs(30), read).await {
            let _ = events.send(AppEvent::Catalog(v));
        }
        drop(stdin);
        let _ = child.kill().await;
    });
}
