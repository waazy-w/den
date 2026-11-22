//! `den alias`: makes typing `claude` (or `claude -c`) in a terminal open den.
//! Everything else (`claude -p …`, `claude mcp add …`, scripts) still runs
//! Claude Code itself, and den always starts the real `claude` binary.

use std::path::{Path, PathBuf};

const START: &str = "# >>> den: claude opens den >>>";
const END: &str = "# <<< den <<<";

const POSIX: &str = r#"unalias claude 2>/dev/null
claude() {
  if [ -z "$DEN_NO_ALIAS" ] && [ -t 0 ] && [ -t 1 ] && command -v den >/dev/null 2>&1; then
    case "$#:$1" in
      0:) den "$PWD"; return ;;
      1:-c|1:--continue|1:-continue) den continue; return ;;
    esac
  fi
  command claude "$@"
}"#;

const FISH: &str = r#"function claude --description 'den for chats, Claude Code for the rest'
    if test -z "$DEN_NO_ALIAS"; and isatty stdin; and isatty stdout; and command -q den
        if test (count $argv) -eq 0
            den $PWD; return
        else if test (count $argv) -eq 1; and contains -- $argv[1] -c --continue -continue
            den continue; return
        end
    end
    command claude $argv
end"#;

const POWERSHELL: &str = r#"function claude {
  $real = Get-Command claude -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  $den = Get-Command den -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
  if ($den -and -not $env:DEN_NO_ALIAS -and -not [Console]::IsInputRedirected -and -not [Console]::IsOutputRedirected) {
    if ($args.Count -eq 0) { & $den.Source (Get-Location).Path; return }
    if ($args.Count -eq 1 -and @('-c', '--continue', '-continue') -contains $args[0]) { & $den.Source continue; return }
  }
  if (-not $real) { Write-Error 'claude: Claude Code is not installed'; return }
  & $real.Source @args
}"#;

/// Shell startup files that get the block, with the snippet for each.
fn targets() -> Vec<(PathBuf, &'static str)> {
    let home = crate::platform::home();
    let mut out = Vec::new();
    if cfg!(windows) {
        for p in powershell_profiles() {
            out.push((p, POWERSHELL));
        }
        // Git Bash reads ~/.bashrc.
        let bashrc = home.join(".bashrc");
        if bashrc.exists() {
            out.push((bashrc, POSIX));
        }
        return out;
    }
    let shell = std::env::var("SHELL").unwrap_or_default();
    let bash = shell.ends_with("bash");
    for (file, wanted) in [(".zshrc", shell.ends_with("zsh")), (".bashrc", bash), (".bash_profile", false)] {
        if file == ".bash_profile" && !(bash && home.join(file).exists()) {
            continue;
        }
        let p = home.join(file);
        if wanted || p.exists() {
            out.push((p, POSIX));
        }
    }
    let fish = home.join(".config/fish/conf.d");
    if shell.ends_with("fish") || fish.parent().is_some_and(Path::exists) {
        out.push((fish.join("den-claude.fish"), FISH));
    }
    out
}

/// The current user's PowerShell profiles (Windows PowerShell and PowerShell 7).
fn powershell_profiles() -> Vec<PathBuf> {
    let docs = std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "[Environment]::GetFolderPath('MyDocuments')"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::platform::home().join("Documents"));
    vec![
        docs.join("WindowsPowerShell").join("Microsoft.PowerShell_profile.ps1"),
        docs.join("PowerShell").join("Microsoft.PowerShell_profile.ps1"),
    ]
}

/// Whether PowerShell's execution policy stops profile scripts from loading.
fn profiles_blocked() -> bool {
    std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", "Get-ExecutionPolicy"])
        .output()
        .map(|o| matches!(String::from_utf8_lossy(&o.stdout).trim(), "Restricted" | "AllSigned" | "Undefined"))
        .unwrap_or(false)
}

/// The file's text with den's block taken out (and nothing else touched).
fn without_block(text: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_end();
        if t == START {
            inside = true;
            continue;
        }
        if inside {
            if t == END {
                inside = false;
            }
            continue;
        }
        out.push_str(line);
    }
    out
}

fn with_block(text: &str, snippet: &str) -> String {
    let mut out = without_block(text);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{START}\n{snippet}\n{END}\n"));
    out
}

