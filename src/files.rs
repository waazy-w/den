use std::path::Path;

const MAX_FILES: usize = 50_000;

pub fn index(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(true)
        .git_ignore(true)
        .max_depth(Some(12))
        .build();
    for entry in walker.flatten() {
        if out.len() >= MAX_FILES {
            break;
        }
        if entry.file_type().is_some_and(|t| t.is_file())
            && let Ok(rel) = entry.path().strip_prefix(root)
        {
            out.push(rel.to_string_lossy().into_owned());
        }
    }
    out.sort_unstable_by_key(|p| (p.matches('/').count(), p.len()));
    out
}

/// Subsequence score: higher is better; None when not a match.
pub fn score(query: &str, cand: &str) -> Option<i32> {
    if query.is_empty() {
        return Some(0);
    }
    let base_start = cand.rfind('/').map_or(0, |i| i + 1);
    let mut score = 0i32;
    let mut qi = query.chars().map(|c| c.to_ascii_lowercase()).peekable();
    let mut prev_match = false;
    let mut prev_char = '/';
    for (i, c) in cand.char_indices() {
        let Some(&q) = qi.peek() else { break };
        if c.to_ascii_lowercase() == q {
            qi.next();
            score += 1;
            if prev_match {
                score += 4;
            }
            if matches!(prev_char, '/' | '_' | '-' | '.' | ' ') {
                score += 6;
            }
            if i >= base_start {
                score += 2;
            }
            prev_match = true;
        } else {
            prev_match = false;
        }
        prev_char = c;
    }
    if qi.peek().is_some() {
        return None;
    }
    Some(score * 4 - cand.len() as i32 / 4)
}

pub fn top<'a, I: IntoIterator<Item = &'a str>>(query: &str, cands: I, n: usize) -> Vec<&'a str> {
    let mut scored: Vec<(i32, &str)> = cands
        .into_iter()
        .filter_map(|c| score(query, c).map(|s| (s, c)))
        .collect();
    scored.sort_by_key(|x| std::cmp::Reverse(x.0));
    scored.into_iter().take(n).map(|(_, c)| c).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_basename_and_contiguous() {
        let c = ["docs/main_notes.md", "src/main.rs", "src/ui/mod.rs"];
        assert_eq!(top("main", c, 1), vec!["src/main.rs"]);
        assert_eq!(top("umod", c, 1), vec!["src/ui/mod.rs"]);
        assert!(score("zzz", "src/main.rs").is_none());
    }
}
