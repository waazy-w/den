use super::wrap::{width, wrap};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear};

pub const STEPS: usize = 6;

struct Step {
    title: &'static str,
    body: &'static str,
    demo: fn() -> Vec<Line<'static>>,
}

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

fn demo_welcome() -> Vec<Line<'static>> {
    let mut sprite = super::companion::lines();
    sprite[1].spans.extend([Span::raw("   "), Span::styled("meet Clawd", theme::bold(theme::accent()))]);
    sprite[2].spans.extend([Span::raw("   "), d("he keeps you company")]);
    sprite
}

fn demo_talk() -> Vec<Line<'static>> {
    vec![
        Line::from(f("╭─────────────────────────────────────────╮")),
        Line::from(vec![f("│ "), Span::styled("› ", theme::bold(theme::accent())), tx("fix the flaky test in "), ac("@tests/api.rs"), f("   │")]),
        Line::from(f("╰─────────────────────────────────────────╯")),
        Line::default(),
        Line::from(vec![key("⏎"), d(" send   "), key("⇧⏎"), d(" new line   "), key("esc"), d(" stop Claude")]),
        Line::from(vec![key("@"), d(" mention a file   "), key("/"), d(" commands")]),
    ]
}

fn demo_approve() -> Vec<Line<'static>> {
    let add = Style::new().fg(theme::add_fg()).bg(theme::add_bg());
    let del = Style::new().fg(theme::del_fg()).bg(theme::del_bg());
    vec![
        Line::from(vec![Span::styled("⚠ ", theme::bold(theme::warn())), Span::styled("Claude wants to edit a file", theme::bold(theme::text()))]),
        Line::from(vec![tx("  src/fetch.rs"), d("   +1 −1")]),
        Line::from(vec![f("  │ "), Span::styled("- let tries = 1;         ", del)]),
        Line::from(vec![f("  │ "), Span::styled("+ let tries = 3;         ", add)]),
        Line::default(),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(" y ", theme::bold(theme::ok()).bg(theme::sel_bg())),
            d(" allow   "),
            Span::styled(" a ", theme::bold(theme::accent()).bg(theme::sel_bg())),
            d(" always   "),
            Span::styled(" n ", theme::bold(theme::err()).bg(theme::sel_bg())),
            d(" deny"),
        ]),
    ]
}

fn demo_tools() -> Vec<Line<'static>> {
    let add = Style::new().fg(theme::add_fg()).bg(theme::add_bg());
    vec![
        Line::from(vec![Span::styled(" ✓", theme::s(theme::ok())), d(" read    "), tx("src/fetch.rs"), d("           180 lines")]),
        Line::from(vec![
            Span::styled("›", theme::s(theme::accent())),
            Span::styled("✓", theme::s(theme::ok())),
            d(" edit    "),
            Span::styled("src/fetch.rs", theme::bold(theme::text())),
            d("              +24 −3"),
        ]),
        Line::from(vec![f("    │ "), Span::styled("+ for attempt in 0..tries {   ", add)]),
        Line::from(vec![Span::styled(" ⠋", theme::s(theme::accent())), d(" bash    "), tx("cargo test"), d("                   2.1s")]),
        Line::default(),
        Line::from(vec![key("^T"), d(" browse   "), key("⏎"), d(" expand   "), key("^O"), d(" expand all")]),
    ]
}

fn demo_sessions() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled("my-app", theme::bold(theme::text())), f("                 2")]),
        Line::from(vec![ac("▍ "), Span::styled("retry logic", theme::bold(theme::text())), f("            "), ac("⠋")]),
        Line::from(vec![d("  docs pass"), f("              "), Span::styled("●", theme::s(theme::ok()))]),
        Line::from(vec![Span::styled("website", theme::bold(theme::text()))]),
        Line::from(vec![d("  fix navbar"), f("             "), Span::styled("⚠", theme::s(theme::warn()))]),
        Line::default(),
        Line::from(vec![ac("⠋"), d(" working   "), Span::styled("⚠", theme::s(theme::warn())), d(" needs you   "), Span::styled("●", theme::s(theme::ok())), d(" finished")]),
        Line::from(vec![key("^N"), d(" new in any folder   "), key("^P"), d(" switch")]),
    ]
}

