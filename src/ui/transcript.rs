use super::markdown;
use super::wrap::{width, wrap};
use crate::session::{Entry, Item, NoteKind, Session, Tool, ToolState};
use crate::{anim, theme};
use crate::tools::{self, DiffKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use std::path::Path;

const MAX_BODY: usize = 60;

/// Blank rows before an entry; a new user turn gets blank · rule · blank.
fn gap_before(prev: Option<&Item>, cur: &Item) -> usize {
    match (prev, cur) {
        (None, _) => 0,
        (Some(_), Item::User(_)) => 3,
        (Some(Item::Tool(_)), Item::Tool(_)) => 0,
        _ => 1,
    }
}

fn rule_line(w: u16) -> Line<'static> {
    Line::from(Span::styled("─".repeat(w as usize), theme::s(theme::rule())))
}

pub fn draw(buf: &mut Buffer, area: Rect, s: &mut Session, expand_all: bool) {
    if area.height == 0 || area.width < 8 {
        return;
    }
    let w = area.width;
    let cwd = s.cwd.clone();
    let mut heights = Vec::with_capacity(s.entries.len());
    let mut total = 0usize;
    let mut sel_range = None;
    let mut jump_range = None;
    let jump = s.jump.map(|(j, _)| j);
    let mut prev: Option<usize> = None;
    for i in 0..s.entries.len() {
        if crate::session::quiet(&s.entries[i].item) {
            heights.push((0, 0));
            continue;
        }
        let gap = gap_before(prev.map(|p| &s.entries[p].item), &s.entries[i].item);
        let selected = s.selected == Some(i);
        let h = ensure_cached(&mut s.entries[i], w, selected, expand_all, &cwd);
        if selected {
            sel_range = Some((total + gap, total + gap + h));
        }
        if jump == Some(i) {
            jump_range = Some(total + gap);
        }
        total += gap + h;
        heights.push((gap, h));
        prev = Some(i);
    }
    let status = status_line(s);
    if status.is_some() {
        total += 2;
    }

    let view = area.height as usize;
    if s.scroll > 0 && total > s.last_total {
        let grew = total - s.last_total;
        s.scroll += grew;
        s.scroll_pos += grew as f32;
    }
    s.last_total = total;
    let max_scroll = total.saturating_sub(view);
    if s.ensure_visible {
        if let Some((a, b)) = sel_range {
            let top = max_scroll.saturating_sub(s.scroll.min(max_scroll));
            if a < top {
                s.scroll = max_scroll - a;
            } else if b > top + view {
                s.scroll = max_scroll.saturating_sub(b.saturating_sub(view));
            }
        }
        s.ensure_visible = false;
    }
    if s.jump_pending {
        s.jump_pending = false;
        if let Some(a) = jump_range {
            s.scroll = max_scroll - a.saturating_sub(1).min(max_scroll);
        }
    }
    s.scroll = s.scroll.min(max_scroll);
    s.scroll_pos = s.scroll_pos.clamp(0.0, max_scroll as f32);
    let flash = s.jump.and_then(|(j, at)| {
        let t = at.elapsed().as_secs_f32() / crate::app::JUMP_FLASH.as_secs_f32();
        (t < 1.0).then_some((j, 1.0 - t))
    });
    let mut flash_rows: Option<(u16, u16, f32)> = None;
    let top = max_scroll - (s.scroll_pos.round() as usize).min(max_scroll);

    let mut y = 0usize;
    let mut row = area.y;
    let bottom = area.y + area.height;
    let mut fades: Vec<(u16, u16, f32)> = Vec::new();
    for (i, (gap, h)) in heights.iter().enumerate() {
        if crate::session::quiet(&s.entries[i].item) {
            continue;
        }
        if y + gap + h <= top {
            y += gap + h;
            continue;
        }
        for g in 0..*gap {
            if y >= top && row < bottom {
                if *gap == 3 && g == 1 {
                    buf.set_line(area.x, row, &rule_line(w), w);
                }
                row += 1;
            }
            y += 1;
        }
        let e = &s.entries[i];
        let start_row = row;
        for l in &e.cache.as_ref().expect("cached above").lines {
            if y >= top && row < bottom {
                buf.set_line(area.x, row, l, w);
                row += 1;
            }
            y += 1;
        }
        if let Some((j, k)) = flash
            && j == i
            && row > start_row
        {
            flash_rows = Some((start_row, row - start_row, k));
        }
        let t = anim::progress(e.born, anim::FADE_IN);
        if t < 1.0 && row > start_row {
            fades.push((start_row, row - start_row, anim::ease_out(t)));
        }
        if row >= bottom {
            break;
        }
    }
    if let Some(st) = status
        && row + 1 < bottom
        && y + 1 >= top
    {
        buf.set_line(area.x, row + 1, &st, w);
    }
    for (r, h, t) in fades {
        theme::fade(buf, Rect::new(area.x, r, w, h), t);
    }
    if let Some((r, h, k)) = flash_rows {
        let bg = theme::mix(theme::bg_rgb(), theme::p().sel_bg, anim::ease_out(k));
        buf.set_style(Rect::new(area.x, r, w, h), ratatui::style::Style::new().bg(bg));
        let bar = Rect::new(area.x.saturating_sub(2), r, 1, h);
        for yy in bar.y..bar.bottom() {
            buf.set_string(bar.x, yy, "▍", theme::s(theme::accent()));
        }
    }
}

