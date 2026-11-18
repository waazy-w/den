pub mod agents;
pub mod markdown;
pub mod panels;
pub mod companion;
pub mod transcript;
pub mod tutorial;
pub mod wrap;

use crate::app::{App, Focus, Modal, display_dir, switcher_hits};
use crate::session::{Session, Status};
use crate::theme;
use crate::tools;
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Widget};
use crate::anim;
use transcript::fmt_dur;
use wrap::{width, wrap};

const SIDEBAR_W: u16 = 28;
const HEADER_H: u16 = 1;
/// Widest a conversation pane gets; beyond this lines get hard to read.
const MAX_PANE_W: u16 = 150;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    if theme::p().bg.is_some() {
        f.buffer_mut().set_style(area, Style::new().bg(theme::bg()).fg(theme::text()));
    }
    let show_sidebar = app.sidebar && area.width >= 72;
    let [side, main] = if show_sidebar {
        Layout::horizontal([Constraint::Length(SIDEBAR_W), Constraint::Min(20)]).areas(area)
    } else {
        [Rect::default(), area]
    };
    app.sidebar_rows.clear();
    app.agent_hits.clear();
    app.sidebar_visible = show_sidebar;
    if show_sidebar {
        draw_sidebar(f.buffer_mut(), side, app);
    }
    app.pane_hits.clear();
    let panes = app.panes();
    let rule = theme::s(theme::rule());
    let split_at = if panes.len() == 2 && main.width >= crate::app::SPLIT_MIN_W {
        let [l, r] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(main);
        for (pane, i) in [(l, panes[0]), (r, panes[1])] {
            if i != app.active {
                app.pane_hits.push((pane, crate::app::PaneHit::Focus(i)));
            }
        }
        draw_main(f, l, app, panes[0]);
        draw_main(f, r, app, panes[1]);
        Some(r.x)
    } else {
        draw_main(f, main, app, app.active);
        None
    };
    if main.height > 2 {
        f.buffer_mut().set_string(main.x, main.y + HEADER_H, "─".repeat(main.width as usize), rule);
    }
    if let Some(x) = split_at {
        for yy in main.y..main.bottom() {
            f.buffer_mut().set_string(x, yy, "│", rule);
        }
        f.buffer_mut().set_string(x, main.y + HEADER_H, "┼", rule);
    }
    if show_sidebar {
        let dx = side.x + side.width - 1;
        f.buffer_mut().set_string(dx, side.y + HEADER_H, "┼", rule);
    }
    apply_selection(f.buffer_mut(), app);
    app.board_hits.clear();
    if let Some(crate::app::Modal::Board(b)) = &app.modal {
        app.board_hits = board_hits(area, b, app);
    }
    app.settings_hits = if matches!(app.modal, Some(crate::app::Modal::Settings { .. })) {
        panels::settings_rows(area)
    } else {
        Vec::new()
    };
    if let Some(m) = &app.modal {
        let t = anim::ease_out(anim::progress(app.modal_at, anim::OPEN));
        // Dim everything behind the panel so it reads as a layer on top.
        theme::fade(f.buffer_mut(), area, 1.0 - 0.6 * t);
        let rise = ((1.0 - t) * 2.0).round() as u16;
        let shifted = Rect { y: area.y + rise, ..area };
        draw_modal(f, shifted, m, app);
    }
}

fn model_label(m: Option<&str>) -> String {
    let Some(m) = m else { return String::new() };
    let m = m.strip_prefix("claude-").unwrap_or(m);
    let m = m.split('[').next().unwrap_or(m);
    let mut parts = m.split('-');
    let Some(family) = parts.next() else { return m.into() };
    let ver: Vec<&str> = parts.take_while(|p| p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit())).collect();
    let mut fam = family.to_string();
    if let Some(c) = fam.get_mut(0..1) {
        c.make_ascii_uppercase();
    }
    if ver.is_empty() { fam } else { format!("{fam} {}", ver.join(".")) }
}

fn status_glyph(s: &Session) -> (String, Style) {
    if !s.permissions.is_empty() {
        return ("⚠".into(), theme::s(theme::warn()));
    }
    match s.status {
        Status::Working => (anim::spinner().into(), theme::s(theme::accent())),
        Status::Starting => (anim::spinner().into(), theme::s(theme::dim())),
        Status::Idle if s.unread => {
            ("●".into(), Style::new().fg(theme::mix(theme::faint_rgb(), theme::ok_rgb(), anim::pulse(s.unread_since))))
        }
        Status::Idle => (String::new(), Style::new()),
    }
}

fn truncate_left(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut w = 1;
    for c in chars.iter().rev() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        out.insert(0, *c);
        w += cw;
    }
    format!("…{out}")
}

