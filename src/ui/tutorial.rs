//! The first-run tour: a "Welcome to den" title card, then five short steps,
//! each with a small live demo that plays as the step opens.

use super::companion;
use super::wrap::{width, wrap};
use crate::{anim, theme};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear};

pub const STEPS: usize = 6;

/// Bump to show the tour again to everyone once (e.g. after it changes).
pub const VERSION: u32 = 2;

fn d(t: &str) -> Span<'static> {
    Span::styled(t.to_string(), theme::s(theme::dim()))
}
fn f(t: &str) -> Span<'static> {
    Span::styled(t.to_string(), theme::s(theme::faint()))
}
fn tx(t: &str) -> Span<'static> {
    Span::styled(t.to_string(), theme::s(theme::text()))
}
fn ac(t: &str) -> Span<'static> {
    Span::styled(t.to_string(), theme::s(theme::accent()))
}
fn key(k: &str) -> Span<'static> {
    Span::styled(format!(" {k} "), theme::bold(theme::text()).bg(theme::sel_bg()))
}

/// The same 5×7 letters as the brand wordmark.
const LETTERS: [[&str; 7]; 3] = [
    ["....#", "....#", ".####", "#...#", "#...#", "#...#", ".####"],
    [".....", ".....", ".###.", "#...#", "#####", "#....", ".####"],
    [".....", ".....", "####.", "#...#", "#...#", "#...#", "#...#"],
];

/// Big pixel "den", revealed column by column as `p` goes 0 → 1.
fn wordmark(buf: &mut Buffer, x: u16, y: u16, p: f32) {
    let total_cols = 3 * 6 - 1;
    let shown = (p * total_cols as f32).ceil() as usize;
    for (li, letter) in LETTERS.iter().enumerate() {
        for (row, line) in letter.iter().enumerate() {
            for (col, px) in line.chars().enumerate() {
                let gx = li * 6 + col;
                if px == '#' && gx < shown {
                    // Two cells wide per pixel keeps the letters square.
                    let fresh = gx + 2 >= shown && p < 1.0;
                    let c = if fresh { theme::accent() } else { theme::text() };
                    buf.set_string(x + gx as u16 * 2, y + row as u16, "  ", Style::new().bg(c));
                }
            }
        }
    }
}

fn reveal(text: &str, e: f32, cps: f32) -> String {
    let n = ((e.max(0.0) * cps) as usize).min(text.chars().count());
    text.chars().take(n).collect()
}

fn demo_talk(e: f32) -> Vec<Line<'static>> {
    let typed = reveal("fix the flaky test in @tests/api.rs", e - 0.2, 30.0);
    let caret = if ((e * 2.0) as u32).is_multiple_of(2) { "▍" } else { " " };
    let mut spans = vec![f("│ "), Span::styled("› ", theme::bold(theme::accent()))];
    match typed.split_once('@') {
        Some((a, b)) => {
            spans.push(tx(a));
            spans.push(ac(&format!("@{b}")));
        }
        None => spans.push(tx(&typed)),
    }
    spans.push(ac(caret));
    let used: usize = spans.iter().map(|x| width(&x.content)).sum();
    spans.push(Span::raw(" ".repeat(47usize.saturating_sub(used))));
    spans.push(f("│"));
    vec![
        Line::from(f("╭──────────────────────────────────────────────╮")),
        Line::from(spans),
        Line::from(f("╰──────────────────────────────────────────────╯")),
        Line::default(),
        Line::from(vec![key("⏎"), d(" send   "), key("⇧⏎"), d(" new line   "), key("esc"), d(" stop Claude")]),
        Line::from(vec![key("@"), d(" mention a file   "), key("/"), d(" commands   "), key("!"), d(" run a shell command")]),
    ]
}

