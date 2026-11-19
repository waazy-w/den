//! Self-update from GitHub Releases, through the GitHub CLI's login (the repo
//! is private), a GH_TOKEN, or plain `curl` if the repo is public. den needs
//! no TLS stack of its own. The new binary replaces the running one on disk;
//! `/reload` (or the next launch) switches to it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

pub const REPO: &str = match option_env!("DEN_REPO") {
    Some(r) => r,
    None => "waazy-w/den",
};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the background check runs.
const EVERY: Duration = Duration::from_secs(6 * 3600);

#[derive(Debug, PartialEq)]
pub enum Outcome {
    UpToDate,
    Installed(String),
    Failed(String),
}

/// Release asset for this machine, e.g. `den-aarch64-macos`.
pub fn asset_name() -> String {
    format!("den-{}-{}{}", std::env::consts::ARCH, std::env::consts::OS, std::env::consts::EXE_SUFFIX)
}

/// Where a replaced den.exe goes: Windows can't overwrite a running program,
/// but it can rename it out of the way.
fn retired(exe: &Path) -> PathBuf {
    exe.with_file_name("den.old.exe")
}

/// Deletes the den.exe a Windows update left behind once it's no longer running.
pub fn clean_retired() {
    if cfg!(windows)
        && let Ok(exe) = std::env::current_exe()
    {
        let _ = std::fs::remove_file(retired(&exe));
    }
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn newer(remote: &str, local: &str) -> bool {
    matches!((parse_version(remote), parse_version(local)), (Some(r), Some(l)) if r > l)
}

fn stamp_file() -> PathBuf {
    crate::store::config_dir().join("update-check")
}

/// True when the background check is due (and allowed).
pub fn due() -> bool {
    if std::env::var_os("DEN_NO_UPDATE").is_some() || crate::store::load_config().auto_update == Some(false) {
        return false;
    }
    // Development builds update through cargo, not here.
    if std::env::current_exe().is_ok_and(|e| e.components().any(|c| c.as_os_str() == "target")) {
        return false;
    }
    std::fs::metadata(stamp_file())
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .is_none_or(|age| age >= EVERY)
}

fn run_cmd(cmd: &mut Command) -> Result<Vec<u8>, String> {
    let out = cmd.stdin(Stdio::null()).output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(out.stdout)
}

fn curl(args: &[&str]) -> Result<Vec<u8>, String> {
    run_cmd(Command::new("curl").args(["-fsSL", "--max-time", "120", "-H", "User-Agent: den-updater"]).args(args))
        .map_err(|e| if e.is_empty() { "download failed".into() } else { e })
}

/// curl with the auth header fed through stdin (`-H @-`), so the token never
/// shows up in the process list.
fn curl_auth(token: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    use std::io::Write;
    let mut child = Command::new("curl")
        .args(["-fsSL", "--max-time", "120", "-H", "User-Agent: den-updater", "-H", "@-"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "Authorization: Bearer {token}");
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        let e = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if e.is_empty() { "download failed".into() } else { e });
    }
    Ok(out.stdout)
}

/// How to reach the (possibly private) repo: the GitHub CLI's login, a
/// token in the environment, or plain HTTPS for a public repo.
enum Source {
    Gh,
    Token(String),
    Public,
}