fn ensure_cached(e: &mut Entry, w: u16, selected: bool, expand_all: bool, cwd: &Path) -> usize {
    let live = match &e.item {
        Item::Tool(t) => t.state == ToolState::Running || anim::active(t.finished_at, anim::FLASH),
        _ => false,
    };
    let key = (w, e.rev, selected, e.expanded || expand_all);
    let stale = live || e.cache.as_ref().is_none_or(|c| c.key != key);
    if stale {
        let lines = render_entry(e, w as usize, selected, expand_all, cwd);
        e.cache = Some(crate::session::RenderCache { key, lines });
    }
    e.cache.as_ref().unwrap().lines.len()
}

fn render_entry(e: &Entry, w: usize, selected: bool, expand_all: bool, cwd: &Path) -> Vec<Line<'static>> {
    match &e.item {
        Item::User(text) => {
            let bar = Span::styled("▍ ", theme::s(theme::accent()));
            let mut out = Vec::new();
            for l in text.split('\n') {
                out.extend(wrap(&tag_spans(l, theme::s(theme::text())), w, std::slice::from_ref(&bar), std::slice::from_ref(&bar)));
            }
            out
        }
        Item::Assistant(text) => {
            let shown = &text[..e.shown.min(text.len())];
            if shown.is_empty() {
                return vec![];
            }
            markdown::render_opts(shown, w, 2, !(e.expanded || expand_all))
        }
        Item::Note(text, kind) => {
            let (icon, st) = match kind {
                NoteKind::Info => ("· ", theme::s(theme::dim())),
                NoteKind::Error => ("✗ ", theme::s(theme::err())),
                NoteKind::Command => ("› ", theme::bold(theme::accent())),
            };
            wrap(&[Span::styled(text.clone(), st)], w, &[Span::raw("  "), Span::styled(icon, st)], &[Span::raw("    ")])
        }
        Item::Tool(t) => tool_lines(t, w, selected, e.expanded || expand_all, cwd),
    }
}

fn tool_meta(t: &Tool) -> String {
    if t.waiting {
        return "awaiting approval".into();
    }
    if t.state == ToolState::Running {
        let secs = t.started.elapsed().as_secs_f32();
        let steps = if t.sub_steps > 0 { format!("{} steps · ", t.sub_steps) } else { String::new() };
        return format!("{steps}{secs:.1}s");
    }
    match t.state {
        ToolState::Denied => return "denied".into(),
        ToolState::Stopped => return "stopped".into(),
        _ => {}
    }
    if let Some((a, d)) = tools::diff_stats(&t.name, &t.input) {
        return format!("+{a} −{d}");
    }
    let n = t.result.as_deref().map_or(0, |r| r.lines().filter(|l| !l.trim().is_empty()).count());
    match t.name.as_str() {
        "Read" => format!("{n} lines"),
        "Glob" => format!("{n} files"),
        "Grep" => format!("{n} matches"),
        "Task" | "Agent" if t.sub_steps > 0 => format!("{} steps", t.sub_steps),
        _ => t.took.map(|d| fmt_dur(d.as_secs_f32())).unwrap_or_default(),
    }
}

pub fn fmt_dur(secs: f32) -> String {
    if secs < 60.0 { format!("{secs:.1}s") } else { format!("{}m{:02}s", secs as u32 / 60, secs as u32 % 60) }
}

fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn tool_lines(t: &Tool, w: usize, selected: bool, expanded: bool, cwd: &Path) -> Vec<Line<'static>> {
    let (icon, icon_st) = match t.state {
        ToolState::Running if t.waiting => ("⚠", theme::s(theme::warn())),
        ToolState::Running => (anim::spinner(), theme::s(theme::accent())),
        ToolState::Ok => {
            let t = anim::progress(t.finished_at, anim::FLASH);
            ("✓", Style::new().fg(theme::mix(theme::flash_rgb(), theme::ok_rgb(), anim::ease_out(t))))
        }
        ToolState::Err => ("✗", theme::s(theme::err())),
        ToolState::Denied => ("⊘", theme::s(theme::warn())),
        ToolState::Stopped => ("■", theme::s(theme::dim())),
    };
    let (verb, target) = tools::summary(&t.name, &t.input, cwd);
    let meta = tool_meta(t);
    let marker = if selected { "›" } else { " " };
    let head = format!(" {marker}{icon} {verb:<7} ");
    let avail = w.saturating_sub(width(&head) + width(&meta) + 3);
    let target = truncate(&target, avail);
    let pad = w.saturating_sub(width(&head) + width(&target) + width(&meta) + 1);
    let bg = if selected { Style::new().bg(theme::sel_bg()) } else { Style::new() };
    let mut spans = vec![
        Span::styled(format!(" {marker}"), bg.fg(theme::accent())),
        Span::styled(icon.to_string(), icon_st.patch(bg)),
        Span::styled(format!(" {verb:<7} "), bg.fg(theme::accent_soft())),
        Span::styled(target, bg.fg(theme::text())),
        Span::styled(" ".repeat(pad), bg),
        Span::styled(meta, bg.fg(theme::dim())),
        Span::styled(" ", bg),
    ];
    if selected {
        spans[3].style = spans[3].style.add_modifier(Modifier::BOLD);
    }
    let mut out = vec![Line::from(spans)];

    let always_open = t.name == "TodoWrite";
    if !expanded && !always_open {
        if t.state == ToolState::Err
            && let Some(r) = &t.result
            && let Some(first) = r.lines().find(|l| !l.trim().is_empty())
        {
            out.push(body_line(&truncate(first.trim(), w.saturating_sub(8)), theme::s(theme::err()), w));
        }
        return out;
    }
    out.extend(tool_body(t, w));
    out
}

fn gutter() -> Vec<Span<'static>> {
    vec![Span::raw("     "), Span::styled("│ ", theme::s(theme::faint()))]
}

fn body_line(text: &str, st: Style, _w: usize) -> Line<'static> {
    let mut g = gutter();
    g.push(Span::styled(text.to_string(), st));
    Line::from(g)
}

fn body_wrapped(text: &str, st: Style, w: usize, out: &mut Vec<Line<'static>>, limit: usize) {
    let lines: Vec<&str> = text.lines().collect();
    for l in lines.iter().take(limit) {
        out.extend(wrap(&[Span::styled(l.replace('\t', "    "), st)], w, &gutter(), &gutter()));
    }
    if lines.len() > limit {
        out.push(body_line(&format!("… {} more lines", lines.len() - limit), theme::s(theme::dim()), w));
    }
}