fn demo_approve(e: f32) -> Vec<Line<'static>> {
    let add = Style::new().fg(theme::add_fg()).bg(theme::add_bg());
    let del = Style::new().fg(theme::del_fg()).bg(theme::del_bg());
    let lines = vec![
        Line::from(vec![Span::styled("⚠ ", theme::bold(theme::warn())), Span::styled("Claude wants to edit a file", theme::bold(theme::text()))]),
        Line::from(vec![tx("  src/fetch.rs"), d("   +1 −1")]),
        Line::from(vec![f("  │ "), Span::styled("- let tries = 1;         ", del)]),
        Line::from(vec![f("  │ "), Span::styled("+ let tries = 3;         ", add)]),
        Line::default(),
    ];
    let shown = ((e / 0.22) as usize + 1).min(lines.len());
    let mut out: Vec<Line> = lines.into_iter().take(shown).collect();
    if shown == 5 {
        // The "allow" key lights up, as if pressed.
        let pressed = e > 1.6;
        let y = if pressed { theme::bold(theme::on_accent()).bg(theme::ok()) } else { theme::bold(theme::ok()).bg(theme::sel_bg()) };
        out.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(" y ", y),
            d(" allow   "),
            Span::styled(" a ", theme::bold(theme::accent()).bg(theme::sel_bg())),
            d(" always   "),
            Span::styled(" n ", theme::bold(theme::err()).bg(theme::sel_bg())),
            d(" deny"),
        ]));
    }
    out
}

fn demo_tools(e: f32) -> Vec<Line<'static>> {
    let add = Style::new().fg(theme::add_fg()).bg(theme::add_bg());
    let state = |at: f32| if e >= at { Span::styled("✓", theme::s(theme::ok())) } else { Span::styled(anim::spinner(), theme::s(theme::accent())) };
    let mut out = vec![Line::from(vec![Span::raw(" "), state(0.5), d(" read    "), tx("src/fetch.rs"), d("           180 lines")])];
    if e > 0.5 {
        out.push(Line::from(vec![Span::raw(" "), state(1.1), d(" edit    "), tx("src/fetch.rs"), d("              +24 −3")]));
        out.push(Line::from(vec![f("    │ "), Span::styled("+ for attempt in 0..tries {   ", add)]));
    }
    if e > 1.1 {
        out.push(Line::from(vec![Span::raw(" "), state(2.0), d(" bash    "), tx("cargo test"), d("                   14 passed")]));
    }
    while out.len() < 4 {
        out.push(Line::default());
    }
    out.push(Line::default());
    out.push(Line::from(vec![key("^T"), d(" browse   "), key("⏎"), d(" expand   "), key("/files"), d(" see every diff")]));
    out
}

fn demo_sessions(e: f32) -> Vec<Line<'static>> {
    let row = |name: &str, status: Span<'static>, line: Span<'static>, at: f32| -> Vec<Line<'static>> {
        if e < at {
            return vec![Line::default(), Line::default()];
        }
        vec![Line::from(vec![d("  "), Span::styled(name.to_string(), theme::bold(theme::text())), f("        "), status]), Line::from(vec![d("    "), line])]
    };
    let mut out = Vec::new();
    out.extend(row("api   · retry logic ", ac(anim::spinner()), d("editing src/fetch.rs"), 0.1));
    out.extend(row("web   · audit       ", ac(anim::spinner()), d("2 agents running"), 0.4));
    out.extend(row("docs  · deploy      ", Span::styled("⚠", theme::s(theme::warn())), Span::styled("waiting for you", theme::s(theme::warn())), 0.7));
    out.push(Line::default());
    out.push(Line::from(vec![key("^N"), d(" new in any folder   "), key("^P"), d(" switch   "), key("^\\"), d(" split view")]));
    out
}

fn demo_done(_e: f32) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![key("^S"), d(" settings & themes   "), key("^F"), d(" search everything")]),
        Line::default(),
        Line::from(vec![key("/help"), d(" every key   "), key("/tutorial"), d(" this tour again")]),
    ]
}

struct Step {
    title: &'static str,
    body: &'static str,
    demo: fn(f32) -> Vec<Line<'static>>,
}

const ALL: [Step; STEPS - 1] = [
    Step {
        title: "Talk to Claude",
        body: "Type and press enter. Mention files with @, open commands with /, start a line with ! to run it in your shell.",
        demo: demo_talk,
    },
    Step {
        title: "You approve what it does",
        body: "When Claude wants to run a command or change a file, you see exactly what, right above the input.",
        demo: demo_approve,
    },
    Step {
        title: "Work stays tidy",
        body: "Every action is one line. Tasks, edits and test runs tick by as Claude works; open any of them for details.",
        demo: demo_tools,
    },
    Step {
        title: "Every project, one sidebar",
        body: "Sessions are grouped by folder and keep working in the background. The sidebar shows what each one is doing.",
        demo: demo_sessions,
    },
    Step {
        title: "You're set",
        body: "Everything is saved, and /reload picks up new versions without stopping Claude.",
        demo: demo_done,
    },
];