fn source() -> Source {
    if Command::new("gh").args(["auth", "status"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success()) {
        return Source::Gh;
    }
    match std::env::var("GH_TOKEN").or_else(|_| std::env::var("GITHUB_TOKEN")) {
        Ok(t) if !t.is_empty() => Source::Token(t),
        _ => Source::Public,
    }
}

/// The latest release's tag, and a handle for this machine's asset.
fn latest(src: &Source) -> Result<(String, Option<String>), String> {
    let want = asset_name();
    let v: serde_json::Value = match src {
        Source::Gh => {
            let out = run_cmd(Command::new("gh").args(["release", "view", "-R", REPO, "--json", "tagName,assets"]))?;
            serde_json::from_slice(&out).map_err(|e| e.to_string())?
        }
        Source::Token(t) => serde_json::from_slice(&curl_auth(t, &[
            "-H",
            "Accept: application/vnd.github+json",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])?)
        .map_err(|e| e.to_string())?,
        Source::Public => serde_json::from_slice(&curl(&[
            "-H",
            "Accept: application/vnd.github+json",
            &format!("https://api.github.com/repos/{REPO}/releases/latest"),
        ])
        .map_err(|e| format!("{e} (private repo? run `gh auth login`)"))?)
        .map_err(|e| e.to_string())?,
    };
    let tag = v
        .get("tagName")
        .or_else(|| v.get("tag_name"))
        .and_then(|t| t.as_str())
        .ok_or("no release found")?
        .to_string();
    let asset = v
        .get("assets")
        .and_then(|a| a.as_array())
        .into_iter()
        .flatten()
        .find(|a| a.get("name").and_then(|n| n.as_str()) == Some(want.as_str()))
        .map(|a| match src {
            Source::Gh => want.clone(),
            Source::Token(_) => a.get("url").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
            Source::Public => a.get("browser_download_url").and_then(|u| u.as_str()).unwrap_or_default().to_string(),
        });
    Ok((tag, asset))
}

fn download(src: &Source, tag: &str, asset: &str, dest: &Path) -> Result<(), String> {
    let d = dest.to_string_lossy();
    match src {
        Source::Gh => run_cmd(Command::new("gh").args(["release", "download", tag, "-R", REPO, "-p", asset, "-O", &d, "--clobber"]))
            .map(|_| ()),
        Source::Token(t) => curl_auth(t, &[
            "-H",
            "Accept: application/octet-stream",
            "-o",
            &d,
            asset,
        ])
        .map(|_| ()),
        Source::Public => curl(&["-o", &d, asset]).map(|_| ()),
    }
}

/// Downloads the release next to `exe`, checks it runs, then swaps it in.
fn install(src: &Source, tag: &str, asset: &str, exe: &Path) -> Result<(), String> {
    let tmp = exe.with_file_name(format!(".den-update-{}{}", std::process::id(), std::env::consts::EXE_SUFFIX));
    let result = (|| {
        download(src, tag, asset, &tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
        }
        let out = Command::new(&tmp).arg("--version").output().map_err(|e| format!("new build won't start: {e}"))?;
        let says = String::from_utf8_lossy(&out.stdout);
        if !out.status.success() || !says.contains(tag.trim_start_matches('v')) {
            return Err(format!("new build reported {:?}", says.trim()));
        }
        if cfg!(windows) {
            let old = retired(exe);
            let _ = std::fs::remove_file(&old);
            std::fs::rename(exe, &old).map_err(|e| format!("couldn't move {} aside: {e}", exe.display()))?;
            return std::fs::rename(&tmp, exe).map_err(|e| {
                let _ = std::fs::rename(&old, exe);
                format!("couldn't replace {}: {e}", exe.display())
            });
        }
        std::fs::rename(&tmp, exe).map_err(|e| format!("couldn't replace {}: {e}", exe.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Checks GitHub and installs a newer release if there is one.
pub fn run() -> Outcome {
    let _ = std::fs::create_dir_all(crate::store::config_dir());
    let _ = std::fs::write(stamp_file(), VERSION);
    let src = source();
    let (tag, asset) = match latest(&src) {
        Ok(x) => x,
        Err(e) => return Outcome::Failed(e),
    };
    if !newer(&tag, VERSION) {
        return Outcome::UpToDate;
    }
    let Some(asset) = asset.filter(|a| !a.is_empty()) else {
        return Outcome::Failed(format!("{tag} has no build for {}", asset_name()));
    };
    let exe = match std::env::current_exe().and_then(std::fs::canonicalize) {
        Ok(e) => e,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    match install(&src, &tag, &asset, &exe) {
        Ok(()) => Outcome::Installed(tag),
        Err(e) => Outcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_versions() {
        assert!(newer("v0.2.0", "0.1.9"));
        assert!(newer("v1.0.0", "0.9.9"));
        assert!(!newer("v0.1.0", "0.1.0"));
        assert!(!newer("v0.1.0-beta", "0.1.0"));
        assert!(!newer("garbage", "0.1.0"));
        assert!(asset_name().starts_with("den-"));
    }
}
