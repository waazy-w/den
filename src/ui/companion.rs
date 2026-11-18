//! Clawd: a sprite laid out on the cell grid. Solid areas are
//! background-colored spaces, so terminals with line spacing (e.g. macOS
//! Terminal) draw no seams through him; half-blocks only at the outer edges.

use crate::theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;

/// `B` body, `E` eye, `n` nub (top half), `l` leg (bottom half), `.` empty.
const SPRITE: [&str; 5] = [
    ".BBBBBBBBBBBBBB.",
    "nBBBEBBBBBBEBBBn",
    ".BBBBBBBBBBBBBB.",
    ".BBBBBBBBBBBBBB.",
    "..l.l......l.l..",
];

pub const W: u16 = 16;
pub const H: u16 = 5;
/// Seconds idle before he dozes off, and before he stops animating.
const SLEEP_AFTER: f32 = 20.0;
pub const DEEP_SLEEP_AFTER: f32 = 200.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mood {
    Awake,
    Working,
    Alert,
    Asleep,
}

pub fn mood(busy: bool, needs_you: bool, idle_secs: f32) -> Mood {
    let nap = std::env::var("CTUI_NAP_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(SLEEP_AFTER);
    if needs_you {
        Mood::Alert
    } else if busy {
        Mood::Working
    } else if idle_secs > nap {
        Mood::Asleep
    } else {
        Mood::Awake
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Eyes {
    Open,
    Down,
    Shut,
    Happy,
}

/// Short one-off moves: a hop when a reply lands, a wiggle when tests pass,
/// a wave hello on a new session.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Reaction {
    Hop,
    Wiggle,
    Wave,
}

impl Reaction {
    pub fn secs(self) -> f32 {
        match self {
            Reaction::Hop => 0.7,
            Reaction::Wiggle => 0.9,
            Reaction::Wave => 1.4,
        }
    }
}

/// Draws Clawd into `area` (centered, bottom-aligned) for `mood` at time `t`.
/// Extras like `!` and `z` use the rows above him when there is room.
/// `react` is a reaction and its progress (0..1).
pub fn draw(buf: &mut Buffer, area: Rect, mood: Mood, t: f32, react: Option<(Reaction, f32)>) {
    if area.width < W || area.height < H {
        return;
    }
    let body = theme::clawd();
    let eye = theme::eye();

    let blinking = (t % 4.3) < 0.15;
    let (eyes, look) = match mood {
        Mood::Asleep => (Eyes::Shut, 0i16),
        Mood::Working => (if blinking { Eyes::Shut } else { Eyes::Down }, 0),
        Mood::Alert => (Eyes::Open, 0),
        Mood::Awake => {
            let p = t % 14.0;
            let look = if (6.0..7.5).contains(&p) {
                -1
            } else if (10.0..11.5).contains(&p) {
                1
            } else {
                0
            };
            (if blinking { Eyes::Shut } else { Eyes::Open }, look)
        }
    };
    // Working: the nubs tap in turn, like typing.
    let tap = mood == Mood::Working && ((t / 0.22) as u32).is_multiple_of(2);

    let mut x0 = area.x + (area.width - W) / 2;
    let mut y0 = area.y + area.height - H;
    let mut eyes = eyes;
    let mut wave: Option<bool> = None;
    if let Some((r, p)) = react {
        eyes = Eyes::Happy;
        match r {
            Reaction::Hop if (0.15..0.85).contains(&p) && y0 > area.y => y0 -= 1,
            Reaction::Wiggle => {
                let k = (p * 6.0) as u32;
                if k < 6 {
                    let dx: i16 = if k.is_multiple_of(2) { -1 } else { 1 };
                    x0 = (x0 as i16 + dx).clamp(area.x as i16, (area.x + area.width - W) as i16) as u16;
                }
            }
            Reaction::Wave => wave = Some(((p * 7.0) as u32).is_multiple_of(2)),
            _ => {}
        }
    }
    for (dy, row) in SPRITE.iter().enumerate() {
        for (dx, ch) in row.chars().enumerate() {
            // Eyes are drawn separately below so they can move and blink.
            let ch = if ch == 'E' { 'B' } else { ch };
            let (sym, st) = match ch {
                'B' => (" ", Style::new().bg(body)),
                'n' if dx > 0 && wave.is_some() => continue,
                'n' => {
                    let down = if dx == 0 { tap } else { mood == Mood::Working && !tap };
                    (if down { "▄" } else { "▀" }, Style::new().fg(body))
                }
                'l' => ("▄", Style::new().fg(body)),
                _ => continue,
            };
            buf.set_string(x0 + dx as u16, y0 + dy as u16, sym, st);
        }
    }
    for ex in [4i16, 11] {
        let x = (x0 as i16 + ex + look) as u16;
        let (sym, st) = match eyes {
            Eyes::Open => (" ", Style::new().bg(eye)),
            Eyes::Down => ("▄", Style::new().fg(eye).bg(body)),
            Eyes::Shut => ("▁", Style::new().fg(eye).bg(body)),
            Eyes::Happy => ("▀", Style::new().fg(eye).bg(body)),
        };
        buf.set_string(x, y0 + 1, sym, st);
    }
    // Waving: the right nub raised, flapping between two heights.
    if let Some(high) = wave {
        let x = x0 + W - 1;
        if high && y0 > area.y {
            buf.set_string(x, y0 - 1, "▄", Style::new().fg(body));
            buf.set_string(x, y0, "▌", Style::new().fg(body));
        } else {
            buf.set_string(x, y0, "▄", Style::new().fg(body));
        }
    }

    match mood {
        Mood::Alert if y0 > area.y && (t * 3.0).sin() > -0.4 => {
            buf.set_string(x0 + W / 2 - 1, y0 - 1, "!", theme::bold(theme::warn()));
        }
        Mood::Asleep if y0 > area.y => {
            let step = ((t / 1.1) as u32) % 3;
            let (dx, dy, z) = [(W - 2, 1, "z"), (W - 1, 2, "z"), (W, 3, "Z")][step as usize];
            if y0 >= area.y + dy && x0 + dx < area.x + area.width {
                buf.set_string(x0 + dx, y0 - dy, z, theme::s(theme::dim()));
            }
        }
        _ => {}
    }
}

/// The sprite as styled lines (for the tutorial card).
pub fn lines() -> Vec<ratatui::text::Line<'static>> {
    let area = Rect::new(0, 0, W, H);
    let mut b = Buffer::empty(area);
    draw(&mut b, area, Mood::Awake, 1.0, None);
    (0..H)
        .map(|y| {
            ratatui::text::Line::from(
                (0..W)
                    .map(|x| {
                        let c = &b[(x, y)];
                        ratatui::text::Span::styled(c.symbol().to_string(), c.style())
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn render(m: Mood, t: f32) -> Buffer {
        let area = Rect::new(0, 0, W + 2, H + 3);
        let mut b = Buffer::empty(area);
        draw(&mut b, area, m, t, None);
        b
    }

    #[test]
    fn reactions_move_him() {
        let area = Rect::new(0, 0, W + 2, H + 3);
        let mut still = Buffer::empty(area);
        draw(&mut still, area, Mood::Awake, 1.0, None);
        let mut hop = Buffer::empty(area);
        draw(&mut hop, area, Mood::Awake, 1.0, Some((Reaction::Hop, 0.5)));
        // Top body row moves up by one.
        assert_ne!(still[(6, 3)].bg, Color::Reset);
        assert_ne!(hop[(6, 2)].bg, Color::Reset);
        assert_eq!(still[(6, 2)].bg, Color::Reset);
        let mut wave = Buffer::empty(area);
        draw(&mut wave, area, Mood::Awake, 1.0, Some((Reaction::Wave, 0.0)));
        assert_eq!(wave[(1 + W - 1, 2)].symbol(), "▄");
    }

    #[test]
    fn sprite_fits_and_solid_cells_use_background() {
        assert!(SPRITE.iter().all(|r| r.chars().count() == W as usize));
        let b = render(Mood::Awake, 1.0);
        let c = &b[(6, 3)];
        assert_eq!(c.symbol(), " ");
        assert_ne!(c.bg, Color::Reset);
    }

    #[test]
    fn moods_change_the_eyes() {
        let eye_row = 3 + 1;
        let open = render(Mood::Awake, 1.0)[(5, eye_row)].clone();
        let shut = render(Mood::Asleep, 1.0)[(5, eye_row)].clone();
        let down = render(Mood::Working, 1.0)[(5, eye_row)].clone();
        assert_eq!(open.symbol(), " ");
        assert_eq!(shut.symbol(), "▁");
        assert_eq!(down.symbol(), "▄");
    }

    #[test]
    fn alert_shows_mark() {
        let b = render(Mood::Alert, 0.1);
        assert!((0..W + 2).any(|x| b[(x, 2)].symbol() == "!"));
    }
}