fn panel(fr: &mut Frame, area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(4));
    let h = h.min(area.height.saturating_sub(2));
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h);
    fr.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .style(Style::new().bg(theme::panel_bg()))
        .border_style(Style::new().fg(theme::accent()).bg(theme::panel_bg()));
    let inner = block.inner(rect);
    fr.render_widget(block, rect);
    inner
}

fn footer(buf: &mut Buffer, x: u16, y: u16, w: usize, step: usize) {
    let mut dots = Vec::new();
    for i in 0..STEPS {
        dots.push(if i == step { ac("━━") } else { f("──") });
        dots.push(Span::raw(" "));
    }
    buf.set_line(x, y, &Line::from(dots), w as u16);
    let next = match step {
        0 => "take the tour",
        s if s + 1 == STEPS => "start",
        _ => "next",
    };
    let mut nav = Vec::new();
    if step > 0 {
        nav.extend([d("←"), f(" back   ")]);
    }
    nav.extend([d("⏎"), f(&format!(" {next}   ")), d("esc"), f(" skip")]);
    let nav = Line::from(nav);
    let nw = nav.spans.iter().map(|s| width(&s.content)).sum::<usize>() as u16;
    buf.set_line(x + (w as u16).saturating_sub(nw), y, &nav, nw);
}

/// `t` eases the step change in (0 → 1); `e` is seconds since the step opened.
pub fn draw(fr: &mut Frame, area: Rect, step: usize, t: f32, e: f32, outfit: Option<companion::Outfit>) {
    let step = step.min(STEPS - 1);
    let inner = panel(fr, area, 74, 22);
    let buf = fr.buffer_mut();
    let cw = inner.width.saturating_sub(6) as usize;
    let bottom = inner.y + inner.height.saturating_sub(2);
    // Steps slide in a few cells from the right as they fade up.
    let slide = ((1.0 - t) * 4.0).round() as u16;
    let x = inner.x + 3 + slide;

    if step == 0 {
        // Title card: the critter hops in waving, then "den" sweeps in.
        let lane = Rect::new(inner.x + 4, inner.y + 1, companion::W + 4, companion::H + 4);
        let hop = (e / 1.4).clamp(0.0, 0.999);
        companion::draw(buf, lane, companion::Mood::Awake, anim::secs(), Some((companion::Reaction::Wave, hop)), outfit);
        let wx = inner.x + 4 + companion::W + 8;
        buf.set_string(wx, inner.y + 2, "welcome to", theme::s(theme::dim()));
        wordmark(buf, wx, inner.y + 3, anim::ease_out(((e - 0.25) / 0.7).clamp(0.0, 1.0)));
        let lines = [
            "A calm, fast home for Claude Code.",
            "Same Claude, same login, settings, tools and permissions.",
        ];
        for (y, (i, l)) in (inner.y + 13..).zip(lines.iter().enumerate()) {
            let k = ((e - 0.9 - i as f32 * 0.25) / 0.35).clamp(0.0, 1.0);
            if k > 0.0 {
                let st = if i == 0 { theme::bold(theme::text()) } else { theme::s(theme::dim()) };
                buf.set_string(inner.x + 4, y, *l, st);
                theme::fade(buf, Rect::new(inner.x + 4, y, width(l) as u16, 1), anim::ease_out(k));
            }
        }
        footer(buf, inner.x + 3, bottom, cw, step);
        return;
    }

    let s = &ALL[step - 1];
    let mut y = inner.y + 1;
    let counter = format!("{} / {}", step, STEPS - 1);
    buf.set_line(x, y, &Line::from(Span::styled(s.title, theme::bold(theme::accent()))), cw as u16);
    buf.set_string(inner.x + 3 + cw as u16 - counter.len() as u16, y, &counter, theme::s(theme::faint()));
    y += 2;
    for l in wrap(&[d(s.body)], cw, &[], &[]) {
        buf.set_line(x, y, &l, cw as u16);
        y += 1;
    }
    y += 1;
    for l in (s.demo)(e) {
        if y >= bottom.saturating_sub(1) {
            break;
        }
        buf.set_line(x + 1, y, &l, cw as u16);
        y += 1;
    }
    let content = Rect::new(inner.x, inner.y, inner.width, bottom.saturating_sub(inner.y));
    theme::fade(buf, content, t);
    footer(buf, inner.x + 3, bottom, cw, step);
}
