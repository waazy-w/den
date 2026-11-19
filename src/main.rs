mod anim;
mod app;
mod claude;
mod files;
mod input;
mod notify;
mod projects;
mod session;
mod squeeze;
mod store;
mod theme;
mod tools;
mod ui;
mod update;

use anyhow::Result;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste, EnableFocusChange, EnableMouseCapture, EventStream,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use futures_util::StreamExt;
use std::io::stdout;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::{Instant, sleep_until};

const FRAME: Duration = Duration::from_millis(16);

struct Opts {
    dir: PathBuf,
    open_new: bool,
    mouse: bool,
    reloaded: bool,
}

fn parse_args() -> Result<Opts> {
    let mut dir = None;
    let mut mouse = true;
    let mut reloaded = false;
    for a in std::env::args().skip(1) {
        match a.as_str() {
            "-h" | "--help" => {
                println!(
                    "den — a clean terminal UI for Claude Code\n\n\
                     usage: den [dir] [--no-mouse]\n\n\
                     Opens your saved sessions; with [dir], also starts a new session there.\n\
                     --no-mouse  keep native terminal text selection (disables wheel scroll)\n\
                     den update  install the newest release now\n\n\
                     env: DEN_CLAUDE_BIN  path to the claude binary (default: claude)"
                );
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("den {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "update" | "--update" if !std::path::Path::new(&a).is_dir() => {
                println!("den {} · checking github.com/{} …", update::VERSION, update::REPO);
                match update::run() {
                    update::Outcome::UpToDate => println!("already up to date"),
                    update::Outcome::Installed(t) => println!("installed {t}"),
                    update::Outcome::Failed(e) => {
                        eprintln!("update failed: {e}");
                        std::process::exit(1);
                    }
                }
                std::process::exit(0);
            }
            "test-notify" => {
                notify::send("notifications work", "this is how den tells you Claude is done");
                std::thread::sleep(Duration::from_secs(4));
                std::process::exit(0);
            }
            "--no-mouse" => mouse = false,
            "--reloaded" => reloaded = true,
            other => dir = Some(PathBuf::from(other)),
        }
    }
    let open_new = dir.is_some();
    let dir = match dir {
        Some(d) => std::fs::canonicalize(&d).map_err(|e| anyhow::anyhow!("{}: {e}", d.display()))?,
        None => std::env::current_dir()?,
    };
    Ok(Opts { dir, open_new: open_new && !reloaded, mouse, reloaded })
}

fn restore_extras(mouse: bool) {
    let mut out = stdout();
    let _ = execute!(out, PopKeyboardEnhancementFlags, DisableBracketedPaste, DisableFocusChange);
    if mouse {
        let _ = execute!(out, DisableMouseCapture);
    }
}

/// Installs from before the rename run as `ctui`; add a `den` beside it.
fn link_new_name() {
    let Ok(exe) = std::env::current_exe() else { return };
    if exe.file_name().is_some_and(|n| n == "ctui") {
        let den = exe.with_file_name("den");
        if !den.exists() {
            let _ = std::os::unix::fs::symlink(&exe, &den);
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Fast paths used by Claude Code's Bash hook; no terminal, no config.
    let argv: Vec<String> = std::env::args().collect();
    match argv.get(1).map(String::as_str) {
        Some("hook") => std::process::exit(squeeze::hook()),
        Some("x") => std::process::exit(squeeze::run(&argv[2..])),
        _ => {}
    }
    store::migrate_legacy();
    let opts = parse_args()?;
    link_new_name();
    theme::init();

    let mut terminal = ratatui::init();
    let _ = execute!(
        stdout(),
        crossterm::terminal::Clear(crossterm::terminal::ClearType::All),
        crossterm::cursor::MoveTo(0, 0)
    );
    let _ = terminal.clear();
    let mouse = opts.mouse;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_extras(mouse);
        hook(info);
    }));
    {
        let mut out = stdout();
        let _ = execute!(out, EnableBracketedPaste, EnableFocusChange);
        if crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false) {
            let _ = execute!(
                out,
                PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
            );
        }
        if mouse {
            let _ = execute!(out, EnableMouseCapture);
        }
    }

    let result = run(&mut terminal, opts).await;
    restore_extras(mouse);
    ratatui::restore();
    if let Ok(true) = result {
        use std::os::unix::process::CommandExt;
        // On Linux a self-update leaves "/path/den (deleted)"; run the new file.
        let exe = std::env::current_exe()?;
        let exe = match exe.to_str().and_then(|p| p.strip_suffix(" (deleted)")) {
            Some(p) => PathBuf::from(p),
            None => exe,
        };
        let mut cmd = std::process::Command::new(exe);
        cmd.arg("--reloaded");
        if !mouse {
            cmd.arg("--no-mouse");
        }
        let err = cmd.exec();
        anyhow::bail!("reload failed: {err}\nYour chats were saved; run `den` to pick up exactly where you left off.");
    }
    result.map(|_| ())
}

/// Runs the UI; returns Ok(true) when the user asked to `/reload`.
async fn run(terminal: &mut ratatui::DefaultTerminal, opts: Opts) -> Result<bool> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut app = app::App::new(tx, opts.dir, opts.open_new, opts.reloaded);
    let mut events = EventStream::new();
    // Closing the terminal window or `kill` still saves everything on the way out.
    use tokio::signal::unix::{SignalKind, signal};
    let mut hangup = signal(SignalKind::hangup())?;
    let mut terminate = signal(SignalKind::terminate())?;

    let mut dirty = true;
    let mut last_draw = Instant::now() - FRAME;
    loop {
        if dirty && last_draw.elapsed() >= FRAME {
            terminal.draw(|f| ui::draw(f, &mut app))?;
            last_draw = Instant::now();
            dirty = false;
        }
        let next_frame = app.frame_interval();
        tokio::select! {
            _ = hangup.recv() => break,
            _ = terminate.recv() => break,
            ev = events.next() => match ev {
                Some(Ok(e)) => { app.on_term(e); dirty = true; }
                Some(Err(_)) | None => break,
            },
            Some(ev) = rx.recv() => {
                app.on_app(ev);
                for _ in 0..512 {
                    match rx.try_recv() {
                        Ok(ev) => app.on_app(ev),
                        Err(_) => break,
                    }
                }
                dirty = true;
            }
            _ = sleep_until(Instant::from_std(app.last_tick) + next_frame.unwrap_or_default()), if next_frame.is_some() => {
                app.tick();
                dirty = true;
            }
            _ = sleep_until(last_draw + FRAME), if dirty => {}
        }
        if app.quit {
            break;
        }
        if app.reload_ready() && app.write_snapshot(&mut rx).await {
            return Ok(true);
        }
    }
    app.persist();
    Ok(false)
}