fn truncate(s: &str, max: usize) -> String {
    if width(s) <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 1;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if w + cw > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

pub fn project_label(app: &App, p: &std::path::Path) -> String {
    let all: Vec<&std::path::Path> = app.sessions.iter().map(|s| s.cwd.as_path()).collect();
    crate::projects::label(p, &all)
}

fn draw_sidebar(buf: &mut Buffer, area: Rect, app: &mut App) {
    let inner_w = area.width.saturating_sub(5) as usize;
    let x = area.x + 2;
    let total = app.sessions.len();
    buf.set_string(x, area.y, "sessions", theme::bold(theme::dim()));
    let n = total.to_string();
    buf.set_string(x + inner_w as u16 - n.len() as u16, area.y, &n, theme::s(theme::dim()));
    buf.set_string(area.x, area.y + HEADER_H, "─".repeat(area.width as usize - 1), theme::s(theme::rule()));
    let mut y = area.y + HEADER_H + 2;
    let max_y = area.y + area.height.saturating_sub(4);
    let mut i = 0;
    while i < app.sessions.len() {
        let cwd = app.sessions[i].cwd.clone();
        let end = (i..app.sessions.len()).find(|&j| app.sessions[j].cwd != cwd).unwrap_or(app.sessions.len());
        if y >= max_y {
            break;
        }
        let here = (i..end).contains(&app.active);
        let label = truncate(&project_label(app, &cwd), inner_w.saturating_sub(3));
        let head_st = if here { theme::bold(theme::text()) } else { theme::bold(theme::dim()) };
        buf.set_line(x, y, &Line::from(Span::styled(label, head_st)), inner_w as u16);
        let n = end - i;
        let plus_x = x + inner_w as u16 - 1;
        // Only the "+" starts a session; the name just switches to the project.
        let plus_rect = Rect::new(plus_x.saturating_sub(1), y, 3, 1);
        let plus_hover = hovered(app.hover, plus_rect);
        if n > 1 {
            let c = n.to_string();
            buf.set_string(plus_x - 2 - c.len() as u16, y, &c, theme::s(theme::faint()));
        }
        let plus_st = if plus_hover { theme::bold(theme::accent()).bg(theme::hi_bg()) } else { theme::s(theme::dim()) };
        buf.set_string(plus_x.saturating_sub(1), y, " + ", plus_st);
        app.sidebar_rows.push((plus_rect, crate::app::SideHit::NewIn(cwd.clone())));
        let target = if here { app.active } else { i };
        app.sidebar_rows.push((Rect::new(area.x, y, area.width, 1), crate::app::SideHit::Session(target)));
        y += 1;
        for j in i..end {
            if y >= max_y {
                break;
            }
            let s = &app.sessions[j];
            let active = j == app.active;
            let shown = !active && app.in_split(j);
            let activity = s.activity().filter(|_| y + 1 < max_y);
            let row_h = if activity.is_some() { 2 } else { 1 };
            let row = Rect::new(area.x, y, area.width.saturating_sub(1), row_h);
            let row_hover = hovered(app.hover, row);
            let close_rect = Rect::new(x + inner_w as u16 - 2, y, 3, 1);
            let (glyph, gst) = if row_hover {
                let on_x = hovered(app.hover, close_rect);
                ("×".to_string(), if on_x { theme::bold(theme::err()) } else { theme::s(theme::dim()) })
            } else {
                status_glyph(s)
            };
            if row_hover {
                buf.set_style(row, Style::new().bg(theme::sel_bg()));
            }
            let name = truncate(&s.name, inner_w.saturating_sub(4));
            let pad = inner_w.saturating_sub(2 + width(&name) + width(&glyph));
            let (bar, name_st) = if active {
                (Span::styled("▍ ", theme::s(theme::accent())), theme::bold(theme::text()))
            } else if shown {
                (Span::styled("▍ ", theme::s(theme::faint())), theme::s(theme::text()))
            } else {
                (Span::raw("  "), if row_hover { theme::s(theme::text()) } else { theme::s(theme::dim()) })
            };
            let bg = if row_hover { Style::new().bg(theme::sel_bg()) } else { Style::new() };
            let (bar, name_st, gst) = (bar.patch_style(bg), name_st.patch(bg), gst.patch(bg));
            buf.set_line(
                x,
                y,
                &Line::from(vec![bar, Span::styled(name, name_st), Span::raw(" ".repeat(pad)), Span::styled(glyph, gst)]),
                inner_w as u16,
            );
            app.sidebar_rows.push((close_rect, crate::app::SideHit::Close(j)));
            app.sidebar_rows.push((Rect::new(area.x, y, area.width, row_h), crate::app::SideHit::Session(j)));
            y += 1;
            if let Some((text, kind)) = activity {
                use crate::session::ActivityKind;
                let fg = match kind {
                    ActivityKind::Waiting => theme::warn(),
                    ActivityKind::Busy => theme::accent_soft(),
                    ActivityKind::Idle => theme::faint(),
                };
                let st = Style::new().fg(fg).patch(bg);
                buf.set_string(x + 2, y, truncate(&text, inner_w.saturating_sub(2)), st);
                y += 1;
            }
        }
        i = end;
        if i < app.sessions.len() && y + 1 < max_y {
            buf.set_string(x, y, "─".repeat(inner_w), theme::s(theme::rule()));
            y += 1;
        }
    }
    y = draw_tasks(buf, Rect::new(x, y, inner_w as u16, max_y.saturating_sub(y)), app);
    let foot = area.y + area.height.saturating_sub(1);
    let button_y = foot;
    let projects_y = foot.saturating_sub(1);
    let settings_y = foot.saturating_sub(2);
    let update_y = foot.saturating_sub(3);
    // Topmost footer button; Clawd and the agents sit above it.
    let board_y = if app.update_ready.is_some() { update_y } else { settings_y };
    let room_top = y + 1;
    let clawd_rows = (board_y.saturating_sub(1 + room_top)).min(companion::H + 3);
    let clawd_top = board_y.saturating_sub(1 + clawd_rows);
    {
        let s = &app.sessions[app.active];
        let shown = agents::visible(&s.agents);
        if !shown.is_empty() {
            // Agents get the space down to Clawd; he steps aside if they need it.
            let need = shown.len() as u16 * agents::SIDE_CARD_H;
            let bottom = if clawd_top >= room_top + need { clawd_top } else { board_y.saturating_sub(1) };
            let col = Rect::new(area.x + 1, room_top, area.width.saturating_sub(3), bottom.saturating_sub(room_top));
            app.agent_hits = agents::column(buf, col, &shown, &s.cwd, app.hover);
        }
    }
    let agents_busy = app.agent_hits.last().is_some_and(|r| r.bottom() > clawd_top);
    if board_y > room_top + companion::H && !agents_busy {
        let rows = clawd_rows;
        let lane = Rect::new(x, board_y - 1 - rows, inner_w as u16, rows);
        let t = if app.idle_secs() > companion::DEEP_SLEEP_AFTER { 0.0 } else { crate::anim::secs() };
        companion::draw(buf, lane, app.clawd_mood(), t, app.reaction_now(), app.outfit());
    }
    let mut button = |by: u16, label: &str, key: &str, hit: crate::app::SideHit| {
        if by <= y {
            return;
        }
        let text = format!(" {label}{:>w$}", format!("{key} "), w = inner_w.saturating_sub(width(label) + 1));
        let r = Rect::new(x, by, inner_w as u16, 1);
        let st = if hovered(app.hover, r) { theme::bold(theme::accent()).bg(theme::hi_bg()) } else { theme::s(theme::dim()) };
        buf.set_string(x, by, &text, st);
        app.sidebar_rows.push((Rect::new(area.x, by, area.width, 1), hit));
    };
    if let Some(tag) = app.update_ready.clone() {
        button(update_y, &format!("↑ {tag} ready"), "reload", crate::app::SideHit::Reload);
    }
    button(settings_y, "settings", "^S", crate::app::SideHit::Settings);
    button(projects_y, "projects", "^G", crate::app::SideHit::Board);
    button(button_y, "+ new session", "^N", crate::app::SideHit::NewButton);
    for yy in area.y..area.y + area.height {
        buf.set_string(area.x + area.width - 1, yy, "│", theme::s(theme::rule()));
    }
}

/// Claude's task list and the files-changed row for the active session.
/// Returns the next free row.
fn draw_tasks(buf: &mut Buffer, area: Rect, app: &mut App) -> u16 {
    use crate::session::TodoState;
    let s = &app.sessions[app.active];
    let mut y = area.y;
    let bottom = area.bottom();
    let w = area.width as usize;
    let open = s.todos.iter().any(|t| t.state != TodoState::Done);
    if open && y + 4 <= bottom {
        y += 1;
        buf.set_string(area.x, y, "─".repeat(w), theme::s(theme::rule()));
        y += 1;
        let done = s.todos.iter().filter(|t| t.state == TodoState::Done).count();
        buf.set_string(area.x, y, "tasks", theme::bold(theme::dim()));
        let n = format!("{done}/{}", s.todos.len());
        buf.set_string(area.x + (w - n.len()) as u16, y, &n, theme::s(theme::dim()));
        y += 1;
        let room = (bottom.saturating_sub(y + 2) as usize).min(8);
        let first_open = s.todos.iter().position(|t| t.state != TodoState::Done).unwrap_or(0);
        let start = first_open.saturating_sub(1).min(s.todos.len().saturating_sub(room));
        for t in s.todos.iter().skip(start).take(room) {
            let (icon, ist, tst, text) = match t.state {
                TodoState::Done => ("✓", theme::s(theme::ok()), theme::s(theme::faint()).add_modifier(Modifier::CROSSED_OUT), &t.text),
                TodoState::Active => (
                    anim::spinner(),
                    theme::s(theme::accent()),
                    theme::s(theme::text()),
                    if t.active_form.is_empty() { &t.text } else { &t.active_form },
                ),
                TodoState::Pending => ("○", theme::s(theme::faint()), theme::s(theme::dim()), &t.text),
            };
            buf.set_string(area.x, y, icon, ist);
            buf.set_string(area.x + 2, y, truncate(text, w.saturating_sub(2)), tst);
            y += 1;
        }
        let hidden = s.todos.len().saturating_sub(start + room);
        if hidden > 0 && y < bottom {
            buf.set_string(area.x + 2, y, format!("+{hidden} more"), theme::s(theme::faint()));
            y += 1;
        }
    }
    let all_done = !open && !s.todos.is_empty();
    if all_done && y + 3 <= bottom {
        y += 1;
        buf.set_string(area.x, y, "─".repeat(w), theme::s(theme::rule()));
        y += 1;
        buf.set_string(area.x, y, "tasks", theme::bold(theme::dim()));
        let n = format!("✓ all {} done", s.todos.len());
        buf.set_string(area.x + (w.saturating_sub(width(&n))) as u16, y, &n, theme::s(theme::ok()));
        y += 1;
    }
    if !s.changes.is_empty() && y + 3 <= bottom {
        y += 1;
        if !open && !all_done {
            buf.set_string(area.x, y, "─".repeat(w), theme::s(theme::rule()));
            y += 1;
        }
        let (a, r) = s.changes.iter().fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
        let n = s.changes.len();
        let label = format!("± {n} file{}", if n == 1 { "" } else { "s" });
        let counts = format!("+{a} −{r}");
        let rect = Rect::new(area.x.saturating_sub(1), y, area.width + 2, 1);
        let hov = hovered(app.hover, rect);
        let bg = if hov { Style::new().bg(theme::hi_bg()) } else { Style::new() };
        if hov {
            buf.set_style(rect, bg);
        }
        buf.set_string(area.x, y, &label, if hov { theme::bold(theme::accent()).patch(bg) } else { theme::s(theme::dim()) });
        let cx = area.x + (w.saturating_sub(width(&counts))) as u16;
        let plus = format!("+{a}");
        buf.set_string(cx, y, &plus, theme::s(theme::add_fg()).patch(bg));
        buf.set_string(cx + width(&plus) as u16 + 1, y, format!("−{r}"), theme::s(theme::del_fg()).patch(bg));
        app.sidebar_rows.push((rect, crate::app::SideHit::Files));
        y += 1;
    }
    y
}

/// One conversation pane: header, transcript, approvals, input and status.
/// `idx` is the session shown; only the active one owns the input.
fn draw_main(f: &mut Frame, area: Rect, app: &mut App, idx: usize) {
    // On very wide screens keep the conversation a readable column, centered.
    let area = if area.width > MAX_PANE_W {
        Rect { x: area.x + (area.width - MAX_PANE_W) / 2, width: MAX_PANE_W, ..area }
    } else {
        area
    };
    let area = Rect { x: area.x + 2, width: area.width.saturating_sub(3), ..area };
    if area.width < 10 || area.height < 6 {
        return;
    }
    let mine = idx == app.active;
    let s = &app.sessions[idx];

    let input_w = area.width.saturating_sub(6) as usize;
    let (rows, cursor) = if mine { app.editor.layout(input_w) } else { (vec![String::new()], (0, 0)) };
    let visible_rows = rows.len().clamp(1, 8);
    let input_h = visible_rows as u16 + 2;
    let mut approval = approval_lines(s, area.width as usize);
    let queue_base = approval.len();
    let qw = area.width as usize;
    for (q, _) in s.queue.iter().take(3) {
        let first = q.lines().next().unwrap_or("");
        // Leave room on the right for the buttons drawn later.
        approval.push(Line::from(vec![
            Span::styled("  ↳ queued  ", theme::s(theme::accent_soft())),
            Span::styled(truncate(first, qw.saturating_sub(12 + 18)), theme::s(theme::dim())),
        ]));
    }
    if s.queue.len() > 3 {
        approval.push(Line::from(Span::styled(format!("    +{} more queued", s.queue.len() - 3), theme::s(theme::faint()))));
    }
    let slide = anim::ease_out(anim::progress(s.perm_since, anim::OPEN));
    let approval_h = (approval.len() as f32 * slide).ceil() as u16;

    let [header, header_rule, body, appr, input, status] = Layout::vertical([
        Constraint::Length(HEADER_H),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(approval_h),
        Constraint::Length(input_h),
        Constraint::Length(1),
    ])
    .areas(area);

    if let Some(hit) = draw_header(f.buffer_mut(), header, app, idx) {
        app.pane_hits.push(hit);
    }
    let _ = header_rule;
    let label = project_label(app, &app.sessions[idx].cwd);
    if mine {
        app.page = body.height as usize;
    }
    let app_text_rect = body;
    let clawd = (app.clawd_mood(), crate::anim::secs(), app.reaction_now(), app.outfit());
    let fx = app.prefs.typing_effect;
    let s = &mut app.sessions[idx];
    if s.entries.is_empty() {
        draw_welcome(f.buffer_mut(), body, s, &label, clawd);
    } else {
        transcript::draw(f.buffer_mut(), body, s, app.expand_all, fx);
    }
    let scrolled = s.scroll;
    if mine {
        app.text_rect = app_text_rect;
        let switch_t = anim::ease_out(anim::progress(app.switched_at, anim::SWITCH));
        theme::fade(f.buffer_mut(), header.union(body), switch_t);
    }
    if scrolled > 0 {
        let label = format!(" ↓ {scrolled} lines below · ^L ");
        let x = body.x + body.width.saturating_sub(width(&label) as u16);
        f.buffer_mut().set_string(x, body.y + body.height - 1, label, theme::s(theme::dim()).bg(theme::sel_bg()));
    }

    let skip = approval.len().saturating_sub(approval_h as usize);
    for (i, l) in approval.iter().skip(skip).enumerate() {
        f.buffer_mut().set_line(appr.x, appr.y + i as u16, l, appr.width);
    }
    // "send now" and "×" on each queued message.
    if mine {
        let n = app.sessions[idx].queue.len().min(3);
        for k in 0..n {
            let Some(row) = (queue_base + k).checked_sub(skip) else { continue };
            let y = appr.y + row as u16;
            let (sr, dr) = queue_buttons(appr, y, appr.width as usize);
            let buf = f.buffer_mut();
            let send_st = if hovered(app.hover, sr) { theme::bold(theme::on_accent()).bg(theme::accent()) } else { theme::s(theme::accent()) };
            let drop_st = if hovered(app.hover, dr) { theme::bold(theme::err()).bg(theme::hi_bg()) } else { theme::s(theme::faint()) };
            buf.set_string(sr.x, y, " ⏎ send now ", send_st);
            buf.set_string(dr.x, y, " × ", drop_st);
            app.pane_hits.push((sr, crate::app::PaneHit::SendQueued(k)));
            app.pane_hits.push((dr, crate::app::PaneHit::DropQueued(k)));
        }
    }
    theme::fade(f.buffer_mut(), appr, slide);

    let s = &app.sessions[idx];
    let focused = mine && app.focus == Focus::Input && app.modal.is_none();
    let mode = mode_style(&s.mode);
    let border = if !s.permissions.is_empty() || !focused {
        theme::sel_bg()
    } else if let Some((rgb, _, _)) = mode {
        if s.busy() {
            theme::mix(theme::p().rule, rgb, 0.6 + 0.4 * (anim::secs() * 2.2).sin())
        } else {
            theme::mix(rgb, rgb, 1.0)
        }
    } else if s.busy() {
        let glow = 0.55 + 0.3 * (anim::secs() * 2.2).sin();
        theme::mix(theme::faint_rgb(), theme::accent_rgb(), glow * 0.6)
    } else if mine && !app.editor.is_empty() {
        theme::dim()
    } else {
        theme::faint()
    };
    let shell = mine && app.editor.text.starts_with('!');
    let mode = if shell { Some(((0xF4, 0x72, 0xB6), "!", "shell mode")) } else { mode };
    let border = if shell { theme::mix((0xF4, 0x72, 0xB6), (0xF4, 0x72, 0xB6), 1.0) } else { border };
    let mut block = Block::new()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::s(border));
    if let Some((rgb, icon, label)) = mode {
        let c = theme::mix(rgb, rgb, 1.0);
        block = block
            .title(Line::from(vec![
                Span::raw(" "),
                Span::styled(format!("{icon} {label}"), Style::new().fg(c).add_modifier(Modifier::BOLD)),
                Span::raw(" "),
            ]))
            .title(
                Line::from(Span::styled(
                    if shell { " runs here · output goes to Claude " } else { " ⇧tab to change " },
                    theme::s(theme::faint()),
                ))
                .right_aligned(),
            );
    }
    let inner = block.inner(input);
    block.render(input, f.buffer_mut());
    let first_row = cursor.0.saturating_sub(visible_rows - 1);
    let prompt_st = if focused { theme::bold(theme::accent()) } else { theme::s(theme::dim()) };
    f.buffer_mut().set_string(inner.x + 1, inner.y, "›", prompt_st);
    if !mine || app.editor.is_empty() {
        let ph = if !mine {
            "click to type here  ·  ^\\ switch pane"
        } else if !s.queue.is_empty() {
            "⏎ send queued now  ·  ↑ edit it  ·  or keep typing"
        } else if !s.permissions.is_empty() {
            "answer the request above"
        } else if s.busy() {
            "queue a follow-up…"
        } else if s.entries.is_empty() {
            "ask anything · @ to mention files · / for commands"
        } else {
            "reply…"
        };
        f.buffer_mut().set_string(inner.x + 3, inner.y, ph, theme::s(theme::faint()));
    } else {
        for (i, r) in rows.iter().skip(first_row).take(visible_rows).enumerate() {
            let line = Line::from(transcript::tag_spans(r, theme::s(theme::text())));
            f.buffer_mut().set_line(inner.x + 3, inner.y + i as u16, &line, inner.width.saturating_sub(3));
        }
    }
    if focused {
        f.set_cursor_position(Position::new(
            inner.x + 3 + cursor.1 as u16,
            inner.y + (cursor.0 - first_row) as u16,
        ));
    }

    let hit = draw_status(f.buffer_mut(), status, app, idx);
    if mine {
        app.model_hit = hit;
        if app.popup.is_some() {
            draw_popup(f, input, app);
        }
    }
}

