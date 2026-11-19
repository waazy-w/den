//! Token optimizer. Noisy command output (builds, installs, test runs) is the
//! biggest thing filling Claude's context. A Claude Code PreToolUse hook
//! (`den hook`) rewrites such commands to run under `den x`, which prints a
//! cleaned-up version: no color codes or progress bars, repeats collapsed, and
//! huge logs cut to the start, the end and every error line. The full output
//! is saved so Claude can still read it.
//!
//! Claude Code checks permissions against the rewritten command, so den only
//! rewrites when the user's own allow rules (or a read-only command) already
//! cover every part of it, and then says "allow" on their behalf. Anything
//! else passes through untouched.

use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Commands whose output is worth squeezing.
const NOISY: &[&str] = &[
    "cargo", "npm", "npx", "pnpm", "yarn", "bun", "pytest", "go", "make", "gradle", "./gradlew", "mvn", "docker", "pip",
    "pip3", "brew", "tsc", "eslint", "jest", "vitest", "swift", "xcodebuild", "rspec", "bundle", "terraform", "pod",
    "flutter", "dotnet", "deno", "uv", "poetry",
];


/// Lines starting with these words are progress chatter when they repeat.
const PROGRESS: &[&str] = &[
    "Compiling", "Checking", "Downloading", "Downloaded", "Fresh", "Installing", "Collecting", "Building", "Updating",
    "Resolving", "Fetching", "Unpacking", "Preparing", "Using", "Requirement", "Locking", "Adding", "Removing",
];

const MAX_LINES: usize = 400;
const MAX_BYTES: usize = 48_000;
const HEAD: usize = 120;
const TAIL: usize = 80;
const MAX_LINE: usize = 2_000;

fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match it.peek() {
            Some('[') => {
                it.next();
                for d in it.by_ref() {
                    if ('@'..='~').contains(&d) {
                        break;
                    }
                }
            }
            Some(']') => {
                it.next();
                while let Some(d) = it.next() {
                    if d == '\x07' || (d == '\x1b' && it.peek() == Some(&'\\')) {
                        if d == '\x1b' {
                            it.next();
                        }
                        break;
                    }
                }
            }
            _ => {
                it.next();
            }
        }
    }
    out
}

/// Cleanup that loses nothing meaningful: color codes, progress-bar redraws,
/// trailing spaces, repeated lines and blank runs. Used for pastes too.
pub fn clean(raw: &str) -> String {
    let text = strip_ansi(raw);
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        // A carriage return redraws the line: keep what was drawn last.
        let line = line.rsplit('\r').next().unwrap_or(line).trim_end();
        let line = if line.len() > MAX_LINE {
            let mut cut = MAX_LINE;
            while !line.is_char_boundary(cut) {
                cut -= 1;
            }
            format!("{}… [+{} chars]", &line[..cut], line.len() - cut)
        } else {
            line.to_string()
        };
        lines.push(line);
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let mut j = i + 1;
        while j < lines.len() && lines[j] == lines[i] {
            j += 1;
        }
        let n = j - i;
        if lines[i].is_empty() {
            if out.last().is_none_or(|l| !l.is_empty()) {
                out.push(String::new());
            }
        } else if n > 1 {
            out.push(format!("{}  (×{n})", lines[i]));
        } else {
            out.push(lines[i].clone());
        }
        i = j;
    }
    out.join("\n")
}

fn first_word(l: &str) -> &str {
    l.split_whitespace().next().unwrap_or("")
}

fn important(l: &str) -> bool {
    let low = l.to_lowercase();
    ["error", "warn", "fail", "panic", "exception", "traceback", "assert", "✗", "fatal", "denied", "not found"]
        .iter()
        .any(|k| low.contains(k))
}