/// Writes a shell file in place, keeping a copy of the old one beside it.
fn rewrite(path: &Path, new: &str) -> std::io::Result<()> {
    // Dotfile managers symlink these; write the real file.
    let real = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let path = real.as_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Keep the copy from before den first touched the file.
    let mut bak = path.as_os_str().to_owned();
    bak.push(".den-backup");
    let bak = PathBuf::from(bak);
    if path.exists() && !bak.exists() {
        std::fs::copy(path, &bak)?;
    }
    let tmp = path.with_extension("den-tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    // Keep the file's own permissions (a private rc file stays private).
    if let Ok(m) = std::fs::metadata(path) {
        f.set_permissions(m.permissions())?;
    }
    std::io::Write::write_all(&mut f, new.as_bytes())?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

/// `den alias [remove]`. Returns the process exit code.
pub fn run(args: &[String]) -> i32 {
    let remove = matches!(args.first().map(String::as_str), Some("remove" | "off" | "undo" | "--remove"));
    let mut failed = false;
    let mut changed = Vec::new();
    for (path, snippet) in targets() {
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        let fish_file = path.file_name().is_some_and(|n| n == "den-claude.fish");
        let new = match (remove, fish_file) {
            (true, true) => {
                if path.exists() && std::fs::remove_file(&path).is_ok() {
                    changed.push(path.clone());
                }
                continue;
            }
            (true, false) => without_block(&old),
            (false, _) => with_block(&old, snippet),
        };
        if new == old || (remove && !old.contains(START)) {
            continue;
        }
        match rewrite(&path, &new) {
            Ok(()) => changed.push(path),
            Err(e) => {
                eprintln!("couldn't update {}: {e}", path.display());
                failed = true;
            }
        }
    }
    for p in &changed {
        println!("{} {}", if remove { "removed from" } else { "added to" }, p.display());
    }
    if cfg!(windows) && !remove && profiles_blocked() {
        println!(
            "\nPowerShell is set to skip profile scripts here, so `claude` won't open den yet. Allow your own scripts with:\n  Set-ExecutionPolicy -Scope CurrentUser RemoteSigned"
        );
    }
    if remove {
        println!("`claude` runs Claude Code itself again (open a new terminal).");
    } else if !changed.is_empty() || !failed {
        println!(
            "\nOpen a new terminal, then:\n  claude              opens den in this folder\n  claude -c           den continue: the folder's last conversation\n  claude <anything>   Claude Code itself, as before\n\n\
             Skip den once with `command claude`; undo with `den alias remove`."
        );
    }
    i32::from(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_is_added_once_and_removed_cleanly() {
        let rc = "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'";
        let once = with_block(rc, POSIX);
        let twice = with_block(&once, POSIX);
        assert_eq!(once, twice);
        assert_eq!(once.matches(START).count(), 1);
        assert!(once.starts_with("export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n# >>> den"));
        assert_eq!(without_block(&once), "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n");
        assert_eq!(without_block("a\nb\n"), "a\nb\n");
    }

    #[cfg(unix)]
    #[test]
    fn posix_function_routes_only_plain_and_continue() {
        // Run the function in sh with stub `den`/`claude` on PATH and a tty check forced true.
        let dir = std::env::temp_dir().join(format!("den-alias-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for name in ["den", "claude"] {
            let p = dir.join(name);
            std::fs::write(&p, format!("#!/bin/sh\necho {name} \"$@\"\n")).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let func = POSIX.replace("[ -t 0 ] && [ -t 1 ]", "true");
        let run = |args: &str| {
            let script = format!("{func}\ncd /\nclaude {args}");
            let out = std::process::Command::new("sh")
                .arg("-c")
                .arg(script)
                .env("PATH", format!("{}:/usr/bin:/bin", dir.display()))
                .env_remove("DEN_NO_ALIAS")
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(run(""), "den /");
        assert_eq!(run("-c"), "den continue");
        assert_eq!(run("--continue"), "den continue");
        assert_eq!(run("-p hi"), "claude -p hi");
        assert_eq!(run("mcp list"), "claude mcp list");
        assert_eq!(run("-c -p hi"), "claude -c -p hi");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