fn demo_done() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![key("/help"), d(" every key   "), key("/tutorial"), d(" this tour again")]),
        Line::default(),
        Line::from(vec![key("^B"), d(" hide sidebar   "), key("^C ^C"), d(" quit")]),
    ]
}

const ALL: [Step; STEPS] = [
    Step {
        title: "Welcome to den",
        body: "A calm, fast home for Claude Code. Same Claude, same login, settings, tools and permissions. Just a cleaner view.",
        demo: demo_welcome,
    },
    Step {
        title: "Talk to Claude",
        body: "Type and press enter. Mention files with @, open commands with /, and press esc to stop Claude mid-reply.",
        demo: demo_talk,
    },
    Step {
        title: "Approve what it does",
        body: "When Claude wants to run a command or change a file, you see exactly what, right above the input.",
        demo: demo_approve,
    },
    Step {
        title: "Tool calls stay tidy",
        body: "Every action collapses to one line. Open any of them when you want the details.",
        demo: demo_tools,
    },
    Step {
        title: "One window, many projects",
        body: "Sessions are grouped by project folder and keep working in the background. The sidebar tells you which ones need you.",
        demo: demo_sessions,
    },
    Step {
        title: "You're set",
        body: "Sessions are saved and pick up where you left off, even after you quit.",
        demo: demo_done,
    },
];

pub fn draw(fr: &mut Frame, area: Rect, step: usize, t: f32) {
    let step = step.min(STEPS - 1);
    let s = &ALL[step];
    let w = 66u16.min(area.width.saturating_sub(4));
    let h = 20u16.min(area.height.saturating_sub(2));
    let rect = Rect::new(area.x + (area.width - w) / 2, area.y + (area.height - h) / 3, w, h);
    fr.render_widget(Clear, rect);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .style(ratatui::style::Style::new().bg(theme::panel_bg()))
        .border_style(ratatui::style::Style::new().fg(theme::accent()).bg(theme::panel_bg()));
    let inner = block.inner(rect);
    fr.render_widget(block, rect);
    let buf = fr.buffer_mut();
    let x = inner.x + 3;
    let cw = inner.width.saturating_sub(6) as usize;
    let mut y = inner.y + 1;

    let counter = format!("{} / {STEPS}", step + 1);
    buf.set_line(x, y, &Line::from(Span::styled(s.title, theme::bold(theme::accent()))), cw as u16);
    buf.set_string(x + cw as u16 - counter.len() as u16, y, &counter, theme::s(theme::faint()));
    y += 2;
    for l in wrap(&[d(s.body)], cw, &[], &[]) {
        buf.set_line(x, y, &l, cw as u16);
        y += 1;
    }
    y += 1;
    let bottom = inner.y + inner.height.saturating_sub(2);
    for l in (s.demo)() {
        if y >= bottom.saturating_sub(1) {
            break;
        }
        buf.set_line(x + 1, y, &l, cw as u16);
        y += 1;
    }

    let content = Rect::new(inner.x, inner.y, inner.width, bottom.saturating_sub(inner.y));
    theme::fade(buf, content, t);

    let mut dots = Vec::new();
    for i in 0..STEPS {
        dots.push(if i == step { ac("●") } else { f("○") });
        dots.push(Span::raw(" "));
    }
    buf.set_line(x, bottom, &Line::from(dots), cw as u16);
    let next = if step + 1 == STEPS { "start" } else { "next" };
    let mut nav = Vec::new();
    if step > 0 {
        nav.extend([d("←"), f(" back   ")]);
    }
    nav.extend([d("⏎"), f(&format!(" {next}   ")), d("esc"), f(" skip")]);
    let nav = Line::from(nav);
    let nw = nav.spans.iter().map(|s| width(&s.content)).sum::<usize>() as u16;
    buf.set_line(x + cw as u16 - nw, bottom, &nav, nw);
}