/// Full squeeze for command output. `full` names where the untouched output
/// is saved, mentioned when anything had to be cut.
pub fn squeeze(raw: &str, full: &str) -> (String, bool) {
    let cleaned = clean(raw);
    let lines: Vec<&str> = cleaned.lines().collect();
    // Collapse runs of progress lines ("Compiling a", "Compiling b", …).
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let w = first_word(lines[i]);
        let mut j = i + 1;
        if PROGRESS.contains(&w) {
            while j < lines.len() && first_word(lines[j]) == w {
                j += 1;
            }
        }
        if j - i >= 6 {
            out.push(lines[i].to_string());
            out.push(format!("  … {} more “{w}” lines", j - i - 2));
            out.push(lines[j - 1].to_string());
        } else {
            out.extend(lines[i..j].iter().map(|s| s.to_string()));
        }
        i = j;
    }
    let bytes: usize = out.iter().map(|l| l.len() + 1).sum();
    if out.len() <= MAX_LINES && bytes <= MAX_BYTES {
        return (out.join("\n"), false);
    }
    // Too big: keep the start, the end, and the errors in between.
    let (head, tail) = (HEAD.min(out.len()), TAIL.min(out.len().saturating_sub(HEAD)));
    let middle = &out[head..out.len() - tail];
    let mut kept = out[..head].to_vec();
    let mut skipped = 0;
    let mut shown_errors = 0;
    for l in middle {
        if important(l) && shown_errors < 60 {
            if skipped > 0 {
                kept.push(format!("  … {skipped} lines …"));
                skipped = 0;
            }
            kept.push(l.clone());
            shown_errors += 1;
        } else {
            skipped += 1;
        }
    }
    if skipped > 0 {
        kept.push(format!("  … {skipped} lines …"));
    }
    kept.extend(out[out.len() - tail..].iter().cloned());
    kept.push(format!("[den trimmed {} of {} lines · full output: {full}]", middle.len() - shown_errors, out.len()));
    (kept.join("\n"), true)
}

// ---- command rewriting ----

/// Splits a shell command into top-level simple commands, returning each
/// one's byte range. None if it's too clever to touch safely.
fn segments(cmd: &str) -> Option<Vec<(usize, usize)>> {
    if cmd.contains("<<") || cmd.contains('`') || cmd.contains("$(") {
        return None;
    }
    let b = cmd.as_bytes();
    let (mut out, mut start, mut i) = (Vec::new(), 0, 0);
    let (mut sq, mut dq) = (false, false);
    let mut depth = 0i32;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\\' if !sq => i += 1,
            b'\'' if !dq => sq = !sq,
            b'"' if !sq => dq = !dq,
            b'(' | b'{' if !sq && !dq => depth += 1,
            b')' | b'}' if !sq && !dq => depth -= 1,
            b'&' | b'|' | b';' | b'\n' if !sq && !dq && depth == 0 => {
                out.push((start, i));
                if i + 1 < b.len() && (b[i + 1] == b'&' || b[i + 1] == b'|') && (c == b'&' || c == b'|') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if sq || dq || depth != 0 {
        return None;
    }
    out.push((start, b.len()));
    Some(out.into_iter().filter(|(a, z)| cmd[*a..*z].trim() != "").collect())
}

fn words(seg: &str) -> Vec<&str> {
    seg.split_whitespace().collect()
}

/// The rewritten command and every simple command in it, if any part is noisy.
pub fn rewrite(cmd: &str, exe: &str) -> Option<(String, Vec<String>)> {
    if cmd.contains("rtk ") || cmd.contains(" x ") && cmd.contains("den") {
        return None;
    }
    let segs = segments(cmd)?;
    let mut out = String::new();
    let mut last = 0;
    let mut changed = false;
    let mut parts = Vec::new();
    for (a, z) in segs {
        let seg = &cmd[a..z];
        parts.push(seg.trim().to_string());
        let lead = seg.len() - seg.trim_start().len();
        let w = words(seg);
        let Some(first) = w.first() else { continue };
        // Env assignments, subshells and redirect-first forms stay as they are.
        if first.contains('=') || first.starts_with('(') || first.starts_with('<') || first.starts_with('>') {
            continue;
        }
        if NOISY.contains(first) {
            out.push_str(&cmd[last..a + lead]);
            out.push_str(exe);
            out.push_str(" x ");
            last = a + lead;
            changed = true;
        }
    }
    out.push_str(&cmd[last..]);
    changed.then_some((out, parts))
}

fn shell_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

// ---- permission rules ----

#[derive(Default)]
struct Rules {
    allow: Vec<String>,
    deny: Vec<String>,
}

fn load_rules(cwd: &Path) -> Rules {
    let home = crate::platform::home();
    let program_data = std::env::var_os("ProgramData").map_or_else(|| PathBuf::from(r"C:\ProgramData"), PathBuf::from);
    let managed = if cfg!(windows) {
        [program_data.join(r"ClaudeCode\managed-settings.json"), program_data.join(r"ClaudeCode\managed-settings.json")]
    } else {
        [
            PathBuf::from("/Library/Application Support/ClaudeCode/managed-settings.json"),
            PathBuf::from("/etc/claude-code/managed-settings.json"),
        ]
    };
    let files = [
        // Organization policy (can only add restrictions here).
        managed[0].clone(),
        managed[1].clone(),
        home.join(".claude/settings.json"),
        home.join(".claude/settings.local.json"),
        cwd.join(".claude/settings.json"),
        cwd.join(".claude/settings.local.json"),
    ];
    let mut r = Rules::default();
    for f in files {
        let Some(v) = std::fs::read_to_string(&f).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
            continue;
        };
        let list = |k: &str| -> Vec<String> {
            v["permissions"][k].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect()
        };
        r.allow.extend(list("allow"));
        r.deny.extend(list("deny"));
        r.deny.extend(list("ask"));
    }
    r
}

