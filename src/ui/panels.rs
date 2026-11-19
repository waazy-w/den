//! Model/effort picker, files-changed viewer and conversation search.

use super::wrap::width;
use super::{centered, modal_block, model_label, project_label, truncate, truncate_left};
use crate::app::{App, search_hits};
use crate::session::FileChange;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Clear;
use std::path::Path;

fn hint(f: &mut Frame, inner: Rect, text: &str) {
    f.buffer_mut().set_string(inner.x + 2, inner.bottom().saturating_sub(1), text, theme::s(theme::faint()));
}

pub fn model(f: &mut Frame, area: Rect, app: &App, sel: usize, effort: usize) {
    let models = app.model_choices();
    let s = app.current();
    let h = models.len() as u16 + 11;
    let rect = centered(area, 70, h);
    f.render_widget(Clear, rect);
    let b = modal_block("model & effort");
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let cur = s.model.as_deref().unwrap_or("default");
    let w = inner.width as usize;
    let buf = f.buffer_mut();
    let mut y = inner.y + 1;
    for (i, m) in models.iter().enumerate() {
        let on = i == sel;
        let current = m.value == cur || (m.value != "default" && !m.resolved.is_empty() && m.resolved == cur);
        let bg = if on { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let name = if m.name.is_empty() { model_label(Some(&m.value)) } else { m.name.clone() };
        let name = truncate(&name, 24);
        let mark = if current { "●" } else { " " };
        let desc = truncate(&m.desc, w.saturating_sub(34));
        let pad = w.saturating_sub(4 + 24 + 1 + width(&desc) + 3);
        buf.set_line(
            inner.x,
            y,
            &Line::from(vec![
                Span::styled(if on { " ▍" } else { "  " }, bg.fg(theme::accent())),
                Span::styled(format!(" {name:<24} "), if on { bg.fg(theme::text()).add_modifier(Modifier::BOLD) } else { bg.fg(theme::dim()) }),
                Span::styled(desc, bg.fg(theme::faint())),
                Span::styled(" ".repeat(pad), bg),
                Span::styled(format!(" {mark} "), bg.fg(theme::accent())),
            ]),
            inner.width,
        );
        y += 1;
    }
    y += 1;
    buf.set_string(inner.x + 2, y, "─".repeat(w.saturating_sub(4)), theme::s(theme::rule()));
    y += 2;
    let efforts = models.get(sel).map(App::effort_choices).unwrap_or_default();
    let cur_effort = s.effort.as_deref().unwrap_or("auto");
    let mut spans = vec![Span::styled("  thinking effort   ", theme::s(theme::dim()))];
    if efforts.len() <= 1 {
        spans.push(Span::styled("not adjustable for this model", theme::s(theme::faint())));
    }
    for (i, e) in efforts.iter().enumerate().take(if efforts.len() <= 1 { 0 } else { usize::MAX }) {
        let picked = i == effort;
        let st = if picked {
            Style::new().fg(theme::on_accent()).bg(theme::accent()).add_modifier(Modifier::BOLD)
        } else if e == cur_effort {
            theme::s(theme::text())
        } else {
            theme::s(theme::dim())
        };
        spans.push(Span::styled(format!(" {e} "), st));
        spans.push(Span::raw(" "));
    }
    buf.set_line(inner.x, y, &Line::from(spans), inner.width);
    y += 1;
    if efforts.get(effort).is_some_and(|e| e != cur_effort) {
        let note = if s.busy() { "  restarts claude after this reply · conversation kept" } else { "  restarts claude · conversation kept" };
        buf.set_string(inner.x, y + 1, note, theme::s(theme::faint()));
    }
    hint(f, inner, "↑↓ model   ←→ effort   ⏎ apply   esc cancel");
}

fn rel(p: &Path, cwd: &Path) -> String {
    p.strip_prefix(cwd).map_or_else(|_| crate::tools::tilde(&p.to_string_lossy()), |r| r.to_string_lossy().into_owned())
}

fn counts(c: &FileChange) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("+{}", c.added), theme::s(theme::add_fg())),
        Span::raw(" "),
        Span::styled(format!("−{}", c.removed), theme::s(theme::del_fg())),
    ]
}