/// Where a queued message's "send now" and "×" buttons sit on row `y`.
fn queue_buttons(area: Rect, y: u16, w: usize) -> (Rect, Rect) {
    let right = area.x + w as u16;
    let drop = Rect::new(right.saturating_sub(3), y, 3, 1);
    let send = Rect::new(right.saturating_sub(3 + 1 + 12), y, 12, 1);
    (send, drop)
}

/// Folder name, mode badge and the split/close-pane button.
fn draw_header(buf: &mut Buffer, area: Rect, app: &App, idx: usize) -> Option<(Rect, crate::app::PaneHit)> {
    let s = &app.sessions[idx];
    let mine = idx == app.active;
    let split = app.split.is_some();
    let title_st = if mine || !split { theme::bold(theme::accent()) } else { theme::bold(theme::dim()) };
    let mut left = vec![Span::styled(project_label(app, &s.cwd), title_st)];
    if split {
        left.push(Span::styled(format!("  {}", s.name), theme::s(theme::faint())));
    }
    let button = if split { " × " } else { " ◫ split " };
    let bw = width(button) as u16;
    let mut right: Vec<Span> = Vec::new();
    if let Some((rgb, icon, label)) = mode_style(&s.mode) {
        let mut badge = vec![
            Span::styled(format!(" {icon} {} ", label.to_uppercase()), Style::new().fg(theme::on_accent()).bg(theme::mix(rgb, rgb, 1.0)).add_modifier(Modifier::BOLD)),
            Span::raw("   "),
        ];
        badge.extend(right);
        right = badge;
    }
    let rw: usize = right.iter().map(|s| width(&s.content)).sum::<usize>() + bw as usize + 1;
    buf.set_line(area.x, area.y, &Line::from(left), area.width.saturating_sub(rw as u16 + 2));
    let rx = area.x + area.width.saturating_sub(rw as u16);
    buf.set_line(rx, area.y, &Line::from(right), rw as u16);
    let br = Rect::new(area.x + area.width.saturating_sub(bw), area.y, bw, 1);
    let st = if hovered(app.hover, br) { theme::bold(theme::accent()).bg(theme::hi_bg()) } else { theme::s(theme::faint()) };
    buf.set_string(br.x, br.y, button, st);
    let hit = if split { crate::app::PaneHit::Close(idx) } else { crate::app::PaneHit::Split };
    Some((br, hit))
}

