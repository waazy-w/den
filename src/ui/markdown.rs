use super::wrap::{width, wrap};
use crate::theme;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

struct ListCtx {
    next: Option<u64>,
}

struct R {
    out: Vec<Line<'static>>,
    width: usize,
    indent: String,
    spans: Vec<Span<'static>>,
    styles: Vec<Style>,
    lists: Vec<ListCtx>,
    quote: usize,
    item_prefix: Option<String>,
    code: Option<(String, String)>,
    table: Option<Table>,
    pending_gap: bool,
    collapse: bool,
}

#[derive(Default)]
struct Table {
    rows: Vec<Vec<Vec<Span<'static>>>>,
    cur_row: Vec<Vec<Span<'static>>>,
    head_rows: usize,
}

/// Code blocks longer than this fold into a one-line chip when collapsed.
const CODE_COLLAPSE: usize = 4;

#[cfg(test)]
pub fn render(src: &str, width: usize, indent: usize) -> Vec<Line<'static>> {
    render_opts(src, width, indent, false)
}

pub fn render_opts(src: &str, width: usize, indent: usize, collapse: bool) -> Vec<Line<'static>> {
    let mut r = R {
        collapse,
        out: Vec::new(),
        width,
        indent: " ".repeat(indent),
        spans: Vec::new(),
        styles: vec![theme::s(theme::text())],
        lists: Vec::new(),
        quote: 0,
        item_prefix: None,
        code: None,
        table: None,
        pending_gap: false,
    };
    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    for ev in Parser::new_ext(src, opts) {
        r.event(ev);
    }
    r.flush();
    if let Some((lang, body)) = r.code.take() {
        r.code_block(&lang, &body);
    }
    r.out
}

impl R {
    fn style(&self) -> Style {
        *self.styles.last().unwrap()
    }

    fn push_style(&mut self, f: impl FnOnce(Style) -> Style) {
        let s = f(self.style());
        self.styles.push(s);
    }