/// Diff lines for every edit made to one file.
pub fn diff_lines(c: &FileChange, w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let n = c.edits.len();
    let row = |sign: &str, text: &str, fg, bg| {
        let body = truncate(&text.replace('\t', "    "), w.saturating_sub(3));
        let pad = w.saturating_sub(2 + width(&body));
        Line::from(vec![
            Span::styled(format!("{sign} "), Style::new().fg(fg).bg(bg)),
            Span::styled(body, Style::new().fg(theme::text()).bg(bg)),
            Span::styled(" ".repeat(pad), Style::new().bg(bg)),
        ])
    };
    for (k, (old, new)) in c.edits.iter().enumerate() {
        let o: Vec<&str> = old.lines().collect();
        let nw: Vec<&str> = new.lines().collect();
        let title = if old.is_empty() { format!("new contents · {} lines", nw.len()) } else { format!("edit {}/{n}", k + 1) };
        out.push(Line::from(Span::styled(format!("@@ {title}"), theme::s(theme::accent_soft()))));
        let pre = o.iter().zip(&nw).take_while(|(a, b)| a == b).count();
        let suf = o[pre..].iter().rev().zip(nw[pre..].iter().rev()).take_while(|(a, b)| a == b).count();
        let ctx = theme::code_bg();
        for l in &o[pre.saturating_sub(2)..pre] {
            out.push(row(" ", l, theme::faint(), ctx));
        }
        for l in &o[pre..o.len() - suf] {
            out.push(row("-", l, theme::del_fg(), theme::del_bg()));
        }
        for l in nw[pre..nw.len() - suf].iter().take(400) {
            out.push(row("+", l, theme::add_fg(), theme::add_bg()));
        }
        for l in o[o.len() - suf..].iter().take(2) {
            out.push(row(" ", l, theme::faint(), ctx));
        }
        out.push(Line::default());
    }
    out
}

pub fn files(f: &mut Frame, area: Rect, app: &App, sel: usize, scroll: u16) {
    let s = app.current();
    let rect = centered(area, area.width.saturating_sub(6), area.height.saturating_sub(4));
    f.render_widget(Clear, rect);
    let (added, removed) = s.changes.iter().fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
    let n = s.changes.len();
    let title = format!("files changed · {n} file{} · +{added} −{removed}", if n == 1 { "" } else { "s" });
    let b = modal_block(&title);
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let body = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(3) };
    let side = body.width >= 90;
    let list_w = if side { (body.width / 3).clamp(24, 40) } else { body.width };
    let list_h = if side { body.height } else { (n as u16).min(body.height / 3).max(1) };
    let buf = f.buffer_mut();
    let first = sel.saturating_sub(list_h.saturating_sub(1) as usize);
    for (row, (i, c)) in s.changes.iter().enumerate().skip(first).take(list_h as usize).enumerate() {
        let on = i == sel;
        let bg = if on { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let cnt = counts(c);
        let cw: usize = cnt.iter().map(|x| width(&x.content)).sum();
        let name = truncate_left(&rel(&c.path, &s.cwd), (list_w as usize).saturating_sub(cw + 4));
        let pad = (list_w as usize).saturating_sub(width(&name) + cw + 3);
        let mut spans = vec![
            Span::styled(if on { "▍" } else { " " }, bg.fg(theme::accent())),
            Span::styled(format!(" {name}"), if on { bg.fg(theme::text()).add_modifier(Modifier::BOLD) } else { bg.fg(theme::dim()) }),
            Span::styled(" ".repeat(pad), bg),
        ];
        spans.extend(cnt.into_iter().map(|x| x.patch_style(bg)));
        spans.push(Span::styled(" ", bg));
        buf.set_line(body.x, body.y + row as u16, &Line::from(spans), list_w);
    }
    let diff = if side {
        for y in body.y..body.bottom() {
            buf.set_string(body.x + list_w + 1, y, "│", theme::s(theme::rule()));
        }
        Rect { x: body.x + list_w + 3, y: body.y, width: body.width.saturating_sub(list_w + 3), height: body.height }
    } else {
        buf.set_string(body.x, body.y + list_h, "─".repeat(body.width as usize), theme::s(theme::rule()));
        Rect { x: body.x, y: body.y + list_h + 1, width: body.width, height: body.height.saturating_sub(list_h + 1) }
    };
    if let Some(c) = s.changes.get(sel) {
        let lines = diff_lines(c, diff.width as usize);
        let max = lines.len().saturating_sub(diff.height as usize);
        let top = (scroll as usize).min(max);
        for (k, l) in lines.iter().skip(top).take(diff.height as usize).enumerate() {
            buf.set_line(diff.x, diff.y + k as u16, l, diff.width);
        }
        if max > 0 {
            let label = format!(" {}–{} of {} ", top + 1, (top + diff.height as usize).min(lines.len()), lines.len());
            buf.set_string(diff.right().saturating_sub(width(&label) as u16), inner.bottom().saturating_sub(1), label, theme::s(theme::faint()));
        }
    }
    hint(f, inner, "↑↓ file   space/PgDn scroll   esc close");
}