type ClawdState = (companion::Mood, f32, Option<(companion::Reaction, f32)>, Option<companion::Outfit>);

fn draw_welcome(buf: &mut Buffer, area: Rect, s: &Session, label: &str, clawd: ClawdState) {
    let sprite_rows = companion::H + 4;
    let text = [
        Line::from(Span::styled("what are we building?", theme::bold(theme::text()))),
        Line::default(),
        Line::from(Span::styled(label.to_string(), theme::bold(theme::accent()))),
        Line::default(),
        Line::from(Span::styled(
            if s.claude_id.is_some() { "your next message resumes this conversation" } else { "/help for keys  ·  ^N for another project" },
            theme::s(theme::faint()),
        )),
    ];
    let total = sprite_rows + 1 + text.len() as u16;
    let top = area.y + area.height.saturating_sub(total) / 2;
    let sx = area.x + area.width.saturating_sub(companion::W) / 2;
    let rows = (companion::H + 3).min(top + sprite_rows - area.y);
    let lane = Rect::new(sx.saturating_sub(3), top + sprite_rows - rows, companion::W + 6, rows);
    companion::draw(buf, lane, clawd.0, clawd.1, clawd.2, clawd.3);
    for (i, l) in text.iter().enumerate() {
        let w = l.width() as u16;
        let x = area.x + area.width.saturating_sub(w) / 2;
        buf.set_line(x, top + sprite_rows + 1 + i as u16, l, area.width);
    }
}

fn approval_lines(s: &Session, w: usize) -> Vec<Line<'static>> {
    let Some(p) = s.permissions.front() else { return vec![] };
    let mut out = vec![Line::default()];
    let (verb, target) = tools::summary(&p.tool_name, &p.input, &s.cwd);
    let queued = if s.permissions.len() > 1 { format!("  +{} more", s.permissions.len() - 1) } else { String::new() };
    out.push(Line::from(vec![
        Span::styled(" ⚠ ", theme::bold(theme::warn())),
        Span::styled(ask_phrase(&p.tool_name, &verb), theme::bold(theme::text())),
        Span::styled(queued, theme::s(theme::dim())),
    ]));
    let pre = [Span::raw("   ")];
    let detail = match p.tool_name.as_str() {
        "Bash" => tools::s(&p.input, "command").to_string(),
        "Edit" | "MultiEdit" | "Write" => {
            let stats = tools::diff_stats(&p.tool_name, &p.input)
                .map(|(a, d)| format!("   +{a} −{d}"))
                .unwrap_or_default();
            format!("{target}{stats}")
        }
        _ => target,
    };
    let mut shown = 0;
    let pairs = tools::edit_pairs(&p.tool_name, &p.input);
    if !pairs.is_empty() {
        out.push(Line::from(vec![Span::raw("   "), Span::styled(detail.clone(), theme::s(theme::text()))]));
        let mut n = 0;
        let mut total = 0;
        for (o, nw) in &pairs {
            for (k, l) in tools::diff_lines(o, nw) {
                let (sign, st) = match k {
                    tools::DiffKind::Add => ("+ ", Style::new().fg(theme::add_fg()).bg(theme::add_bg())),
                    tools::DiffKind::Del => ("- ", Style::new().fg(theme::del_fg()).bg(theme::del_bg())),
                    tools::DiffKind::Same => continue,
                };
                total += 1;
                if n < 6 {
                    let text = truncate(&format!("{sign}{}", l.replace('\t', "    ")), w.saturating_sub(5));
                    let pad = w.saturating_sub(5 + width(&text));
                    out.push(Line::from(vec![
                        Span::raw("   "),
                        Span::styled("│ ", theme::s(theme::faint())),
                        Span::styled(text, st),
                        Span::styled(" ".repeat(pad), Style::new().bg(st.bg.unwrap())),
                    ]));
                    n += 1;
                }
            }
        }
        if total > n {
            out.push(Line::from(Span::styled(format!("   │ … {} more changed lines", total - n), theme::s(theme::dim()))));
        }
    }
    for l in detail.lines().filter(|_| pairs.is_empty()) {
        for wl in wrap(&[Span::styled(l.to_string(), theme::s(theme::text()))], w, &pre, &pre) {
            if shown < 5 {
                out.push(wl);
            }
            shown += 1;
        }
    }
    if shown > 5 {
        out.push(Line::from(Span::styled(format!("   … {} more lines", shown - 5), theme::s(theme::dim()))));
    }
    let key = |k: &'static str, label: &'static str, st: Style| {
        [Span::styled(format!(" {k} "), st.add_modifier(Modifier::BOLD).bg(theme::sel_bg())), Span::styled(format!(" {label}   "), theme::s(theme::dim()))]
    };
    let mut keys = vec![Span::raw("   ")];
    keys.extend(key("y", "allow", theme::s(theme::ok())));
    keys.extend(key("a", "always", theme::s(theme::accent())));
    keys.extend(key("n", "deny", theme::s(theme::err())));
    out.push(Line::default());
    out.push(Line::from(keys));
    out
}

fn ask_phrase(tool: &str, verb: &str) -> String {
    let what = match tool {
        "Bash" => "run a command",
        "Edit" | "MultiEdit" => "edit a file",
        "Write" => "write a file",
        "Read" => "read a file",
        "NotebookEdit" => "edit a notebook",
        "WebFetch" => "fetch a page",
        "WebSearch" => "search the web",
        "Task" | "Agent" => "start an agent",
        _ => return format!("Claude wants to use {verb}"),
    };
    format!("Claude wants to {what}")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}


/// `5h ━━━──── 18% · 2h 14m   week ━────── 5%`
fn usage_spans(limits: &[crate::store::Limit]) -> Vec<Span<'static>> {
    let now = now_secs();
    let mut out = Vec::new();
    for l in limits.iter().take(3) {
        let label = match l.key.as_str() {
            "five_hour" => "5h".to_string(),
            "seven_day" => "week".to_string(),
            "seven_day_opus" => "opus wk".to_string(),
            "seven_day_sonnet" => "sonnet wk".to_string(),
            k => k.replace('_', " "),
        };
        let reset = l.resets_at > 0 && now >= l.resets_at;
        let pct = if reset { 0.0 } else { (l.utilization * 100.0).clamp(0.0, 100.0) };
        let fg = if pct >= 95.0 {
            theme::err()
        } else if pct >= 80.0 {
            theme::warn()
        } else {
            theme::accent()
        };
        let cells = 10usize;
        let filled = ((pct / 100.0) * cells as f32).round() as usize;
        let filled = if pct > 0.0 { filled.max(1) } else { 0 };
        if !out.is_empty() {
            out.push(Span::raw("    "));
        }
        out.push(Span::styled(format!("{label} "), theme::s(theme::dim())));
        out.push(Span::styled("━".repeat(filled), theme::s(fg)));
        out.push(Span::styled("─".repeat(cells - filled), theme::s(theme::faint())));
        out.push(Span::styled(format!(" {pct:.0}%"), theme::s(theme::text())));
    }
    out
}