    fn prefix(&self) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
        let mut base = vec![Span::raw(self.indent.clone())];
        for _ in 0..self.quote {
            base.push(Span::styled("│ ", theme::s(theme::faint())));
        }
        let list_indent = self.lists.len().saturating_sub(1) * 2;
        if !self.lists.is_empty() {
            base.push(Span::raw(" ".repeat(list_indent)));
        }
        let mut first = base.clone();
        let mut rest = base;
        if let Some(p) = &self.item_prefix {
            let w = width(p);
            first.push(Span::styled(p.clone(), theme::s(theme::dim())));
            rest.push(Span::raw(" ".repeat(w)));
        } else if !self.lists.is_empty() {
            rest.push(Span::raw("  "));
            first.push(Span::raw("  "));
        }
        (first, rest)
    }

    fn gap(&mut self) {
        if self.pending_gap && !self.out.is_empty() {
            self.out.push(Line::default());
        }
        self.pending_gap = false;
    }

    fn flush(&mut self) {
        if self.spans.is_empty() {
            return;
        }
        self.gap();
        let spans = std::mem::take(&mut self.spans);
        let (first, rest) = self.prefix();
        self.out.extend(wrap(&spans, self.width, &first, &rest));
        self.item_prefix = None;
    }

    fn event(&mut self, ev: Event) {
        if let Some((_, body)) = &mut self.code {
            match ev {
                Event::Text(t) => {
                    body.push_str(&t);
                    return;
                }
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, body) = self.code.take().unwrap();
                    self.gap();
                    self.code_block(&lang, &body);
                    self.pending_gap = true;
                    return;
                }
                _ => return,
            }
        }
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(t) => {
                let st = Style::new().fg(theme::code_fg());
                self.add_span(Span::styled(t.to_string(), st));
            }
            Event::SoftBreak => self.flush_keep_item(),
            Event::HardBreak => self.flush_keep_item(),
            Event::Rule => {
                self.flush();
                self.gap();
                let w = self.width.saturating_sub(self.indent.len()).min(48);
                self.out.push(Line::from(vec![
                    Span::raw(self.indent.clone()),
                    Span::styled("─".repeat(w), theme::s(theme::faint())),
                ]));
                self.pending_gap = true;
            }
            Event::TaskListMarker(done) => {
                let m = if done { "☑ " } else { "☐ " };
                self.add_span(Span::styled(m, theme::s(theme::dim())));
            }
            Event::Html(t) | Event::InlineHtml(t) => self.text(&t),
            _ => {}
        }
    }

    fn flush_keep_item(&mut self) {
        if self.table.is_some() {
            self.text(" ");
            return;
        }
        self.flush();
    }

    fn add_span(&mut self, span: Span<'static>) {
        if let Some(t) = &mut self.table {
            if let Some(c) = t.cur_row.last_mut() {
                c.push(span);
            }
        } else {
            self.spans.push(span);
        }
    }

    fn text(&mut self, t: &str) {
        let st = self.style();
        self.add_span(Span::styled(t.to_string(), st));
    }

    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph => {
                if self.item_prefix.is_none() {
                    self.flush();
                }
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.pending_gap = !self.out.is_empty();
                let fg = if level == HeadingLevel::H1 { theme::accent() } else { theme::text() };
                self.push_style(|_| theme::bold(fg));
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quote += 1;
                self.push_style(|s| s.fg(theme::dim()).add_modifier(Modifier::ITALIC));
            }
            Tag::CodeBlock(kind) => {
                self.flush();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split(',').next().unwrap_or("").trim().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                self.code = Some((lang, String::new()));
            }
            Tag::List(start) => {
                self.flush();
                if self.lists.is_empty() {
                    self.pending_gap = !self.out.is_empty();
                }
                self.lists.push(ListCtx { next: start });
            }
            Tag::Item => {
                self.flush();
                let l = self.lists.last_mut();
                let p = match l.and_then(|l| {
                    let n = l.next;
                    if let Some(n) = &mut l.next {
                        *n += 1;
                    }
                    n
                }) {
                    Some(n) => format!("{n}. "),
                    None => "• ".to_string(),
                };
                self.item_prefix = Some(p);
            }
            Tag::Emphasis => self.push_style(|s| s.add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.push_style(|s| s.add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self.push_style(|s| s.add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { .. } => self.push_style(|s| s.fg(theme::accent()).add_modifier(Modifier::UNDERLINED)),
            Tag::Table(_) => {
                self.flush();
                self.table = Some(Table::default());
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(t) = &mut self.table {
                    t.cur_row.clear();
                }
            }
            Tag::TableCell => {
                if let Some(t) = &mut self.table {
                    t.cur_row.push(Vec::new());
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                self.flush();
                self.pending_gap = self.lists.is_empty();
            }
            TagEnd::Heading(_) => {
                self.flush();
                self.styles.pop();
                self.pending_gap = true;
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote -= 1;
                self.styles.pop();
                self.pending_gap = true;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.pending_gap = true;
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link => {
                self.styles.pop();
            }
            TagEnd::TableHead => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.cur_row);
                    t.rows.push(row);
                    t.head_rows = t.rows.len();
                }
            }
            TagEnd::TableRow => {
                if let Some(t) = &mut self.table {
                    let row = std::mem::take(&mut t.cur_row);
                    t.rows.push(row);
                }
            }
            TagEnd::Table => {
                if let Some(t) = self.table.take() {
                    self.gap();
                    self.render_table(t);
                    self.pending_gap = true;
                }
            }
            _ => {}
        }
    }

    fn code_block(&mut self, lang: &str, body: &str) {
        let n = body.trim_end_matches('\n').split('\n').count();
        if self.collapse && n > CODE_COLLAPSE {
            let name = if lang.is_empty() { "code" } else { lang };
            self.out.push(Line::from(vec![
                Span::raw(self.indent.clone()),
                Span::styled("▸ ", theme::s(theme::dim())),
                Span::styled(name.to_string(), theme::s(theme::accent_soft())),
                Span::styled(format!(" · {n} lines"), theme::s(theme::dim())),
                Span::styled("   ^O to show", theme::s(theme::faint())),
            ]));
            return;
        }
        let bg = Style::new().bg(theme::code_bg());
        let inner = self.width.saturating_sub(self.indent.len()).max(4);
        let label = if lang.is_empty() { String::new() } else { format!(" {lang}") };
        let mut header = vec![Span::raw(self.indent.clone())];
        header.push(Span::styled(
            format!("{label:<inner$}"),
            bg.fg(theme::dim()),
        ));
        self.out.push(Line::from(header));
        for raw in body.trim_end_matches('\n').split('\n') {
            let raw = raw.replace('\t', "    ");
            let first = vec![Span::raw(self.indent.clone()), Span::styled(" ", bg)];
            let lines = wrap(
                &[Span::styled(raw, bg.fg(theme::text()))],
                self.width.saturating_sub(1),
                &first,
                &first,
            );
            for mut l in lines {
                let w: usize = l.spans.iter().map(|s| width(&s.content)).sum();
                let pad = self.width.saturating_sub(w);
                l.spans.push(Span::styled(" ".repeat(pad), bg));
                self.out.push(l);
            }
        }
        let mut footer = vec![Span::raw(self.indent.clone())];
        footer.push(Span::styled(" ".repeat(inner), bg));
        self.out.push(Line::from(footer));
    }

    fn render_table(&mut self, t: Table) {
        let cols = t.rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if cols == 0 {
            return;
        }
        let cell_w = |c: &Vec<Span>| c.iter().map(|s| width(&s.content)).sum::<usize>();
        let mut widths = vec![0usize; cols];
        for row in &t.rows {
            for (i, c) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell_w(c));
            }
        }
        let total: usize = widths.iter().sum::<usize>() + 3 * (cols - 1) + self.indent.len();
        let sep = Span::styled(" │ ", theme::s(theme::faint()));
        if total > self.width {
            for row in t.rows {
                let mut spans = Vec::new();
                for (i, c) in row.into_iter().enumerate() {
                    if i > 0 {
                        spans.push(sep.clone());
                    }
                    spans.extend(c);
                }
                let p = vec![Span::raw(self.indent.clone())];
                self.out.extend(wrap(&spans, self.width, &p, &p));
            }
            return;
        }
        for (ri, row) in t.rows.into_iter().enumerate() {
            let mut spans = vec![Span::raw(self.indent.clone())];
            for (i, c) in row.into_iter().enumerate() {
                if i > 0 {
                    spans.push(sep.clone());
                }
                let w = cell_w(&c);
                let head = ri < t.head_rows;
                for mut s in c {
                    if head {
                        s.style = s.style.add_modifier(Modifier::BOLD);
                    }
                    spans.push(s);
                }
                spans.push(Span::raw(" ".repeat(widths[i] - w)));
            }
            self.out.push(Line::from(spans));
            if ri + 1 == t.head_rows {
                let mut rule = vec![Span::raw(self.indent.clone())];
                let parts: Vec<String> = widths.iter().map(|w| "─".repeat(*w)).collect();
                rule.push(Span::styled(parts.join("─┼─"), theme::s(theme::faint())));
                self.out.push(Line::from(rule));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(lines: &[Line]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>().trim_end().to_string())
            .collect()
    }

    #[test]
    fn paragraphs_and_lists() {
        let out = plain(&render("Hello **world**.\n\n- one\n- two\n\nDone", 40, 2));
        assert_eq!(out, vec!["  Hello world.", "", "  • one", "  • two", "", "  Done"]);
    }

    #[test]
    fn soft_breaks_keep_lines() {
        let out = plain(&render("1\n2\n3", 40, 0));
        assert_eq!(out, vec!["1", "2", "3"]);
    }

    #[test]
    fn ordered_list_numbers() {
        let out = plain(&render("1. a\n2. b", 40, 0));
        assert_eq!(out, vec!["1. a", "2. b"]);
    }

    #[test]
    fn code_block_has_frame() {
        let out = plain(&render("```rust\nfn x() {}\n```", 30, 0));
        assert_eq!(out, vec![" rust", " fn x() {}", ""]);
    }

    #[test]
    fn unterminated_code_block_while_streaming() {
        let out = plain(&render("```\nlet a = 1;", 30, 0));
        assert!(out.iter().any(|l| l.contains("let a = 1;")));
    }

    #[test]
    fn long_code_collapses() {
        let body: String = (0..30).map(|i| format!("line{i}\n")).collect();
        let out = plain(&render_opts(&format!("```\n{body}```"), 50, 0, true));
        assert_eq!(out, vec!["▸ code · 30 lines   ^O to show"]);
    }

    #[test]
    fn table_aligns() {
        let out = plain(&render("| a | bb |\n|---|---|\n| ccc | d |", 40, 0));
        assert_eq!(out[0], "a   │ bb");
        assert_eq!(out[2], "ccc │ d");
    }
}