pub fn search(f: &mut Frame, area: Rect, app: &App, query: &crate::input::Editor, sel: usize) {
    let hits = search_hits(&app.sessions, &query.text);
    let rect = centered(area, area.width.saturating_sub(10).min(100), area.height.saturating_sub(6));
    f.render_widget(Clear, rect);
    let title = if query.text.trim().len() < 2 { "search".to_string() } else { format!("search · {} matches", hits.len()) };
    let b = modal_block(&title);
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let buf = f.buffer_mut();
    buf.set_string(inner.x + 1, inner.y, "›", theme::bold(theme::accent()));
    if query.is_empty() {
        buf.set_string(inner.x + 3, inner.y, "search every conversation…", theme::s(theme::faint()));
    } else {
        buf.set_string(inner.x + 3, inner.y, &query.text, theme::s(theme::text()));
    }
    let q = query.text.trim().to_lowercase();
    let rows = inner.height.saturating_sub(4) as usize;
    let first = sel.saturating_sub(rows.saturating_sub(1));
    let w = inner.width as usize;
    if hits.is_empty() && q.len() >= 2 {
        buf.set_string(inner.x + 3, inner.y + 2, "no matches", theme::s(theme::dim()));
    }
    for (row, (i, h)) in hits.iter().enumerate().skip(first).take(rows).enumerate() {
        let on = i == sel;
        let bg = if on { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let s = &app.sessions[h.session];
        let who = truncate(&format!("{} · {}", project_label(app, &s.cwd), s.name), 26);
        let role = format!("{:<6}", h.role);
        let room = w.saturating_sub(4 + 26 + 2 + 7);
        // Keep the match in view: start a little before it.
        let lower = h.line.to_lowercase();
        let at = lower.find(&q).unwrap_or(0);
        let start = h.line.char_indices().map(|(b, _)| b).filter(|b| *b <= at).rev().nth(20).unwrap_or(0);
        let snippet = if start > 0 { format!("…{}", &h.line[start..]) } else { h.line.clone() };
        let snippet = truncate(&snippet, room);
        let mut spans = vec![
            Span::styled(if on { " ▍" } else { "  " }, bg.fg(theme::accent())),
            Span::styled(format!(" {who:<26}  "), bg.fg(if on { theme::text() } else { theme::dim() })),
            Span::styled(role, bg.fg(theme::faint())),
            Span::raw(" ").patch_style(bg),
        ];
        // Highlight the matched text.
        let sl = snippet.to_lowercase();
        let found = sl
            .find(&q)
            .filter(|p| sl.len() == snippet.len() && snippet.is_char_boundary(*p) && snippet.is_char_boundary(p + q.len()));
        match found {
            Some(p) => {
                spans.push(Span::styled(snippet[..p].to_string(), bg.fg(theme::text())));
                spans.push(Span::styled(snippet[p..p + q.len()].to_string(), bg.fg(theme::accent()).add_modifier(Modifier::BOLD)));
                spans.push(Span::styled(snippet[p + q.len()..].to_string(), bg.fg(theme::text())));
            }
            None => spans.push(Span::styled(snippet, bg.fg(theme::text()))),
        }
        let used: usize = spans.iter().map(|x| width(&x.content)).sum();
        spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), bg));
        buf.set_line(inner.x, inner.y + 2 + row as u16, &Line::from(spans), inner.width);
    }
    hint(f, inner, "↑↓ pick   ⏎ jump there   esc close");
    let cx = width(&query.text[..query.cursor]);
    f.set_cursor_position(Position::new(inner.x + 3 + cx as u16, inner.y));
}

pub fn theme(f: &mut Frame, area: Rect, sel: usize) {
    let list = theme::PALETTES;
    let rect = centered(area, 64, list.len() as u16 + 5);
    f.render_widget(Clear, rect);
    let b = modal_block("theme");
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let buf = f.buffer_mut();
    let w = inner.width as usize;
    for (i, t) in list.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        let on = i == sel;
        let bg = if on { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let c = |x: theme::Rgb| theme::mix(x, x, 1.0);
        let mut spans = vec![
            Span::styled(if on { " ▍" } else { "  " }, bg.fg(theme::accent())),
            Span::styled(format!(" {:<10}", t.name), if on { bg.fg(theme::text()).add_modifier(Modifier::BOLD) } else { bg.fg(theme::dim()) }),
        ];
        // Swatch: background with text, accent, ok, warn and error samples.
        let sw_bg = c(t.bg.unwrap_or((0x18, 0x18, 0x1B)));
        spans.push(Span::styled(" Aa ", Style::new().bg(sw_bg).fg(c(t.text))));
        for x in [t.accent, t.ok, t.warn, t.err] {
            spans.push(Span::styled("■", Style::new().bg(sw_bg).fg(c(x))));
        }
        spans.push(Span::styled(" ", Style::new().bg(sw_bg)));
        spans.push(Span::styled(format!("  {}", truncate(t.blurb, w.saturating_sub(28))), bg.fg(theme::faint())));
        let used: usize = spans.iter().map(|x| width(&x.content)).sum();
        spans.push(Span::styled(" ".repeat(w.saturating_sub(used)), bg));
        buf.set_line(inner.x, y, &Line::from(spans), inner.width);
    }
    hint(f, inner, "↑↓ preview   ⏎ keep   esc cancel");
}