/// Context bar, token counts, working time and model, right under the input.
/// Returns where the model name sits (click it to switch).
fn draw_status(buf: &mut Buffer, area: Rect, app: &App, idx: usize) -> Rect {
    let mine = idx == app.active;
    if let Some((msg, at)) = app.toast.as_ref().filter(|_| mine) {
        let age = at.elapsed().as_secs_f32();
        let t = if age < 0.15 { age / 0.15 } else { ((3.0 - age) / 0.5).clamp(0.0, 1.0) };
        let fg = theme::mix(theme::faint_rgb(), theme::warn_rgb(), anim::ease_out(t));
        buf.set_line(area.x + 1, area.y, &Line::from(Span::styled(msg.clone(), Style::new().fg(fg))), area.width);
        return Rect::default();
    }
    if mine && app.focus == Focus::Transcript {
        let hint = "↑↓ select   ⏎ expand   ^O expand all   esc back";
        buf.set_string(area.x + 1, area.y, hint, theme::s(theme::dim()));
        return Rect::default();
    }
    let s = &app.sessions[idx];
    let mut spans = vec![Span::raw(" ")];
    let gauge = |label: &str, pct: f32, spans: &mut Vec<Span<'static>>| {
        spans.push(Span::styled(format!("{label} "), theme::s(theme::dim())));
        spans.extend(bar(pct, 8));
        spans.push(Span::styled(format!(" {:.0}%", pct.clamp(0.0, 100.0)), theme::s(theme::text())));
        spans.push(Span::raw("   "));
    };
    gauge("ctx", s.ctx_shown, &mut spans);
    let now = now_secs();
    for l in app.lifetime.limits.iter().take(2) {
        let label = match l.key.as_str() {
            "five_hour" => "5h",
            "seven_day" => "week",
            _ => "plan",
        };
        let pct = if l.resets_at > 0 && now >= l.resets_at { 0.0 } else { l.utilization * 100.0 };
        gauge(label, pct, &mut spans);
        // Near the limit: say when it frees up.
        if pct >= 80.0
            && let Some(r) = crate::app::reset_in(l.resets_at, now)
        {
            spans.pop();
            let c = if pct >= 95.0 { theme::err() } else { theme::warn() };
            spans.push(Span::styled(format!(" ↻{r}"), theme::s(c)));
            spans.push(Span::raw("   "));
        }
    }
    spans.push(Span::styled(fmt_tokens(s.usage.tok_in + s.usage.tok_out), theme::s(theme::text())));
    spans.push(Span::styled(" tokens", theme::s(theme::dim())));
    let live = s.turn_started.filter(|_| s.busy()).map_or(0.0, |t| t.elapsed().as_secs_f32());
    let secs = s.usage.working_ms as f32 / 1000.0 + live;
    if secs > 0.0 {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(fmt_dur(secs), theme::s(theme::text())));
    }
    let mut model = model_label(s.model.as_deref());
    if model.is_empty() {
        model = "model".into();
    }
    if let Some(e) = &s.effort {
        model = format!("{model} · {e}");
    }
    let mw = width(&model) as u16;
    let room = area.width.saturating_sub(mw + 3) as usize;
    if spans.iter().map(|s| width(&s.content)).sum::<usize>() > room {
        spans.retain(|s| s.content != " tokens");
    }
    buf.set_line(area.x, area.y, &Line::from(spans), room as u16);
    let r = Rect::new(area.x + area.width.saturating_sub(mw + 1), area.y, mw, 1);
    let st = if hovered(app.hover, r) { theme::s(theme::accent()).bg(theme::hi_bg()) } else { theme::s(theme::dim()) };
    buf.set_string(r.x, r.y, model, st);
    r
}

/// Color, icon and label for non-default permission modes.
pub fn mode_style(mode: &str) -> Option<(theme::Rgb, &'static str, &'static str)> {
    match mode {
        "auto" => Some(((0xFF, 0x8A, 0x3D), "⏵⏵", "auto mode")),
        "acceptEdits" => Some(((0xA7, 0x8B, 0xFA), "⏵⏵", "accept edits")),
        "plan" => Some(((0x2D, 0xD4, 0xBF), "⏸", "plan mode")),
        "bypassPermissions" => Some(((0xF8, 0x71, 0x71), "⏵⏵", "bypass")),
        _ => None,
    }
}

fn hovered(at: Option<(u16, u16)>, r: Rect) -> bool {
    at.is_some_and(|(x, y)| r.contains((x, y).into()))
}

pub fn fmt_tokens(n: u64) -> String {
    match n {
        0..1_000 => n.to_string(),
        1_000..1_000_000 => format!("{:.1}k", n as f64 / 1e3),
        _ => format!("{:.2}M", n as f64 / 1e6),
    }
}

fn draw_popup(f: &mut Frame, input: Rect, app: &App) {
    let Some(p) = &app.popup else { return };
    let total = p.items.len();
    let visible = total.min(10);
    let offset = (p.sel + 1).saturating_sub(visible);
    let w = input.width.min(76);
    let h = visible as u16 + 2;
    let y = input.y.saturating_sub(h);
    let rect = Rect::new(input.x, y, w, h);
    f.render_widget(Clear, rect);
    let mut block = Block::bordered()
        .border_type(BorderType::Rounded)
        .style(Style::new().bg(theme::panel_bg()))
        .border_style(Style::new().fg(theme::dim()).bg(theme::panel_bg()));
    if total > visible {
        block = block.title_bottom(
            Line::from(Span::styled(format!(" {}/{} ", p.sel + 1, total), theme::s(theme::dim()))).right_aligned(),
        );
    }
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let buf = f.buffer_mut();
    let prefix = match p.kind {
        crate::app::PopupKind::Slash => "/",
        crate::app::PopupKind::File | crate::app::PopupKind::Dir => "",
    };
    let name_w = p.items.iter().map(|(n, _)| width(n) + 1).max().unwrap_or(0).min(28);
    for (row, (i, (name, desc))) in p.items.iter().enumerate().skip(offset).take(visible).enumerate() {
        let sel = i == p.sel;
        let bg = if sel { Style::new().bg(theme::sel_bg()) } else { Style::new() };
        let name = truncate(&format!("{prefix}{name}"), name_w.max(4));
        let fg = if sel { theme::accent() } else { theme::text() };
        let gap = name_w.saturating_sub(width(&name)) + 2;
        let rem = (inner.width as usize).saturating_sub(1 + width(&name) + gap + 1);
        let d = truncate(desc, rem);
        let pad = rem.saturating_sub(width(&d));
        let spans = vec![
            Span::styled(" ", bg),
            Span::styled(name, bg.fg(fg)),
            Span::styled(" ".repeat(gap), bg),
            Span::styled(d, bg.fg(theme::dim())),
            Span::styled(" ".repeat(pad + 1), bg),
        ];
        buf.set_line(inner.x, inner.y + row as u16, &Line::from(spans), inner.width);
    }
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h)
}

/// A raised panel: solid surface, orange frame, bold title.
fn modal_block(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .style(Style::new().bg(theme::panel_bg()))
        .border_style(Style::new().fg(theme::accent()).bg(theme::panel_bg()))
        .title(Line::from(vec![Span::raw(" "), Span::styled(title, theme::bold(theme::accent())), Span::raw(" ")]))
}

