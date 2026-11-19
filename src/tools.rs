use serde_json::Value;
use std::path::Path;

pub fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(Value::as_str).unwrap_or("")
}

pub fn short_path(p: &str, cwd: &Path) -> String {
    let path = Path::new(p);
    if let Ok(rel) = path.strip_prefix(cwd)
        && !rel.as_os_str().is_empty()
    {
        return rel.display().to_string();
    }
    tilde(p)
}

pub fn tilde(p: &str) -> String {
    let home = crate::platform::home().to_string_lossy().into_owned();
    if home != "."
        && let Some(rest) = p.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    p.to_string()
}

/// Verb and target shown in a collapsed tool row.
pub fn summary(name: &str, input: &Value, cwd: &Path) -> (String, String) {
    let path = |k: &str| short_path(s(input, k), cwd);
    let first_line = |t: &str| t.lines().next().unwrap_or("").to_string();
    match name {
        "Read" => ("read".into(), path("file_path")),
        "Write" => ("write".into(), path("file_path")),
        "Edit" | "MultiEdit" => ("edit".into(), path("file_path")),
        "NotebookEdit" => ("notebook".into(), path("notebook_path")),
        "Bash" => ("bash".into(), first_line(s(input, "command"))),
        "BashOutput" => ("output".into(), s(input, "bash_id").into()),
        "Grep" => {
            let mut t = s(input, "pattern").to_string();
            let p = s(input, "path");
            if !p.is_empty() {
                t = format!("{t}  in {}", short_path(p, cwd));
            }
            ("grep".into(), t)
        }
        "Glob" => ("glob".into(), s(input, "pattern").into()),
        "WebFetch" => ("fetch".into(), s(input, "url").into()),
        "WebSearch" => ("search".into(), s(input, "query").into()),
        "Task" | "Agent" => ("agent".into(), s(input, "description").into()),
        "TodoWrite" => {
            let n = input.get("todos").and_then(Value::as_array).map_or(0, Vec::len);
            ("todos".into(), format!("{n} items"))
        }
        "Skill" => ("skill".into(), s(input, "skill").into()),
        _ if name.starts_with("mcp__") => {
            let mut parts = name.splitn(3, "__").skip(1);
            let server = parts.next().unwrap_or("mcp");
            let tool = parts.next().unwrap_or("");
            (server.replace("claude_ai_", "").to_lowercase(), tool.into())
        }
        _ => {
            let target = input
                .as_object()
                .and_then(|o| o.values().find_map(Value::as_str))
                .map(first_line)
                .unwrap_or_default();
            (name.to_lowercase(), target)
        }
    }
}

pub fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|p| match p.get("type").and_then(Value::as_str) {
                Some("text") => s(p, "text").to_string(),
                Some("image") => "[image]".into(),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffKind {
    Same,
    Add,
    Del,
}

/// Line diff via LCS; falls back to delete-all/add-all for huge inputs.
pub fn diff_lines<'a>(old: &'a str, new: &'a str) -> Vec<(DiffKind, &'a str)> {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len() * b.len() > 400_000 {
        let mut out: Vec<_> = a.iter().map(|l| (DiffKind::Del, *l)).collect();
        out.extend(b.iter().map(|l| (DiffKind::Add, *l)));
        return out;
    }
    let (n, m) = (a.len(), b.len());
    let mut dp = vec![0u32; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if a[i] == b[j] {
                dp[at(i + 1, j + 1)] + 1
            } else {
                dp[at(i + 1, j)].max(dp[at(i, j + 1)])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::with_capacity(n + m);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((DiffKind::Same, a[i]));
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            out.push((DiffKind::Del, a[i]));
            i += 1;
        } else {
            out.push((DiffKind::Add, b[j]));
            j += 1;
        }
    }
    out.extend(a[i..].iter().map(|l| (DiffKind::Del, *l)));
    out.extend(b[j..].iter().map(|l| (DiffKind::Add, *l)));
    out
}

pub fn edit_pairs(name: &str, input: &Value) -> Vec<(String, String)> {
    match name {
        "Edit" => vec![(s(input, "old_string").into(), s(input, "new_string").into())],
        "MultiEdit" => input
            .get("edits")
            .and_then(Value::as_array)
            .map(|es| {
                es.iter()
                    .map(|e| (s(e, "old_string").into(), s(e, "new_string").into()))
                    .collect()
            })
            .unwrap_or_default(),
        "Write" => vec![(String::new(), s(input, "content").into())],
        _ => Vec::new(),
    }
}

pub fn diff_stats(name: &str, input: &Value) -> Option<(usize, usize)> {
    let pairs = edit_pairs(name, input);
    if pairs.is_empty() {
        return None;
    }
    let (mut add, mut del) = (0, 0);
    for (o, n) in &pairs {
        for (k, _) in diff_lines(o, n) {
            match k {
                DiffKind::Add => add += 1,
                DiffKind::Del => del += 1,
                DiffKind::Same => {}
            }
        }
    }
    Some((add, del))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_minimal() {
        let d = diff_lines("a\nb\nc", "a\nx\nc");
        assert_eq!(
            d,
            vec![(DiffKind::Same, "a"), (DiffKind::Del, "b"), (DiffKind::Add, "x"), (DiffKind::Same, "c")]
        );
    }

    #[test]
    fn stats_for_edit() {
        let i = json!({"old_string": "a\nb", "new_string": "a\nb\nc\nd"});
        assert_eq!(diff_stats("Edit", &i), Some((2, 0)));
    }

    #[test]
    fn summary_relativizes() {
        let (v, t) = summary("Read", &json!({"file_path": "/p/src/x.rs"}), Path::new("/p"));
        assert_eq!((v.as_str(), t.as_str()), ("read", "src/x.rs"));
    }

    #[test]
    fn summary_mcp() {
        let (v, t) = summary("mcp__claude_ai_Gmail__send_message", &json!({}), Path::new("/"));
        assert_eq!((v.as_str(), t.as_str()), ("gmail", "send_message"));
    }

    #[test]
    fn result_text_array() {
        let c = json!([{"type": "text", "text": "hi"}, {"type": "image"}]);
        assert_eq!(result_text(&c), "hi\n[image]");
    }
}