fn glob(pat: &str, s: &str) -> bool {
    let parts: Vec<&str> = pat.split('*').collect();
    if parts.len() == 1 {
        return pat == s;
    }
    let mut rest = s;
    for (k, p) in parts.iter().enumerate() {
        if k == 0 {
            let Some(r) = rest.strip_prefix(p) else { return false };
            rest = r;
        } else if k == parts.len() - 1 {
            return rest.ends_with(p);
        } else {
            let Some(at) = rest.find(p) else { return false };
            rest = &rest[at + p.len()..];
        }
    }
    true
}

/// Does a permission rule like `Bash(cargo test:*)` cover `cmd`?
fn rule_matches(rule: &str, cmd: &str) -> bool {
    let rule = rule.trim();
    if rule == "Bash" || rule == "Bash(*)" {
        return true;
    }
    let Some(inner) = rule.strip_prefix("Bash(").and_then(|r| r.strip_suffix(')')) else { return false };
    if let Some(prefix) = inner.strip_suffix(":*") {
        return cmd == prefix || cmd.starts_with(&format!("{prefix} "));
    }
    glob(inner, cmd)
}

/// Harmless on their own: `pwd`, or `cd` into a sub-folder.
fn harmless(part: &str) -> bool {
    let w: Vec<&str> = part.split_whitespace().collect();
    match w.as_slice() {
        ["pwd"] => true,
        // Only plain relative folders: no roots, drives, home, or `..` (either slash).
        ["cd", dir] => {
            !dir.starts_with(['/', '\\', '~']) && !dir.contains(':') && !dir.split(['/', '\\']).any(|c| c == "..")
        }
        _ => false,
    }
}

/// den only says "allow" for what the user's own rules already allow.
fn allowed(rules: &Rules, part: &str) -> bool {
    harmless(part) || rules.allow.iter().any(|r| rule_matches(r, part))
}

/// Redirects, expansions and grouping can write files or run other code;
/// den never vouches for commands that use them (`2>&1` is fine).
fn risky(cmd: &str) -> bool {
    cmd.replace("2>&1", "").chars().any(|c| matches!(c, '>' | '<' | '$' | '`' | '(' | ')' | '{' | '}'))
}

/// The PreToolUse hook: reads Claude Code's JSON on stdin, prints a decision.
pub fn hook() -> i32 {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return 0;
    }
    let Ok(v) = serde_json::from_str::<Value>(&input) else { return 0 };
    let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "den".into());
    // Claude Code's Bash on Windows is Git Bash, which wants C:/x/den.exe.
    let exe = if cfg!(windows) { exe.replace('\\', "/") } else { exe };
    if let Some(out) = decide(&v, &exe) {
        println!("{out}");
    }
    0
}