#[allow(clippy::explicit_counter_loop)]
fn draw_modal(f: &mut Frame, area: Rect, m: &Modal, app: &App) {
    match m {
        Modal::Tutorial { step } => tutorial::draw(f, area, *step, anim::ease_out(anim::progress(app.step_at, anim::SWITCH))),
        Modal::Project { query, sel } => {
            let choices = app.project_choices();
            let picks = crate::projects::picker_with(&query.text, &choices, &app.all_workspaces());
            let rows = picks.len().clamp(1, 14) as u16;
            let rect = centered(area, 72, rows + 7);
            f.render_widget(Clear, rect);
            let b = modal_block("new session");
            let inner = b.inner(rect);
            f.render_widget(b, rect);
            let buf = f.buffer_mut();
            buf.set_string(inner.x + 1, inner.y, "›", theme::bold(theme::accent()));
            if query.is_empty() {
                let hint = if app.all_workspaces().is_empty() {
                    "pick a project, type a path, or a new name…"
                } else {
                    "pick a project, or type a name to create one…"
                };
                buf.set_string(inner.x + 3, inner.y, hint, theme::s(theme::faint()));
            } else {
                buf.set_string(inner.x + 3, inner.y, truncate_left(&query.text, inner.width as usize - 4), theme::s(theme::text()));
            }
            let cur = app.current().cwd.clone();
            let open: Vec<&std::path::Path> = app.sessions.iter().map(|s| s.cwd.as_path()).collect();
            let all: Vec<&std::path::Path> = choices.iter().map(|p| p.as_path()).collect();
            let iw = inner.width as usize;
            let mut y = inner.y + 2;
            if picks.is_empty() {
                buf.set_string(inner.x + 3, y, "no matching projects", theme::s(theme::dim()));
            }
            let visible = (inner.height.saturating_sub(4)) as usize;
            let offset = (*sel + 1).saturating_sub(visible.max(1));
            for (i, pk) in picks.iter().enumerate().skip(offset) {
                if y >= inner.y + inner.height.saturating_sub(2) {
                    break;
                }
                let selected = i == *sel;
                let bg = if selected { Style::new().bg(theme::sel_bg()) } else { Style::new() };
                let (name, path, tag) = match pk {
                    crate::projects::Pick::Open(p) => {
                        let ok = p.is_dir();
                        (if ok { "open folder".to_string() } else { "no such folder".to_string() }, display_dir(p), String::new())
                    }
                    crate::projects::Pick::Create(p) => {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        (format!("+ create {name}"), display_dir(p.parent().unwrap_or(p)), "new folder".to_string())
                    }
                    crate::projects::Pick::Project(p) => {
                        let n = open.iter().filter(|o| **o == p.as_path()).count();
                        let ws = p.parent().filter(|pp| app.all_workspaces().iter().any(|w| w == pp));
                        let tag = if *p == cur {
                            "current".to_string()
                        } else if n > 0 {
                            format!("{n} open")
                        } else if let Some(w) = ws {
                            w.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
                        } else {
                            String::new()
                        };
                        (crate::projects::label(p, &all), display_dir(p), tag)
                    }
                };
                let name = truncate(&name, 22);
                let tagw = width(&tag);
                let path = truncate_left(&path, iw.saturating_sub(31 + tagw));
                let used = 3 + 24 + width(&path) + tagw + 2;
                let pad = iw.saturating_sub(used);
                let nfg = if selected { theme::accent() } else { theme::text() };
                buf.set_line(
                    inner.x,
                    y,
                    &Line::from(vec![
                        Span::styled("   ", bg),
                        Span::styled(format!("{name:<24}"), bg.fg(nfg)),
                        Span::styled(path, bg.fg(theme::dim())),
                        Span::styled(" ".repeat(pad), bg),
                        Span::styled(tag, bg.fg(theme::faint())),
                        Span::styled("  ", bg),
                    ]),
                    inner.width,
                );
                y += 1;
            }
            buf.set_line(
                inner.x + 1,
                inner.y + inner.height - 1,
                &Line::from(vec![
                    Span::styled("⏎", theme::s(theme::dim())),
                    Span::styled(" start   ", theme::s(theme::faint())),
                    Span::styled("tab", theme::s(theme::dim())),
                    Span::styled(" complete path   ", theme::s(theme::faint())),
                    Span::styled("/ws add", theme::s(theme::dim())),
                    Span::styled(" folder of projects   ", theme::s(theme::faint())),
                    Span::styled("esc", theme::s(theme::dim())),
                    Span::styled(" cancel", theme::s(theme::faint())),
                ]),
                inner.width,
            );
            let cx = width(&query.text[..query.cursor]).min(inner.width as usize - 4);
            f.set_cursor_position(Position::new(inner.x + 3 + cx as u16, inner.y));
        }
        Modal::Switcher { query, sel } => {
            let hits = switcher_hits(&app.sessions, &query.text);
            let h = (hits.len() as u16 + 5).min(area.height.saturating_sub(4));
            let rect = centered(area, 60, h);
            f.render_widget(Clear, rect);
            let b = modal_block("sessions");
            let inner = b.inner(rect);
            f.render_widget(b, rect);
            let buf = f.buffer_mut();
            buf.set_string(inner.x + 1, inner.y, "›", theme::bold(theme::accent()));
            if query.is_empty() {
                buf.set_string(inner.x + 3, inner.y, "filter…", theme::s(theme::faint()));
            } else {
                buf.set_string(inner.x + 3, inner.y, &query.text, theme::s(theme::text()));
            }
            let mut y = inner.y + 2;
            for i in hits {
                if y + 1 >= inner.y + inner.height {
                    break;
                }
                let s = &app.sessions[i];
                let selected = i == *sel;
                let bg = if selected { Style::new().bg(theme::sel_bg()) } else { Style::new() };
                let (glyph, gst) = status_glyph(s);
                let num = if i < 9 { format!("{} ", i + 1) } else { "  ".into() };
                let proj = truncate(&project_label(app, &s.cwd), 18);
                let name = truncate(&s.name, (inner.width as usize).saturating_sub(width(&proj) + 12));
                let pad = (inner.width as usize).saturating_sub(width(&num) + width(&name) + width(&proj) + 6);
                buf.set_line(
                    inner.x,
                    y,
                    &Line::from(vec![
                        Span::styled(" ", bg),
                        Span::styled(num, bg.fg(theme::faint())),
                        Span::styled(name, bg.fg(if selected { theme::accent() } else { theme::text() })),
                        Span::styled(" ".repeat(pad), bg),
                        Span::styled(proj, bg.fg(theme::dim())),
                        Span::styled(" ", bg),
                        Span::styled(format!("{glyph:<1}"), gst.patch(bg)),
                        Span::styled(if glyph.is_empty() { "  " } else { " " }, bg),
                    ]),
                    inner.width,
                );
                y += 1;
            }
            let cx = width(&query.text[..query.cursor]);
            f.set_cursor_position(Position::new(inner.x + 3 + cx as u16, inner.y));
        }
        Modal::Stats => draw_stats(f, area, app),
        Modal::Agents => {
            let rect = centered(area, area.width.saturating_sub(6), area.height.saturating_sub(4));
            f.render_widget(Clear, rect);
            let s = app.current();
            let running = s.agents.iter().filter(|a| a.status == crate::session::AgentStatus::Running).count();
            let title = format!("agents · {running} running · {} total", s.agents.len());
            let b = modal_block(&title);
            let inner = b.inner(rect);
            f.render_widget(b, rect);
            let grid = Rect { x: inner.x + 1, y: inner.y + 1, width: inner.width.saturating_sub(2), height: inner.height.saturating_sub(2) };
            agents::board(f.buffer_mut(), grid, &s.agents, &s.cwd);
            f.buffer_mut().set_string(inner.x + 2, inner.bottom().saturating_sub(1), "esc close", theme::s(theme::faint()));
        }
        Modal::Board(b) => draw_board(f, area, b, app),
        Modal::Model { sel, effort } => panels::model(f, area, app, *sel, *effort),
        Modal::Theme { sel, .. } => panels::theme(f, area, *sel),
        Modal::Settings { sel } => panels::settings(f, area, app, *sel),
        Modal::Files { sel, scroll } => panels::files(f, area, app, *sel, *scroll),
        Modal::Search { query, sel } => panels::search(f, area, app, query, *sel),
        Modal::Help => {
            let rows: &[(&str, &str)] = &[
                ("⏎ / ⇧⏎ / ⌥⏎", "send · newline"),
                ("esc", "interrupt Claude / close popup"),
                ("y  a  n", "allow · always · deny a tool"),
                ("^T", "browse tool calls (⏎ expands)"),
                ("^X  /agents", "see running subagents"),
                ("^F  /search", "search every conversation"),
                ("^\\  /split", "two sessions side by side"),
                ("⌥M  /model", "model and thinking effort"),
                ("/files", "files Claude changed, with diffs"),
                ("^S  /settings", "theme, typing effect, notifications"),
                ("/theme", "pick a color theme"),
                ("⇧tab  /mode", "permission mode · auto mode"),
                ("drag  /copy", "copy text · copy last reply"),
                ("^O", "expand / collapse all tools"),
                ("PgUp PgDn ⇧↑↓", "scroll · ^L jump to bottom"),
                ("^N  ^P", "new session (pick project) · switch"),
                ("⌥1-9  ⌥↑↓", "jump · cycle sessions"),
                ("^D", "close session (empty input)"),
                ("^B", "toggle sidebar"),
                ("^C ^C  /  ^Q", "quit"),
                ("", ""),
                ("/new /close /rename", "session commands"),
                ("/clear /model", "fresh start · switch model"),
                ("/tutorial", "replay the intro tour"),
            ];
            let rect = centered(area, 58, rows.len() as u16 + 4);
            f.render_widget(Clear, rect);
            let b = modal_block("keys");
            let inner = b.inner(rect);
            f.render_widget(b, rect);
            let lines: Vec<Line> = rows
                .iter()
                .map(|(k, d)| {
                    Line::from(vec![
                        Span::styled(format!(" {k:<22}"), theme::s(theme::accent())),
                        Span::styled(*d, theme::s(theme::dim())),
                    ])
                })
                .collect();
            f.render_widget(Paragraph::new(lines), inner.inner(ratatui::layout::Margin::new(0, 1)));
        }
    }
}

