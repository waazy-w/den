//! The few things that differ between macOS/Linux and Windows.

use std::path::PathBuf;

/// The user's home folder: $HOME, or %USERPROFILE% on Windows.
pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Local calendar time. `month` is 1-12, `weekday` 0 = Sunday.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Local {
    pub year: i32,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
}

pub fn now_epoch() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

#[cfg(unix)]
pub fn local(epoch: u64) -> Local {
    let t = epoch as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon as u32 + 1,
        day: tm.tm_mday as u32,
        hour: tm.tm_hour as u32,
        minute: tm.tm_min as u32,
    }
}

#[cfg(windows)]
pub fn local(epoch: u64) -> Local {
    use windows_sys::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows_sys::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let ticks = (epoch + 11_644_473_600) * 10_000_000;
    let ft = FILETIME { dwLowDateTime: ticks as u32, dwHighDateTime: (ticks >> 32) as u32 };
    let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    let mut loc: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc) == 0 || SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut loc) == 0 {
            loc = utc;
        }
    }
    Local {
        year: loc.wYear as i32,
        month: loc.wMonth as u32,
        day: loc.wDay as u32,
        hour: loc.wHour as u32,
        minute: loc.wMinute as u32,
    }
}

/// Whether a process with this pid is still running.
#[cfg(unix)]
pub fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Asks a process to stop.
#[cfg(unix)]
pub fn terminate(pid: u32) {
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

#[cfg(windows)]
pub fn terminate(pid: u32) {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
    unsafe {
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// A program on PATH, trying Windows' runnable extensions (`claude.exe`,
/// `claude.cmd`, …). On macOS/Linux the name is returned as is.
pub fn find_program(name: &str) -> PathBuf {
    #[cfg(windows)]
    {
        let path = std::path::Path::new(name);
        if path.extension().is_none() {
            let exts = ["exe", "cmd", "bat", "com"];
            for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
                for ext in exts {
                    let p = dir.join(format!("{name}.{ext}"));
                    if p.is_file() {
                        return p;
                    }
                }
            }
            // Claude Code's native installer puts it here.
            let p = home().join(".local").join("bin").join(format!("{name}.exe"));
            if p.is_file() {
                return p;
            }
        }
    }
    PathBuf::from(name)
}

/// The command that runs a `!shell` line: the user's $SHELL, or PowerShell.
pub fn shell_command(line: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", line]);
        c
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
        let mut c = std::process::Command::new(shell);
        c.arg("-c").arg(line);
        c
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_time_is_sane() {
        let l = super::local(super::now_epoch());
        assert!(l.year >= 2024 && (1..=12).contains(&l.month) && (1..=31).contains(&l.day) && l.hour < 24);
    }
}
