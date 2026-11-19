//! Desktop notifications with den's own icon. macOS shows the sending app's
//! icon, so den keeps a tiny AppleScript applet (`den.app`) carrying the den
//! icon and asks it to post each notification. Falls back to plain osascript.

use std::path::PathBuf;
use std::process::{Command, Stdio};

const ICON: &[u8] = include_bytes!("../assets/brand/den.icns");
const SCRIPT: &str = r#"on run
	set f to (POSIX path of (path to home folder)) & "Library/Caches/den/notify.txt"
	set t to paragraphs of (read (POSIX file f) as «class utf8»)
	display notification (item 3 of t) with title (item 1 of t) subtitle (item 2 of t)
end run"#;

fn home() -> PathBuf {
    crate::platform::home()
}

fn app_path() -> PathBuf {
    home().join("Library/Application Support/den/den.app")
}

fn message_file() -> PathBuf {
    home().join("Library/Caches/den/notify.txt")
}

fn quiet(cmd: &mut Command) -> bool {
    cmd.stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

/// Builds den.app once (and again if the icon changes).
fn ensure_app() -> Option<PathBuf> {
    let app = app_path();
    let icon = app.join("Contents/Resources/applet.icns");
    let fresh = !app.join("Contents/Resources/Assets.car").exists();
    if fresh && std::fs::metadata(&icon).is_ok_and(|m| m.len() == ICON.len() as u64) {
        return Some(app);
    }
    let _ = std::fs::remove_dir_all(&app);
    std::fs::create_dir_all(app.parent()?).ok()?;
    if !quiet(Command::new("osacompile").arg("-o").arg(&app).args(["-e", SCRIPT])) {
        return None;
    }
    std::fs::write(&icon, ICON).ok()?;
    // Newer osacompile adds an asset catalog whose default icon wins over ours.
    let _ = std::fs::remove_file(app.join("Contents/Resources/Assets.car"));
    let plist = app.join("Contents/Info.plist");
    let set = |key: &str, ty: &str, val: &str| {
        let _ = quiet(Command::new("/usr/libexec/PlistBuddy").args(["-c", &format!("Delete :{key}")]).arg(&plist));
        let _ = quiet(Command::new("/usr/libexec/PlistBuddy").args(["-c", &format!("Add :{key} {ty} {val}")]).arg(&plist));
    };
    set("CFBundleName", "string", "den");
    set("CFBundleIdentifier", "string", "dev.den.notifier");
    set("CFBundleIconFile", "string", "applet");
    set("LSUIElement", "bool", "true");
    let _ = quiet(Command::new("/usr/libexec/PlistBuddy").args(["-c", "Delete :CFBundleIconName"]).arg(&plist));
    // Editing the bundle invalidates its signature; re-sign ad hoc.
    let _ = quiet(Command::new("codesign").args(["--force", "--deep", "--sign", "-"]).arg(&app));
    Some(app)
}

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Posts "den — `title`: `body`". Runs in the background; never blocks.
pub fn send(title: &str, body: &str) {
    if cfg!(windows) {
        let (title, body) = (one_line(title), one_line(body));
        std::thread::spawn(move || toast(&title, &body));
        return;
    }
    if !cfg!(target_os = "macos") {
        return;
    }
    let (title, body) = (one_line(title), one_line(body));
    std::thread::spawn(move || {
        if let Some(app) = ensure_app() {
            let file = message_file();
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if std::fs::write(&file, format!("den\n{title}\n{body}\n")).is_ok()
                && quiet(Command::new("open").arg("-g").arg(&app))
            {
                return;
            }
        }
        let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
        let script = format!("display notification \"{}\" with title \"den\" subtitle \"{}\"", esc(&body), esc(&title));
        let _ = quiet(Command::new("osascript").args(["-e", &script]));
    });
}

/// A Windows toast through PowerShell's WinRT bridge. Text goes in through
/// environment variables, so nothing in it is ever run as script.
fn toast(title: &str, body: &str) {
    const PS: &str = r#"
[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
[Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
$x = New-Object Windows.Data.Xml.Dom.XmlDocument
$x.LoadXml('<toast><visual><binding template="ToastGeneric"><text/><text/></binding></visual></toast>')
$t = $x.GetElementsByTagName('text')
$t.Item(0).AppendChild($x.CreateTextNode('den - ' + $env:DEN_TITLE)) | Out-Null
$t.Item(1).AppendChild($x.CreateTextNode($env:DEN_BODY)) | Out-Null
$app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
[Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($x))
"#;
    let _ = quiet(
        Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-WindowStyle", "Hidden", "-Command", PS])
            .env("DEN_TITLE", title)
            .env("DEN_BODY", body),
    );
}