/// Highlights the mouse selection and, once released, copies its text.
fn apply_selection(buf: &mut Buffer, app: &mut App) {
    let Some((a, b)) = app.selection else { return };
    let (start, end) = if (a.1, a.0) <= (b.1, b.0) { (a, b) } else { (b, a) };
    let r = app.text_rect;
    let mut text = String::new();
    for y in start.1.max(r.y)..=end.1.min(r.bottom().saturating_sub(1)) {
        let x0 = if y == start.1 { start.0 } else { r.x };
        let x1 = if y == end.1 { end.0 } else { r.right().saturating_sub(1) };
        let mut line = String::new();
        for x in x0.max(r.x)..=x1.min(r.right().saturating_sub(1)) {
            if let Some(cell) = buf.cell_mut((x, y)) {
                line.push_str(cell.symbol());
                cell.set_bg(theme::hi_bg());
            }
        }
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(line.trim_end());
    }
    if app.copy_pending {
        app.copy_pending = false;
        app.selection = None;
        let cleaned: String = text
            .lines()
            .map(|l| l.trim_start_matches(['▍', '│']).trim_start_matches(' '))
            .collect::<Vec<_>>()
            .join("\n");
        app.copy_text(&cleaned);
    }
}

fn draw_stats(f: &mut Frame, area: Rect, app: &App) {
    use crate::session::Usage;
    let rows_n = app.sessions.len().min(10) as u16;
    let rect = centered(area, 86, rows_n + 17);
    f.render_widget(Clear, rect);
    let b = modal_block("stats");
    let inner = b.inner(rect);
    f.render_widget(b, rect);
    let buf = f.buffer_mut();
    let x = inner.x + 2;
    let w = inner.width.saturating_sub(4);
    let cols = |name: &str, u: &Usage, st: Style| -> Line<'static> {
        let n = truncate(name, 24);
        Line::from(vec![
            Span::styled(format!("{n:<25}"), st),
            Span::styled(format!("{:>8}", fmt_tokens(u.tok_in)), theme::s(theme::text())),
            Span::styled(format!("{:>8}", fmt_tokens(u.tok_out)), theme::s(theme::text())),
            Span::styled(format!("{:>9}", format!("${:.2}", u.cost)), theme::s(theme::text())),
            Span::styled(format!("{:>9}", fmt_dur(u.working_ms as f32 / 1000.0)), theme::s(theme::text())),
            Span::styled(format!("{:>7}", u.turns), theme::s(theme::dim())),
        ])
    };
    let mut y = inner.y + 1;
    let head = Line::from(vec![
        Span::styled(format!("{:<25}", "session"), theme::bold(theme::accent())),
        Span::styled(format!("{:>8}{:>8}{:>9}{:>9}{:>7}", "in", "out", "cost", "time", "turns"), theme::bold(theme::accent_soft())),
    ]);
    buf.set_line(x, y, &head, w);
    y += 1;
    buf.set_string(x, y, "─".repeat(w as usize), theme::s(theme::rule()));
    y += 1;
    let mut open = Usage::default();
    for (i, s) in app.sessions.iter().enumerate() {
        open.add(&s.usage);
        if i < 10 {
            let st = if i == app.active { theme::bold(theme::accent()) } else { theme::s(theme::text()) };
            buf.set_line(x, y, &cols(&s.name, &s.usage, st), w);
            y += 1;
        }
    }
    buf.set_string(x, y, "─".repeat(w as usize), theme::s(theme::rule()));
    y += 1;
    buf.set_line(x, y, &cols("all open sessions", &open, theme::bold(theme::text())), w);
    y += 2;
    let since = app.lifetime.since.clone().unwrap_or_default();
    let life = &app.lifetime.usage;
    buf.set_line(x, y, &Line::from(Span::styled(format!("lifetime  ·  since {since}"), theme::bold(theme::accent()))), w);
    y += 1;
    buf.set_line(x, y, &cols(&format!("{} sessions started", app.lifetime.sessions), life, theme::s(theme::dim())), w);
    y += 2;
    let usage = usage_spans(&app.lifetime.limits);
    if !usage.is_empty() {
        let mut l = vec![Span::styled("plan usage   ", theme::bold(theme::accent()))];
        l.extend(usage);
        buf.set_line(x, y, &Line::from(l), w);
        y += 2;
    }
    let (runs, before, after) = crate::squeeze::savings();
    let saved = if runs > 0 {
        format!("~{} tokens saved across {runs} commands ({:.0}% smaller)", fmt_tokens(before.saturating_sub(after) / 4), 100.0 * (1.0 - after as f64 / before.max(1) as f64))
    } else if crate::squeeze::rtk_active() {
        "RTK is squeezing command output for you".to_string()
    } else {
        "nothing squeezed yet".to_string()
    };
    buf.set_line(x, y, &Line::from(vec![Span::styled("squeezer     ", theme::bold(theme::accent())), Span::styled(saved, theme::s(theme::text()))]), w);
    y += 2;
    let total = life.tok_in + life.tok_out;
    let avg = if life.turns > 0 { life.cost / life.turns as f64 } else { 0.0 };
    buf.set_line(
        x,
        y,
        &Line::from(vec![
            Span::styled(fmt_tokens(total), theme::bold(theme::accent())),
            Span::styled(" tokens total   ", theme::s(theme::dim())),
            Span::styled(format!("${avg:.3}"), theme::bold(theme::accent())),
            Span::styled(" per turn   ", theme::s(theme::dim())),
            Span::styled("esc", theme::bold(theme::accent())),
            Span::styled(" close", theme::s(theme::faint())),
        ]),
        w,
    );
}

fn bar(pct: f32, cells: usize) -> Vec<Span<'static>> {
    let pct = pct.clamp(0.0, 100.0);
    let filled = ((pct / 100.0) * cells as f32).round() as usize;
    let filled = if pct > 0.0 { filled.max(1).min(cells) } else { 0 };
    let fg = if pct >= 95.0 {
        theme::err()
    } else if pct >= 80.0 {
        theme::warn()
    } else {
        theme::accent()
    };
    vec![Span::styled("━".repeat(filled), theme::s(fg)), Span::styled("─".repeat(cells - filled), theme::s(theme::faint()))]
}

fn board_geom(area: Rect) -> (Rect, Rect, Rect) {
    let rect = centered(area, area.width.saturating_sub(8).min(110), area.height.saturating_sub(4).min(30));
    let inner = Rect { x: rect.x + 1, y: rect.y + 1, width: rect.width.saturating_sub(2), height: rect.height.saturating_sub(2) };
    let left = Rect { width: 28.min(inner.width / 3), height: inner.height.saturating_sub(1), ..inner };
    let right = Rect {
        x: left.x + left.width + 1,
        width: inner.width.saturating_sub(left.width + 1),
        height: inner.height.saturating_sub(1),
        ..inner
    };
    (rect, left, right)
}

const BOARD_LIST_TOP: u16 = 4;

fn board_hits(area: Rect, b: &crate::app::Board, app: &App) -> Vec<(Rect, crate::app::BoardHit)> {
    use crate::app::BoardHit;
    let (_, left, right) = board_geom(area);
    let mut hits = Vec::new();
    let folders = app.board_folders();
    for i in 0..folders.len() {
        let y = left.y + 2 + i as u16;
        if y < left.bottom().saturating_sub(2) {
            hits.push((Rect::new(left.x, y, left.width, 1), BoardHit::Folder(i)));
        }
    }
    hits.push((Rect::new(left.x + 1, left.bottom().saturating_sub(1), 16, 1), BoardHit::AddFolder));
    if b.folder > 0 {
        hits.push((Rect::new(right.x + 1, right.y + 2, 16, 1), BoardHit::NewProject));
        hits.push((Rect::new(right.x + 19, right.y + 2, 16, 1), BoardHit::RemoveFolder));
    }
    let projects = app.board_projects(b.folder, &b.filter);
    let visible = right.height.saturating_sub(BOARD_LIST_TOP) as usize;
    let offset = (b.sel + 1).saturating_sub(visible.max(1));
    for (row, i) in (offset..projects.len()).take(visible).enumerate() {
        hits.push((Rect::new(right.x, right.y + BOARD_LIST_TOP + row as u16, right.width, 1), BoardHit::Project(i)));
    }
    hits
}