fn tool_body(t: &Tool, w: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    let text_st = theme::s(theme::dim());
    match t.name.as_str() {
        "Edit" | "MultiEdit" | "Write" => {
            let pairs = tools::edit_pairs(&t.name, &t.input);
            let mut shown = 0;
            for (o, n) in &pairs {
                let d = tools::diff_lines(o, n);
                let keep: Vec<bool> = (0..d.len())
                    .map(|i| {
                        let lo = i.saturating_sub(2);
                        let hi = (i + 3).min(d.len());
                        d[lo..hi].iter().any(|(k, _)| *k != DiffKind::Same)
                    })
                    .collect();
                let mut skipped = false;
                for (i, (k, l)) in d.iter().enumerate() {
                    if !keep[i] {
                        skipped = true;
                        continue;
                    }
                    if skipped {
                        out.push(body_line("⋯", theme::s(theme::faint()), w));
                        skipped = false;
                    }
                    if shown >= MAX_BODY {
                        break;
                    }
                    shown += 1;
                    let (sign, st) = match k {
                        DiffKind::Add => ("+ ", Style::new().fg(theme::add_fg()).bg(theme::add_bg())),
                        DiffKind::Del => ("- ", Style::new().fg(theme::del_fg()).bg(theme::del_bg())),
                        DiffKind::Same => ("  ", theme::s(theme::dim())),
                    };
                    let content = format!("{sign}{}", l.replace('\t', "    "));
                    for mut line in wrap(&[Span::styled(content, st)], w, &gutter(), &gutter()) {
                        if *k != DiffKind::Same {
                            let lw: usize = line.spans.iter().map(|s| width(&s.content)).sum();
                            line.spans.push(Span::styled(" ".repeat(w.saturating_sub(lw)), Style::new().bg(st.bg.unwrap())));
                        }
                        out.push(line);
                    }
                }
            }
            if shown >= MAX_BODY {
                out.push(body_line("… diff truncated", theme::s(theme::dim()), w));
            }
            if t.state == ToolState::Err
                && let Some(r) = &t.result
            {
                body_wrapped(r, theme::s(theme::err()), w, &mut out, 6);
            }
        }
        "Bash" => {
            let cmd = tools::s(&t.input, "command");
            for (i, l) in cmd.lines().enumerate() {
                let p = if i == 0 { "$ " } else { "  " };
                out.extend(wrap(&[Span::styled(format!("{p}{l}"), theme::s(theme::text()))], w, &gutter(), &gutter()));
            }
            if let Some(r) = &t.result {
                let st = if t.state == ToolState::Err { theme::s(theme::err()) } else { text_st };
                body_wrapped(r.trim_end(), st, w, &mut out, MAX_BODY);
            }
        }
        "TodoWrite" => {
            if let Some(todos) = t.input.get("todos").and_then(Value::as_array) {
                for td in todos {
                    let status = tools::s(td, "status");
                    let (mark, st) = match status {
                        "completed" => ("☑ ", theme::s(theme::dim()).add_modifier(Modifier::CROSSED_OUT)),
                        "in_progress" => ("◐ ", theme::bold(theme::accent())),
                        _ => ("☐ ", theme::s(theme::text())),
                    };
                    let label = if status == "in_progress" {
                        let a = tools::s(td, "activeForm");
                        if a.is_empty() { tools::s(td, "content") } else { a }
                    } else {
                        tools::s(td, "content")
                    };
                    let mut first = gutter();
                    first.push(Span::styled(mark, st.remove_modifier(Modifier::CROSSED_OUT)));
                    let mut rest = gutter();
                    rest.push(Span::raw("  "));
                    out.extend(wrap(&[Span::styled(label.to_string(), st)], w, &first, &rest));
                }
            }
        }
        _ => {
            if t.input.as_object().is_some_and(|o| !o.is_empty()) && t.result.is_none() {
                body_wrapped(&serde_json::to_string_pretty(&t.input).unwrap_or_default(), text_st, w, &mut out, 20);
            }
            if let Some(r) = &t.result {
                let st = if t.state == ToolState::Err { theme::s(theme::err()) } else { text_st };
                body_wrapped(r.trim_end(), st, w, &mut out, MAX_BODY);
            }
        }
    }
    out
}

fn status_line(s: &Session) -> Option<Line<'static>> {
    if !s.busy() || !s.permissions.is_empty() {
        return None;
    }
    let tool_running = s
        .entries
        .last()
        .is_some_and(|e| matches!(&e.item, Item::Tool(t) if t.state == ToolState::Running));
    if tool_running {
        return None;
    }
    let label = match s.status {
        crate::session::Status::Starting => "starting",
        _ if s.thinking => "thinking",
        _ => "working",
    };
    let secs = s.turn_started.map_or(0.0, |t| t.elapsed().as_secs_f32());
    let mut spans = vec![
        Span::raw("  "),
        Span::styled(anim::spinner(), theme::s(theme::accent())),
        Span::raw(" "),
    ];
    spans.extend(anim::shimmer(label));
    spans.push(Span::styled(format!("  {}  ·  esc to stop", fmt_dur(secs)), theme::s(theme::faint())));
    Some(Line::from(spans))
}

/// Splits text so `[Image #1]` / `[Pasted text #2 …]` tags render as orange chips.
pub fn tag_spans(text: &str, base: Style) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut last = 0;
    for (s, e) in crate::input::tag_ranges(text) {
        if s > last {
            out.push(Span::styled(text[last..s].to_string(), base));
        }
        out.push(Span::styled(text[s..e].to_string(), theme::bold(theme::accent()).bg(theme::sel_bg())));
        last = e;
    }
    if last < text.len() {
        out.push(Span::styled(text[last..].to_string(), base));
    }
    out
}
