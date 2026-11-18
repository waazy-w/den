use crate::files;
use crate::tools;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const MAX_RECENT: usize = 100;

fn is_scratch(p: &Path) -> bool {
    let s = p.to_string_lossy();
    ["/tmp/", "/private/tmp/", "/var/folders/", "/private/var/folders/"]
        .iter()
        .any(|pre| s.starts_with(pre))
}

/// Recent project folders from Claude Code's history, newest first.
pub fn discover() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME") else { return vec![] };
    let root = PathBuf::from(home).join(".claude").join("projects");
    let Ok(dirs) = std::fs::read_dir(root) else { return vec![] };
    let mut found: Vec<(SystemTime, PathBuf)> = Vec::new();
    for d in dirs.flatten() {
        let Ok(files) = std::fs::read_dir(d.path()) else { continue };
        let mut transcripts: Vec<(SystemTime, PathBuf)> = files
            .flatten()
            .filter(|f| f.path().extension().is_some_and(|e| e == "jsonl"))
            .filter_map(|f| Some((f.metadata().ok()?.modified().ok()?, f.path())))
            .collect();
        transcripts.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
        let Some(&(mtime, _)) = transcripts.first() else { continue };
        let cwd = transcripts
            .iter()
            .take(3)
            .find_map(|(_, f)| cwd_from_transcript(f).filter(|c| c.is_dir()))
            .or_else(|| resolve_slug(&d.file_name().to_string_lossy()));
        if let Some(cwd) = cwd
            && !is_scratch(&cwd)
        {
            found.push((mtime, cwd));
        }
    }
    found.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    let mut out: Vec<PathBuf> = Vec::new();
    for (_, p) in found {
        if !out.contains(&p) {
            out.push(p);
        }
        if out.len() >= MAX_RECENT {
            break;
        }
    }
    out
}

fn cwd_from_transcript(file: &Path) -> Option<PathBuf> {
    use std::io::{BufRead, BufReader};
    let f = std::fs::File::open(file).ok()?;
    for line in BufReader::new(f).lines().take(400).map_while(Result::ok) {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line)
            && let Some(c) = v.get("cwd").and_then(|c| c.as_str())
        {
            return Some(PathBuf::from(c));
        }
    }
    None
}

fn base(p: &Path) -> String {
    p.file_name().map_or_else(|| tools::tilde(&p.to_string_lossy()), |n| n.to_string_lossy().into_owned())
}

/// Folder name, or parent/name when another project shares it.
pub fn label(p: &Path, all: &[&Path]) -> String {
    let b = base(p);
    let clash = all.iter().any(|o| *o != p && base(o) == b);
    match (clash, p.parent().and_then(|pp| pp.file_name())) {
        (true, Some(parent)) => format!("{}/{b}", parent.to_string_lossy()),
        _ => b,
    }
}

/// Claude Code names project dirs by replacing every non-alphanumeric char of
/// the path with '-'. Walk the real filesystem to find the folder that matches.
pub fn resolve_slug(slug: &str) -> Option<PathBuf> {
    fn norm(name: &str) -> String {
        name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
    }
    fn walk(dir: &Path, rest: &str, depth: usize) -> Option<PathBuf> {
        if rest.is_empty() {
            return Some(dir.to_path_buf());
        }
        if depth > 12 {
            return None;
        }
        let rest = rest.strip_prefix('-')?;
        let mut entries: Vec<(String, PathBuf)> = std::fs::read_dir(dir)
            .ok()?
            .flatten()
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .map(|e| (norm(&e.file_name().to_string_lossy()), e.path()))
            .filter(|(n, _)| rest.starts_with(n.as_str()) && (rest.len() == n.len() || rest[n.len()..].starts_with('-')))
            .collect();
        entries.sort_by_key(|(n, _)| std::cmp::Reverse(n.len()));
        entries.into_iter().find_map(|(n, p)| walk(&p, &rest[n.len()..], depth + 1))
    }
    walk(Path::new("/"), slug, 0)
}

/// Normalizes a typed or pasted path: quotes, `\ ` escapes, `file://` URLs.
pub fn clean_path(p: &str) -> String {
    let mut t = p.trim().trim_matches(|c| c == '\'' || c == '"').trim().to_string();
    if let Some(rest) = t.strip_prefix("file://") {
        let mut out = Vec::new();
        let b = rest.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'%'
                && i + 2 < b.len()
                && let Ok(v) = u8::from_str_radix(&rest[i + 1..i + 3], 16)
            {
                out.push(v);
                i += 3;
                continue;
            }
            out.push(b[i]);
            i += 1;
        }
        t = String::from_utf8_lossy(&out).into_owned();
    }
    let mut out = String::with_capacity(t.len());
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&n) = chars.peek()
            && !n.is_alphanumeric()
        {
            out.push(n);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

pub fn expand(p: &str) -> PathBuf {
    let cleaned = clean_path(p);
    let p = cleaned.as_str();
    if let Some(rest) = p.strip_prefix('~')
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(rest.trim_start_matches('/'));
    }
    PathBuf::from(p)
}