fn ago(p: &std::path::Path) -> String {
    let Ok(m) = std::fs::metadata(p).and_then(|m| m.modified()) else { return String::new() };
    let secs = m.elapsed().map_or(0, |d| d.as_secs());
    match secs {
        0..3600 => "just now".into(),
        3600..86400 => format!("{}h ago", secs / 3600),
        86400..172800 => "yesterday".into(),
        172800..1209600 => format!("{}d ago", secs / 86400),
        _ => format!("{}w ago", secs / 604800),
    }
}

fn draw_board(f: &mut Frame, area: Rect, b: &crate::app::Board, app: &App) {
    use crate::app::{BoardFocus, BoardInput};
    let (rect, left, right) = board_geom(area);
    f.render_widget(Clear, rect);
    f.render_widget(modal_block("projects"), rect);
    let buf = f.buffer_mut();
    let button = |buf: &mut Buffer, x: u16, y: u16, label: &str| {
        let r = Rect::new(x, y, width(label) as u16, 1);
        let st = if hovered(app.hover, r) {
            theme::bold(theme::accent()).bg(theme::hi_bg())
        } else {
            theme::s(theme::text()).bg(theme::sel_bg())
        };
        buf.set_string(x, y, label, st);
    };

    // folders
    buf.set_string(left.x + 1, left.y, "FOLDERS", theme::bold(theme::dim()));
    let folders = app.board_folders();
    for (i, (name, _, n)) in folders.iter().enumerate() {
        let y = left.y + 2 + i as u16;
        if y >= left.bottom().saturating_sub(2) {
            break;
        }
        let sel = i == b.folder;
        let r = Rect::new(left.x, y, left.width, 1);
        let bg = if sel && b.focus == BoardFocus::Folders {
            Style::new().bg(theme::sel_bg())
        } else if hovered(app.hover, r) {
            Style::new().bg(theme::hi_bg())
        } else {
            Style::new()
        };
        let bar_span = if sel { Span::styled("▍", theme::s(theme::accent())) } else { Span::styled(" ", bg) };
        let count = n.to_string();
        let name = truncate(name, (left.width as usize).saturating_sub(count.len() + 4));
        let pad = (left.width as usize).saturating_sub(2 + width(&name) + count.len() + 1);
        let name_st = if sel { theme::bold(theme::text()) } else { theme::s(theme::dim()) };
        buf.set_line(
            left.x,
            y,
            &Line::from(vec![
                bar_span,
                Span::styled(" ", bg),
                Span::styled(name, name_st.patch(bg)),
                Span::styled(" ".repeat(pad), bg),
                Span::styled(count, theme::s(theme::faint()).patch(bg)),
                Span::styled(" ", bg),
            ]),
            left.width,
        );
    }
    button(buf, left.x + 1, left.bottom().saturating_sub(1), " + add folder ");
    for yy in left.y..left.bottom() {
        buf.set_string(left.right(), yy, "│", theme::s(theme::faint()));
    }

    // projects
    let all_ws = app.all_workspaces();
    let (title, path) = match b.folder.checked_sub(1).and_then(|i| all_ws.get(i)) {
        Some(w) => (folders[b.folder].0.clone(), display_dir(w)),
        None => ("Recent".to_string(), "recent projects".to_string()),
    };
    buf.set_line(
        right.x + 1,
        right.y,
        &Line::from(vec![
            Span::styled(title, theme::bold(theme::accent())),
            Span::raw("  "),
            Span::styled(truncate_left(&path, right.width as usize / 2), theme::s(theme::dim())),
        ]),
        right.width,
    );
    if b.folder > 0 {
        button(buf, right.x + 1, right.y + 2, " + new project ");
        button(buf, right.x + 19, right.y + 2, " remove folder ");
    } else if all_ws.is_empty() {
        buf.set_string(right.x + 1, right.y + 2, "tip: + add folder to manage a folder full of projects", theme::s(theme::faint()));
    }
    if !b.filter.is_empty() {
        let label = format!("filter: {}", b.filter);
        let x = right.right().saturating_sub(width(&label) as u16 + 1);
        buf.set_string(x, right.y + 2, label, theme::s(theme::accent_soft()));
    }

    let projects = app.board_projects(b.folder, &b.filter);
    let all: Vec<&std::path::Path> = projects.iter().map(|p| p.as_path()).collect();
    let visible = right.height.saturating_sub(BOARD_LIST_TOP) as usize;
    let offset = (b.sel + 1).saturating_sub(visible.max(1));
    if projects.is_empty() {
        buf.set_string(right.x + 2, right.y + BOARD_LIST_TOP, "no projects here yet", theme::s(theme::dim()));
    }
    for (row, (i, p)) in projects.iter().enumerate().skip(offset).take(visible).enumerate() {
        let y = right.y + BOARD_LIST_TOP + row as u16;
        let r = Rect::new(right.x, y, right.width, 1);
        let sel = i == b.sel && b.focus == BoardFocus::Projects;
        let bg = if sel {
            Style::new().bg(theme::sel_bg())
        } else if hovered(app.hover, r) {
            Style::new().bg(theme::hi_bg())
        } else {
            Style::new()
        };
        let open = app.sessions.iter().filter(|s| s.cwd == *p).count();
        let meta = if open > 0 { format!("{open} open  ·  {}", ago(p)) } else { ago(p) };
        let name = truncate(&crate::projects::label(p, &all), 24);
        let path_w = (right.width as usize).saturating_sub(30 + width(&meta) + 2);
        let pth = truncate_left(&display_dir(p), path_w);
        let pad = (right.width as usize).saturating_sub(2 + 26 + width(&pth) + width(&meta) + 2);
        buf.set_line(
            right.x,
            y,
            &Line::from(vec![
                if sel { Span::styled("▍", theme::s(theme::accent())) } else { Span::styled(" ", bg) },
                Span::styled(" ", bg),
                Span::styled(format!("{name:<26}"), if sel { theme::bold(theme::text()).patch(bg) } else { theme::s(theme::text()).patch(bg) }),
                Span::styled(pth, theme::s(theme::faint()).patch(bg)),
                Span::styled(" ".repeat(pad), bg),
                Span::styled(meta, theme::s(if open > 0 { theme::accent_soft() } else { theme::dim() }).patch(bg)),
                Span::styled("  ", bg),
            ]),
            right.width,
        );
    }

    // input line or key hints
    let foot = rect.bottom().saturating_sub(1);
    if let Some((kind, e)) = &b.input {
        let prompt = match kind {
            BoardInput::AddFolder => " folder of projects › ",
            BoardInput::NewProject => " new project name › ",
            BoardInput::Filter => " filter › ",
        };
        let line = Line::from(vec![
            Span::styled(prompt, theme::bold(theme::accent()).bg(theme::sel_bg())),
            Span::styled(format!("{} ", e.text), theme::s(theme::text()).bg(theme::sel_bg())),
        ]);
        let x = rect.x + 2;
        buf.set_line(x, foot, &line, rect.width.saturating_sub(4));
        let cx = x + width(prompt) as u16 + width(&e.text[..e.cursor]) as u16;
        f.set_cursor_position(Position::new(cx, foot));
    } else {
        let keys: [(&str, &str); 7] =
            [("↑↓", "move"), ("←→", "switch"), ("⏎", "open"), ("n", "new project"), ("a", "add folder"), ("/", "filter"), ("esc", "close")];
        let mut spans = vec![Span::raw(" ")];
        for (k, l) in keys {
            spans.push(Span::styled(k, theme::s(theme::dim())));
            spans.push(Span::styled(format!(" {l}  "), theme::s(theme::faint())));
        }
        buf.set_line(rect.x + 2, foot, &Line::from(spans), rect.width.saturating_sub(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_labels() {
        assert_eq!(model_label(Some("claude-opus-5-5")), "Opus 5.5");
        assert_eq!(model_label(Some("claude-haiku-5-5")), "Haiku 5.5");
        assert_eq!(model_label(Some("claude-sonnet-4-5-20250929")), "Sonnet 4.5");
        assert_eq!(model_label(None), "");
    }
}