fn settings_rect(area: Rect) -> Rect {
    centered(area, 78, crate::app::SETTINGS.len() as u16 * 2 + 6)
}

/// Click targets for the settings rows (same geometry as `settings`).
pub fn settings_rows(area: Rect) -> Vec<Rect> {
    let r = settings_rect(area);
    let inner = Rect { x: r.x + 1, y: r.y + 1, width: r.width.saturating_sub(2), height: r.height.saturating_sub(2) };
    (0..crate::app::SETTINGS.len()).map(|i| Rect::new(inner.x, inner.y + 1 + i as u16 * 2, inner.width, 1)).collect()
}

pub fn settings(f: &mut Frame, area: Rect, app: &App, sel: usize) {
    let rect = settings_rect(area);
    f.render_widget(Clear, rect);
    let b = modal_block("settings");
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let p = app.prefs;
    let rows = settings_rows(area);
    let buf = f.buffer_mut();
    let w = inner.width as usize;
    for (i, ((label, hint), r)) in crate::app::SETTINGS.iter().zip(&rows).enumerate() {
        let on = i == sel || super::hovered(app.hover, *r);
        let bg = if on { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let value: Span = match i {
            0 => Span::styled(format!("{} ›", theme::p().name), bg.fg(theme::accent())),
            _ => {
                let v = [p.typing_effect, p.notifications, p.auto_update, p.outfit, p.squeeze, p.auto_resume, p.smart_compact][i - 1];
                if v {
                    Span::styled(" ● on ", bg.fg(theme::on_accent()).bg(theme::accent()).add_modifier(Modifier::BOLD))
                } else {
                    Span::styled(" ○ off ", bg.fg(theme::dim()).bg(theme::hi_bg()))
                }
            }
        };
        let vw = width(&value.content);
        let left = format!(" {label:<15}");
        let hint = truncate(hint, w.saturating_sub(width(&left) + vw + 6));
        let pad = w.saturating_sub(2 + width(&left) + width(&hint) + vw + 1);
        buf.set_line(
            inner.x,
            r.y,
            &Line::from(vec![
                Span::styled(if i == sel { " ▍" } else { "  " }, bg.fg(theme::accent())),
                Span::styled(left, if on { bg.fg(theme::text()).add_modifier(Modifier::BOLD) } else { bg.fg(theme::text()) }),
                Span::styled(hint, bg.fg(theme::faint())),
                Span::styled(" ".repeat(pad), bg),
                value,
                Span::styled(" ", bg),
            ]),
            inner.width,
        );
    }
    hint(f, inner, "↑↓ choose   ⏎ / click toggle   esc close");
}

pub fn doctor(f: &mut Frame, area: Rect, checks: &[crate::app::Check]) {
    let rows: u16 = checks.iter().map(|c| if c.fix.is_empty() { 1 } else { 2 }).sum();
    let rect = centered(area, 104, rows + checks.len() as u16 + 5);
    f.render_widget(Clear, rect);
    let b = modal_block("doctor");
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let buf = f.buffer_mut();
    let w = inner.width as usize;
    let mut y = inner.y + 1;
    for c in checks {
        let (icon, st) = match c.ok {
            Some(true) => ("✓", theme::s(theme::ok())),
            Some(false) => ("✗", theme::bold(theme::err())),
            None => ("·", theme::s(theme::dim())),
        };
        buf.set_line(
            inner.x + 2,
            y,
            &Line::from(vec![
                Span::styled(format!("{icon} "), st),
                Span::styled(format!("{:<16}", c.label), theme::bold(theme::text())),
                Span::styled(truncate(&c.detail, w.saturating_sub(24)), theme::s(theme::dim())),
            ]),
            inner.width.saturating_sub(2),
        );
        y += 1;
        if !c.fix.is_empty() {
            let fix_st = if c.ok == Some(false) { theme::s(theme::warn()) } else { theme::s(theme::faint()) };
            buf.set_string(inner.x + 20, y, truncate(&format!("→ {}", c.fix), w.saturating_sub(22)), fix_st);
            y += 1;
        }
        y += 1;
    }
    hint(f, inner, "⏎ check again   esc close");
}
