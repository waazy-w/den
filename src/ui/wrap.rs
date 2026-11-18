use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Word-wraps styled spans to `max` columns. `first`/`rest` are prefixes
/// (gutters, indents) prepended to the first and continuation lines.
pub fn wrap(
    spans: &[Span<'static>],
    max: usize,
    first: &[Span<'static>],
    rest: &[Span<'static>],
) -> Vec<Line<'static>> {
    let pw_first: usize = first.iter().map(|s| width(&s.content)).sum();
    let pw_rest: usize = rest.iter().map(|s| width(&s.content)).sum();
    let mut out = Vec::new();
    let mut cur: Vec<Span<'static>> = first.to_vec();
    let mut col = 0usize;
    let fresh = max.saturating_sub(pw_rest).max(1);
    let mut avail = max.saturating_sub(pw_first).max(1);
    let mut line_has_content = false;

    let push_text = |cur: &mut Vec<Span<'static>>, text: &str, style: Style| {
        if let Some(last) = cur.last_mut()
            && last.style == style
        {
            let mut s = last.content.to_string();
            s.push_str(text);
            last.content = s.into();
            return;
        }
        cur.push(Span::styled(text.to_string(), style));
    };

    for span in spans {
        let style = span.style;
        for token in split_words(&span.content) {
            let is_space = token.chars().all(char::is_whitespace);
            let tw = width(token);
            if col + tw <= avail {
                if !(is_space && col == 0 && line_has_content) {
                    push_text(&mut cur, token, style);
                    col += tw;
                }
                line_has_content = true;
                continue;
            }
            if is_space {
                out.push(Line::from(std::mem::take(&mut cur)));
                cur = rest.to_vec();
                col = 0;
                avail = fresh;
                continue;
            }
            if col > 0 && tw <= fresh {
                out.push(Line::from(std::mem::take(&mut cur)));
                cur = rest.to_vec();
                col = 0;
                avail = fresh;
            }
            if col + tw <= avail {
                push_text(&mut cur, token, style);
                col += tw;
                line_has_content = true;
                continue;
            }
            let mut buf = String::new();
            for g in token.graphemes(true) {
                let gw = width(g);
                if col + gw > avail && col > 0 {
                    if !buf.is_empty() {
                        push_text(&mut cur, &buf, style);
                        buf.clear();
                    }
                    out.push(Line::from(std::mem::take(&mut cur)));
                    cur = rest.to_vec();
                    col = 0;
                    avail = fresh;
                }
                buf.push_str(g);
                col += gw;
            }
            if !buf.is_empty() {
                push_text(&mut cur, &buf, style);
            }
            line_has_content = true;
        }
    }
    out.push(Line::from(cur));
    out
}

fn split_words(s: &str) -> impl Iterator<Item = &str> {
    let mut idx = 0;
    let bytes = s.as_bytes();
    std::iter::from_fn(move || {
        if idx >= s.len() {
            return None;
        }
        let start = idx;
        let space = (bytes[idx] as char).is_ascii_whitespace();
        while idx < s.len() {
            let c = s[idx..].chars().next().unwrap();
            if c.is_whitespace() != space {
                break;
            }
            idx += c.len_utf8();
        }
        Some(&s[start..idx])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn wraps_on_words() {
        let l = wrap(&[Span::raw("hello brave new world")], 11, &[], &[]);
        assert_eq!(plain(&l), vec!["hello brave", "new world"]);
    }

    #[test]
    fn prefixes_apply() {
        let l = wrap(&[Span::raw("aaa bbb ccc")], 9, &[Span::raw("> ")], &[Span::raw("  ")]);
        assert_eq!(plain(&l), vec!["> aaa bbb", "  ccc"]);
    }

    #[test]
    fn breaks_long_words() {
        let l = wrap(&[Span::raw("abcdefghij")], 4, &[], &[]);
        assert_eq!(plain(&l), vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn empty_input_yields_one_line() {
        let l = wrap(&[], 10, &[Span::raw("x")], &[]);
        assert_eq!(plain(&l), vec!["x"]);
    }

    #[test]
    fn wide_chars_counted() {
        let l = wrap(&[Span::raw("日本語テキスト")], 6, &[], &[]);
        assert_eq!(plain(&l), vec!["日本語", "テキス", "ト"]);
    }
}
