//! Running subagents as mini Claude Code windows.

use super::transcript::fmt_dur;
use super::wrap::width;
use crate::session::{Agent, AgentLine, AgentStatus, ToolState};
use crate::{anim, theme, tools};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Widget};
use std::path::Path;
use std::time::Duration;

/// Finished agents stay in the strip this long.
const LINGER: Duration = Duration::from_secs(6);

/// Agents worth showing in the strip above the input.
pub fn visible(agents: &[Agent]) -> Vec<&Agent> {
    agents
        .iter()
        .filter(|a| a.status == AgentStatus::Running || a.finished.is_some_and(|f| f.elapsed() < LINGER))
        .collect()
}

fn trunc(s: &str, max: usize) -> String {
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

fn fmt_tokens(n: u64) -> String {
    if n < 1000 { n.to_string() } else { format!("{:.1}k", n as f64 / 1e3) }
}

/// One mini window: header with type/task and timer, recent activity, footer.
pub fn card(buf: &mut Buffer, r: Rect, a: &Agent, cwd: &Path, hover: bool) {
    let elapsed = a.finished.unwrap_or_else(std::time::Instant::now).duration_since(a.started).as_secs_f32();
    let (border, status) = match a.status {
        AgentStatus::Running => (if hover { theme::dim() } else { theme::rule() }, format!("{} {}", anim::spinner(), fmt_dur(elapsed))),
        AgentStatus::Done => (theme::ok(), format!("✓ {}", fmt_dur(elapsed))),
        AgentStatus::Failed => (theme::err(), "✗ failed".to_string()),
    };
    let kind = if a.kind.is_empty() { "agent".to_string() } else { a.kind.clone() };
    let title_room = (r.width as usize).saturating_sub(width(&status) + 8);
    let full = if r.width < 40 { a.desc.clone() } else { format!("{kind} · {}", a.desc) };
    let title = trunc(&full, title_room);
    let bg = Style::new().bg(theme::panel_bg());
    Block::bordered()
        .border_type(BorderType::Rounded)
        .style(bg)
        .border_style(Style::new().fg(border).bg(theme::bg()))
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled("◆ ", theme::s(theme::accent())),
            Span::styled(title, theme::bold(theme::text())),
            Span::raw(" "),
        ]))
        .title(Line::from(Span::styled(format!(" {status} "), theme::s(theme::dim()))).right_aligned())
        .render(r, buf);
    let inner = Rect { x: r.x + 2, y: r.y + 1, width: r.width.saturating_sub(4), height: r.height.saturating_sub(2) };
    if inner.height == 0 {
        return;
    }
    let iw = inner.width as usize;
    let body_rows = inner.height.saturating_sub(1) as usize;

    let mut rows: Vec<Line> = Vec::new();
    for line in a.log.iter().rev().take(body_rows).collect::<Vec<_>>().into_iter().rev() {
        rows.push(match line {
            AgentLine::Tool { name, input, state, .. } => {
                let (icon, st) = match state {
                    ToolState::Running => (anim::spinner(), theme::s(theme::accent())),
                    ToolState::Ok => ("✓", theme::s(theme::ok())),
                    ToolState::Err | ToolState::Denied => ("✗", theme::s(theme::err())),
                    ToolState::Stopped => ("■", theme::s(theme::dim())),
                };
                let (verb, target) = tools::summary(name, input, cwd);
                Line::from(vec![
                    Span::styled(icon.to_string(), st),
                    Span::styled(format!(" {verb:<6} "), theme::s(theme::dim())),
                    Span::styled(trunc(&target, iw.saturating_sub(9)), theme::s(theme::text())),
                ])
            }
            AgentLine::Text(t) => {
                let first = t.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                Line::from(Span::styled(
                    trunc(first.trim(), iw),
                    theme::s(theme::dim()).add_modifier(Modifier::ITALIC),
                ))
            }
        });
    }
    if rows.is_empty() {
        let msg = if a.activity.is_empty() { "starting…".to_string() } else { a.activity.clone() };
        rows.push(Line::from(Span::styled(trunc(&msg, iw), theme::s(theme::dim()))));
    }
    for (k, l) in rows.iter().enumerate() {
        buf.set_line(inner.x, inner.y + k as u16, l, inner.width);
    }

    let footer = if a.status == AgentStatus::Running && !a.activity.is_empty() {
        Line::from(Span::styled(trunc(&a.activity, iw), theme::s(theme::accent_soft())))
    } else {
        let tools = if a.tools == 1 { "1 tool".to_string() } else { format!("{} tools", a.tools) };
        Line::from(Span::styled(
            trunc(&format!("{tools} · {} tokens", fmt_tokens(a.tokens)), iw),
            theme::s(theme::faint()),
        ))
    };
    buf.set_line(inner.x, inner.y + inner.height - 1, &footer, inner.width);
}

pub const SIDE_CARD_H: u16 = 6;

/// Mini windows stacked in the sidebar. Returns click targets.
pub fn column(buf: &mut Buffer, area: Rect, agents: &[&Agent], cwd: &Path, hover: Option<(u16, u16)>) -> Vec<Rect> {
    let mut hits = Vec::new();
    if area.height < SIDE_CARD_H || area.width < 16 {
        return hits;
    }
    let fit = (area.height / SIDE_CARD_H) as usize;
    let mut shown = agents.len().min(fit);
    // Make room for the "+N more" line, but always keep at least one card.
    if agents.len() > shown && area.height < shown as u16 * SIDE_CARD_H + 1 && shown > 1 {
        shown -= 1;
    }
    let overflow = agents.len() > shown && area.height > shown as u16 * SIDE_CARD_H;
    let mut y = area.y;
    for a in agents.iter().take(shown) {
        let r = Rect::new(area.x, y, area.width, SIDE_CARD_H);
        let hov = hover.is_some_and(|(x, yy)| r.contains((x, yy).into()));
        card(buf, r, a, cwd, hov);
        hits.push(r);
        y += SIDE_CARD_H;
    }
    if overflow {
        let r = Rect::new(area.x, y, area.width, 1);
        let more = format!(" +{} more agents · ^X", agents.len() - shown);
        buf.set_string(r.x, r.y, more, theme::s(theme::accent_soft()));
        hits.push(r);
    }
    hits
}

/// Full-screen grid of every agent in the session.
pub fn board(buf: &mut Buffer, area: Rect, agents: &[Agent], cwd: &Path) {
    let mut list: Vec<&Agent> = agents.iter().collect();
    list.sort_by_key(|a| (a.status != AgentStatus::Running, std::cmp::Reverse(a.started)));
    let cols = if area.width >= 90 { 2 } else { 1 };
    let card_h = 11u16;
    let gap = 1u16;
    let w = (area.width - gap * (cols - 1)) / cols;
    if list.is_empty() {
        buf.set_string(area.x + 2, area.y + 1, "No subagents in this session yet.", theme::s(theme::dim()));
        buf.set_string(
            area.x + 2,
            area.y + 2,
            "When Claude launches agents (the Agent tool), they appear here live.",
            theme::s(theme::faint()),
        );
        return;
    }
    for (i, a) in list.iter().enumerate() {
        let (c, row) = (i as u16 % cols, i as u16 / cols);
        let y = area.y + row * (card_h + gap);
        if y + card_h > area.bottom() {
            break;
        }
        card(buf, Rect::new(area.x + c * (w + gap), y, w, card_h), a, cwd, false);
    }
}