fn decide(v: &Value, exe: &str) -> Option<Value> {
    let tool = &v["tool_input"];
    let cmd = tool["command"].as_str()?;
    if tool["run_in_background"].as_bool() == Some(true) {
        return None;
    }
    let (new, parts) = rewrite(cmd, &shell_quote(exe))?;
    let mode = v["permission_mode"].as_str().unwrap_or("default");
    let mut updated = tool.clone();
    updated["command"] = json!(new);
    if mode == "bypassPermissions" {
        return Some(json!({"hookSpecificOutput": {"hookEventName": "PreToolUse", "updatedInput": updated}}));
    }
    // Plan mode runs nothing; other modes keep Claude Code's own judgment.
    if !matches!(mode, "default" | "acceptEdits" | "auto") || risky(cmd) {
        return None;
    }
    let cwd = PathBuf::from(v["cwd"].as_str().unwrap_or("."));
    let rules = load_rules(&cwd);
    let blocked = parts.iter().chain(std::iter::once(&cmd.to_string())).any(|p| rules.deny.iter().any(|r| rule_matches(r, p)));
    if blocked || !parts.iter().all(|p| allowed(&rules, p)) {
        return None;
    }
    Some(json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "allow",
        "permissionDecisionReason": "den: allowed by your rules; output squeezed",
        "updatedInput": updated,
    }}))
}

// ---- den x: run and squeeze ----

fn cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::platform::home().join(".cache"));
    base.join("den")
}

/// Bytes before and after, summed over every squeezed command.
pub fn savings() -> (u64, u64, u64) {
    let Ok(text) = std::fs::read_to_string(cache_dir().join("squeeze.tsv")) else { return (0, 0, 0) };
    text.lines().fold((0, 0, 0), |(n, b, a), l| {
        let mut it = l.split('\t').filter_map(|x| x.parse::<u64>().ok());
        match (it.next(), it.next()) {
            (Some(x), Some(y)) => (n + 1, b + x, a + y),
            _ => (n, b, a),
        }
    })
}

/// Runs `args` with stdout and stderr merged, prints the squeezed output and
/// exits with the command's status.
pub fn run(args: &[String]) -> i32 {
    let Some((prog, rest)) = args.split_first() else { return 2 };
    let Ok((r, w)) = std::io::pipe() else { return 126 };
    let w2 = match w.try_clone() {
        Ok(f) => f,
        Err(_) => return 126,
    };
    let child = std::process::Command::new(crate::platform::find_program(prog)).args(rest).stdout(w).stderr(w2).spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            eprintln!("den: {prog}: {e}");
            return if e.kind() == std::io::ErrorKind::NotFound { 127 } else { 126 };
        }
    };
    let mut raw = Vec::new();
    let mut r = r;
    let _ = r.read_to_end(&mut raw);
    let status = child.wait().map_or(1, |s| {
        #[cfg(unix)]
        let signal = {
            use std::os::unix::process::ExitStatusExt;
            s.signal().unwrap_or(0)
        };
        #[cfg(not(unix))]
        let signal = 0;
        s.code().unwrap_or(128 + signal)
    });
    let text = String::from_utf8_lossy(&raw);
    let dir = cache_dir();
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
    let full = dir.join("out").join(format!("{stamp}.log"));
    let (out, trimmed) = squeeze(&text, &full.to_string_lossy());
    if trimmed {
        save_private(&full, text.as_bytes());
    }
    let mut stdout = std::io::stdout();
    let _ = stdout.write_all(out.as_bytes());
    if !out.is_empty() {
        let _ = stdout.write_all(b"\n");
    }
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("squeeze.tsv")) {
        let _ = writeln!(f, "{}\t{}", raw.len(), out.len());
    }
    status
}

fn save_private(path: &Path, data: &[u8]) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    if let Ok(mut f) = opts.open(path) {
        let _ = f.write_all(data);
    }
}

