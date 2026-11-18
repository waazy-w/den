use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Clone, Debug)]
pub enum Attachment {
    Text(String),
    Image { path: std::path::PathBuf, media: &'static str },
}

/// Pastes longer than this collapse into a `[Pasted text #N]` tag.
const COLLAPSE_LINES: usize = 3;
const COLLAPSE_CHARS: usize = 600;

#[derive(Default)]
pub struct Editor {
    pub text: String,
    pub cursor: usize,
    history: Vec<String>,
    hist_pos: Option<usize>,
    draft: String,
    /// Tag → content, kept for the whole run so recalled history still expands.
    attachments: Vec<(String, Attachment)>,
    pastes: usize,
    images: usize,
}

pub fn image_media(path: &std::path::Path) -> Option<&'static str> {
    match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// A dragged-in file arrives as its path, sometimes quoted or with escaped spaces.
fn as_image_path(s: &str) -> Option<std::path::PathBuf> {
    let t = s.trim().trim_matches(|c| c == '\'' || c == '"');
    if t.contains('\n') || !(t.starts_with('/') || t.starts_with('~')) {
        return None;
    }
    let unescaped = t.replace("\\ ", " ");
    let p = crate::projects::expand(&unescaped);
    (p.is_file() && image_media(&p).is_some()).then_some(p)
}

/// Byte ranges of attachment tags like `[Image #1]` in `text`.
pub fn tag_ranges(text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(off) = text[i..].find('[') {
        let start = i + off;
        let rest = &text[start..];
        if (rest.starts_with("[Pasted text #") || rest.starts_with("[Image #"))
            && let Some(end) = rest.find(']')
        {
            out.push((start, start + end + 1));
            i = start + end + 1;
        } else {
            i = start + 1;
        }
    }
    out
}

impl Editor {
    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn set_history(&mut self, h: Vec<String>) {
        self.history = h;
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn set(&mut self, s: &str) {
        self.text = s.to_string();
        self.cursor = self.text.len();
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.hist_pos = None;
    }

    pub fn take(&mut self) -> String {
        let t = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.hist_pos = None;
        if !t.trim().is_empty() && self.history.last() != Some(&t) {
            self.history.push(t.clone());
        }
        t
    }

    /// Inserts pasted text, collapsing long pastes and image paths into tags.
    pub fn paste(&mut self, s: &str) {
        if let Some(path) = as_image_path(s) {
            self.attach_image(path);
            return;
        }
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        let lines = s.lines().count();
        if lines > COLLAPSE_LINES || s.len() > COLLAPSE_CHARS {
            self.pastes += 1;
            let tag = if lines > 1 {
                format!("[Pasted text #{} +{} lines]", self.pastes, lines)
            } else {
                format!("[Pasted text #{} {} chars]", self.pastes, s.chars().count())
            };
            self.attachments.push((tag.clone(), Attachment::Text(s)));
            self.insert(&tag);
        } else {
            self.insert(&s);
        }
    }

    pub fn attach_image(&mut self, path: std::path::PathBuf) {
        let Some(media) = image_media(&path) else { return };
        self.images += 1;
        let tag = format!("[Image #{}]", self.images);
        self.attachments.push((tag.clone(), Attachment::Image { path, media }));
        if !self.text[..self.cursor].ends_with([' ', '\n']) && self.cursor > 0 {
            self.insert(" ");
        }
        self.insert(&tag);
        self.insert(" ");
    }

    /// Attachments whose tags appear in `text`, in order of appearance.
    pub fn attachments_in(&self, text: &str) -> Vec<(String, Attachment)> {
        tag_ranges(text)
            .into_iter()
            .filter_map(|(s, e)| self.attachments.iter().find(|(t, _)| t == &text[s..e]).cloned())
            .collect()
    }

    pub fn insert(&mut self, s: &str) {
        let s = s.replace("\r\n", "\n").replace('\r', "\n");
        self.text.insert_str(self.cursor, &s);
        self.cursor += s.len();
    }

    fn prev_boundary(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }

    fn next_boundary(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }

    pub fn backspace(&mut self) {
        if let Some(&(s, _)) = tag_ranges(&self.text).iter().find(|(_, e)| *e == self.cursor) {
            self.text.replace_range(s..self.cursor, "");
            self.cursor = s;
            return;
        }
        let p = self.prev_boundary();
        self.text.replace_range(p..self.cursor, "");
        self.cursor = p;
    }

    pub fn delete(&mut self) {
        let n = self.next_boundary();
        self.text.replace_range(self.cursor..n, "");
    }

    pub fn left(&mut self) {
        self.cursor = self.prev_boundary();
    }

    pub fn right(&mut self) {
        self.cursor = self.next_boundary();
    }

    fn word_start(&self) -> usize {
        let before = &self.text[..self.cursor];
        let trimmed = before.trim_end();
        trimmed
            .rfind(|c: char| c.is_whitespace() || c == '/')
            .map_or(0, |i| i + trimmed[i..].chars().next().unwrap().len_utf8())
    }

    pub fn word_left(&mut self) {
        self.cursor = self.word_start();
    }

    pub fn word_right(&mut self) {
        let after = &self.text[self.cursor..];
        let skip_ws = after.len() - after.trim_start().len();
        let rest = &after[skip_ws..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '/')
            .unwrap_or(rest.len())
            .max(rest.chars().next().map_or(0, char::len_utf8));
        self.cursor += skip_ws + end.min(rest.len());
    }

    pub fn delete_word(&mut self) {
        let s = self.word_start();
        self.text.replace_range(s..self.cursor, "");
        self.cursor = s;
    }

    fn line_start(&self) -> usize {
        self.text[..self.cursor].rfind('\n').map_or(0, |i| i + 1)
    }

    fn line_end(&self) -> usize {
        self.text[self.cursor..].find('\n').map_or(self.text.len(), |i| self.cursor + i)
    }

    pub fn home(&mut self) {
        self.cursor = self.line_start();
    }

    pub fn end(&mut self) {
        self.cursor = self.line_end();
    }

    pub fn kill_line_back(&mut self) {
        let s = self.line_start();
        self.text.replace_range(s..self.cursor, "");
        self.cursor = s;
    }

    pub fn kill_line_fwd(&mut self) {
        let e = self.line_end();
        self.text.replace_range(self.cursor..e, "");
    }

    /// Moves within multi-line text; falls through to history at the edges.
    pub fn up(&mut self) {
        let ls = self.line_start();
        if ls == 0 {
            self.history_prev();
            return;
        }
        let col = self.text[ls..self.cursor].chars().count();
        let prev_start = self.text[..ls - 1].rfind('\n').map_or(0, |i| i + 1);
        self.cursor = advance(&self.text, prev_start, ls - 1, col);
    }

    pub fn down(&mut self) {
        let le = self.line_end();
        if le == self.text.len() {
            self.history_next();
            return;
        }
        let col = self.text[self.line_start()..self.cursor].chars().count();
        let next_start = le + 1;
        let next_end = self.text[next_start..].find('\n').map_or(self.text.len(), |i| next_start + i);
        self.cursor = advance(&self.text, next_start, next_end, col);
    }

    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.hist_pos {
            None => {
                self.draft = self.text.clone();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(p) => p - 1,
        };
        self.hist_pos = Some(pos);
        self.set(&self.history[pos].clone());
    }

    fn history_next(&mut self) {
        let Some(p) = self.hist_pos else { return };
        if p + 1 >= self.history.len() {
            self.hist_pos = None;
            let d = std::mem::take(&mut self.draft);
            self.set(&d);
        } else {
            self.hist_pos = Some(p + 1);
            self.set(&self.history[p + 1].clone());
        }
    }

    /// The `@token` or `/command` the cursor is in, with its byte start.
    pub fn token_at_cursor(&self) -> Option<(usize, &str)> {
        let before = &self.text[..self.cursor];
        let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        let tok = &before[start..];
        (tok.starts_with('@') || (tok.starts_with('/') && start == 0)).then_some((start, tok))
    }

    pub fn replace_token(&mut self, start: usize, with: &str) {
        let end = self.text[self.cursor..]
            .find(char::is_whitespace)
            .map_or(self.text.len(), |i| self.cursor + i);
        self.text.replace_range(start..end, with);
        self.cursor = start + with.len();
    }

    /// Char-wrapped visual rows plus the cursor's (row, col).
    pub fn layout(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut rows = vec![String::new()];
        let mut col = 0;
        let mut cur = (0, 0);
        let mut byte = 0;
        for g in self.text.graphemes(true) {
            if byte == self.cursor {
                cur = (rows.len() - 1, col);
            }
            byte += g.len();
            if g == "\n" {
                rows.push(String::new());
                col = 0;
                continue;
            }
            let w = UnicodeWidthStr::width(g);
            if col + w > width {
                rows.push(String::new());
                col = 0;
            }
            rows.last_mut().unwrap().push_str(g);
            col += w;
        }
        if byte == self.cursor {
            if col >= width {
                rows.push(String::new());
                col = 0;
            }
            cur = (rows.len() - 1, col);
        }
        (rows, cur)
    }
}

fn advance(text: &str, start: usize, end: usize, cols: usize) -> usize {
    text[start..end]
        .char_indices()
        .nth(cols)
        .map_or(end, |(i, _)| start + i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_and_backspace_unicode() {
        let mut e = Editor::default();
        e.insert("héllo👋");
        e.backspace();
        assert_eq!(e.text, "héllo");
        e.left();
        e.backspace();
        assert_eq!(e.text, "hélo");
    }

    #[test]
    fn delete_word() {
        let mut e = Editor::default();
        e.insert("open src/main.rs");
        e.delete_word();
        assert_eq!(e.text, "open src/");
    }

    #[test]
    fn up_down_multiline_then_history() {
        let mut e = Editor::default();
        e.insert("first");
        e.take();
        e.insert("ab\ncd");
        e.up();
        assert_eq!(e.cursor, 2);
        e.up();
        assert_eq!(e.text, "first");
        e.down();
        assert_eq!(e.text, "ab\ncd");
    }

    #[test]
    fn token_detection() {
        let mut e = Editor::default();
        e.insert("look at @src/ma");
        assert_eq!(e.token_at_cursor(), Some((8, "@src/ma")));
        e.replace_token(8, "@src/main.rs ");
        assert_eq!(e.text, "look at @src/main.rs ");
        e.clear();
        e.insert("/comp");
        assert_eq!(e.token_at_cursor(), Some((0, "/comp")));
    }

    #[test]
    fn long_paste_collapses_and_backspace_removes_tag() {
        let mut e = Editor::default();
        e.insert("see ");
        e.paste("a\nb\nc\nd\ne");
        assert_eq!(e.text, "see [Pasted text #1 +5 lines]");
        let atts = e.attachments_in(&e.text.clone());
        assert!(matches!(&atts[0].1, Attachment::Text(t) if t == "a\nb\nc\nd\ne"));
        e.backspace();
        assert_eq!(e.text, "see ");
    }

    #[test]
    fn short_paste_stays_inline() {
        let mut e = Editor::default();
        e.paste("hello\nworld");
        assert_eq!(e.text, "hello\nworld");
    }

    #[test]
    fn layout_wraps_and_tracks_cursor() {
        let mut e = Editor::default();
        e.insert("abcdef\ngh");
        let (rows, cur) = e.layout(4);
        assert_eq!(rows, vec!["abcd", "ef", "gh"]);
        assert_eq!(cur, (2, 2));
    }
}