pub fn looks_like_path(q: &str) -> bool {
    let q = clean_path(q);
    q.starts_with('/') || q.starts_with('~') || q.starts_with('.')
}

#[derive(Clone, Debug, PartialEq)]
pub enum Pick {
    Project(PathBuf),
    Open(PathBuf),
    Create(PathBuf),
}

/// Sub-folders of a workspace, most recently changed first.
pub fn children(ws: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(ws) else { return vec![] };
    let mut v: Vec<(SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH), e.path()))
        .collect();
    v.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    v.into_iter().map(|(_, p)| p).collect()
}

/// Like `picker`, plus "create <workspace>/<query>" when nothing matches exactly.
pub fn picker_with(query: &str, projects: &[PathBuf], workspaces: &[PathBuf]) -> Vec<Pick> {
    let mut out = picker(query, projects);
    let q = query.trim();
    if !q.is_empty() && !looks_like_path(q) && !q.contains('/') {
        let exact = projects.iter().any(|p| p.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(q)));
        if !exact {
            for ws in workspaces {
                out.push(Pick::Create(ws.join(q)));
            }
        }
    }
    out
}

pub fn picker(query: &str, projects: &[PathBuf]) -> Vec<Pick> {
    let q = query.trim();
    if looks_like_path(q) {
        let mut out = vec![Pick::Open(expand(q))];
        let cleaned = clean_path(q);
        let ql = cleaned.trim_end_matches('/');
        out.extend(
            projects
                .iter()
                .filter(|p| tools::tilde(&p.to_string_lossy()).starts_with(ql) || p.to_string_lossy().starts_with(ql))
                .take(8)
                .cloned()
                .map(Pick::Project),
        );
        return out;
    }
    let refs: Vec<&Path> = projects.iter().map(PathBuf::as_path).collect();
    let mut scored: Vec<(i32, usize)> = projects
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let hay = format!("{} {}", label(p, &refs), tools::tilde(&p.to_string_lossy()));
            files::score(q, &hay).map(|s| (s, i))
        })
        .collect();
    if !q.is_empty() {
        scored.sort_by_key(|(s, _)| std::cmp::Reverse(*s));
    }
    scored.into_iter().take(12).map(|(_, i)| Pick::Project(projects[i].clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_disambiguate() {
        let a = PathBuf::from("/x/work/api");
        let b = PathBuf::from("/x/play/api");
        let c = PathBuf::from("/x/web");
        let all = [a.as_path(), b.as_path(), c.as_path()];
        assert_eq!(label(&a, &all), "work/api");
        assert_eq!(label(&c, &all), "web");
    }

    #[test]
    fn picker_paths_and_filter() {
        let ps = vec![PathBuf::from("/x/api"), PathBuf::from("/x/web")];
        assert_eq!(picker("web", &ps), vec![Pick::Project(PathBuf::from("/x/web"))]);
        assert_eq!(picker("/x/a", &ps)[0], Pick::Open(PathBuf::from("/x/a")));
        assert_eq!(picker("", &ps).len(), 2);
    }

    #[test]
    fn cleans_pasted_paths() {
        assert_eq!(clean_path("  '/Users/me/My Folder'  "), "/Users/me/My Folder");
        assert_eq!(clean_path("/Users/me/Claude\\ Projects/A\\(1\\)"), "/Users/me/Claude Projects/A(1)");
        assert_eq!(clean_path("file:///Users/me/My%20Folder/"), "/Users/me/My Folder/");
        assert!(looks_like_path("'/tmp'"));
    }

    #[test]
    fn resolves_slug_against_filesystem() {
        let base = std::env::temp_dir().join("ctui slug test").join("my.app");
        std::fs::create_dir_all(&base).unwrap();
        let base = std::fs::canonicalize(&base).unwrap();
        let slug: String = base.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
        assert_eq!(resolve_slug(&slug), Some(base));
    }

    #[test]
    fn scratch_dirs_skipped() {
        assert!(is_scratch(Path::new("/private/tmp/foo")));
        assert!(!is_scratch(Path::new("/Users/me/code")));
    }
}