/// Is RTK already rewriting commands (so den shouldn't add its own hook)?
pub fn rtk_active() -> bool {
    let home = crate::platform::home();
    std::fs::read_to_string(home.join(".claude/settings.json")).is_ok_and(|s| s.contains("rtk hook"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_strips_color_progress_and_repeats() {
        let raw = "\x1b[32mok\x1b[0m\nload 10%\rload 100%\nsame\nsame\nsame\n\n\n\nend   \n";
        assert_eq!(clean(raw), "ok\nload 100%\nsame  (×3)\n\nend");
    }

    #[test]
    fn progress_runs_collapse() {
        let raw: String = (0..20).map(|i| format!("   Compiling crate{i} v1.0\n")).collect::<String>() + "Finished";
        let (out, trimmed) = squeeze(&raw, "/x");
        assert!(!trimmed);
        assert_eq!(out.lines().count(), 4);
        assert!(out.contains("18 more “Compiling” lines"));
    }

    #[test]
    fn huge_output_keeps_ends_and_errors() {
        let mut raw = String::new();
        for i in 0..2000 {
            raw.push_str(&format!("line {i}\n"));
            if i == 1000 {
                raw.push_str("error[E0308]: mismatched types\n");
            }
        }
        let (out, trimmed) = squeeze(&raw, "/tmp/full.log");
        assert!(trimmed);
        assert!(out.contains("line 0\n") && out.contains("line 1999"));
        assert!(out.contains("error[E0308]"));
        assert!(out.contains("full output: /tmp/full.log"));
        assert!(out.lines().count() < 230);
    }

    #[test]
    fn rewrites_noisy_commands_only() {
        let (c, parts) = rewrite("cd app && cargo test --quiet | tail -5", "/b/den").unwrap();
        assert_eq!(c, "cd app && /b/den x cargo test --quiet | tail -5");
        assert_eq!(parts, vec!["cd app", "cargo test --quiet", "tail -5"]);
        assert!(rewrite("ls -la", "den").is_none());
        assert!(rewrite("rtk cargo test", "den").is_none());
        assert!(rewrite("FOO=1 cargo test", "den").is_none());
        assert!(rewrite("cat <<EOF\nx\nEOF", "den").is_none());
        assert!(rewrite("echo 'a && cargo b'", "den").is_none());
    }

    #[test]
    fn rules_match_like_claude_code() {
        assert!(rule_matches("Bash(cargo test:*)", "cargo test --quiet"));
        assert!(rule_matches("Bash(cargo test:*)", "cargo test"));
        assert!(!rule_matches("Bash(cargo test:*)", "cargo testing"));
        assert!(rule_matches("Bash(npm run *)", "npm run build"));
        assert!(rule_matches("Bash", "anything"));
        assert!(!rule_matches("Read(*)", "cargo test"));
    }

    #[test]
    fn hook_only_allows_what_rules_allow() {
        let input = |cmd: &str, mode: &str| json!({"tool_input": {"command": cmd}, "permission_mode": mode, "cwd": "/nonexistent"});
        // No rules: leave it to Claude Code.
        let none = Rules::default();
        assert!(!allowed(&none, "cargo test"));
        assert!(allowed(&none, "cd src"));
        assert!(!allowed(&none, "cd ../.."));
        assert!(!allowed(&none, r"cd ..\.."));
        assert!(!allowed(&none, r"cd C:\Windows"));
        assert!(!allowed(&none, r"cd \\server\share"));
        assert!(!allowed(&none, "cat secret"));
        let rules = Rules { allow: vec!["Bash(cargo build:*)".into()], deny: vec![] };
        assert!(allowed(&rules, "cargo build --release"));
        assert!(!allowed(&rules, "echo x"));
        assert!(risky("cat a > ~/.bashrc && cargo build"));
        assert!(risky("cargo build $(evil)"));
        assert!(!risky("cargo test 2>&1"));
        assert!(decide(&input("cargo build", "plan"), "den").is_none());
        // bypassPermissions always squeezes.
        let out = decide(&input("cargo build", "bypassPermissions"), "den").unwrap();
        assert_eq!(out["hookSpecificOutput"]["updatedInput"]["command"], "den x cargo build");
        assert!(out["hookSpecificOutput"]["permissionDecision"].is_null());
    }
}
