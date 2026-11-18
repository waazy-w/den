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
    let nap = std::env::var("DEN_NAP_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(SLEEP_AFTER);
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
pub fn draw(buf: &mut Buffer, area: Rect, mood: Mood, t: f32, react: Option<(Reaction, f32)>, outfit: Option<Outfit>) {
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

    let worn = outfit.filter(|o| y0 >= area.y + o.rows());
    if let Some(o) = worn {
        draw_outfit(buf, x0, y0, o);
    }

    match mood {
        Mood::Alert if y0 > area.y && (t * 3.0).sin() > -0.4 => {
            // Beside the hat if he's wearing one.
            let x = if worn.is_some_and(|o| o.rows() > 0) { x0 + W } else { x0 + W / 2 - 1 };
            if x < area.right() {
                buf.set_string(x, y0 - 1, "!", theme::bold(theme::warn()));
            }
        }
        Mood::Asleep if y0 > area.y => {
            let step = ((t / 1.1) as u32) % 3;
            let (dx, dy, z) = [(W, 1, "z"), (W + 1, 2, "z"), (W + 2, 3, "Z")][step as usize];
            if y0 >= area.y + dy && x0 + dx < area.x + area.width {
                buf.set_string(x0 + dx, y0 - dy, z, theme::s(theme::dim()));
            }
        }
        _ => {}
    }
}

/// Seasonal headwear (or shades), picked from the date.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outfit {
    Santa,
    Party,
    Beanie,
    Flower,
    Shades,
    Witch,
}

impl Outfit {
    pub const ALL: [Outfit; 6] = [Outfit::Santa, Outfit::Party, Outfit::Beanie, Outfit::Flower, Outfit::Shades, Outfit::Witch];

    /// Rows needed above the body.
    pub fn rows(self) -> u16 {
        match self {
            Outfit::Santa | Outfit::Party | Outfit::Witch => 3,
            Outfit::Beanie | Outfit::Flower => 2,
            Outfit::Shades => 0,
        }
    }

    pub fn for_date(month: u32, day: u32) -> Option<Outfit> {
        match (month, day) {
            (11, 15..) | (12, _) => Some(Outfit::Santa),
            (1, 1..=7) => Some(Outfit::Party),
            (1, _) | (2, _) | (11, _) => Some(Outfit::Beanie),
            (3..=5, _) => Some(Outfit::Flower),
            (6..=8, _) => Some(Outfit::Shades),
            (10, _) => Some(Outfit::Witch),
            _ => None,
        }
    }

    /// Today's outfit (DEN_OUTFIT=santa|party|… forces one, for previews).
    pub fn today() -> Option<Outfit> {
        if let Ok(name) = std::env::var("DEN_OUTFIT") {
            return Outfit::ALL.into_iter().find(|o| format!("{o:?}").eq_ignore_ascii_case(&name));
        }
        let now = unsafe { libc::time(std::ptr::null_mut()) };
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe { libc::localtime_r(&now, &mut tm) };
        Outfit::for_date(tm.tm_mon as u32 + 1, tm.tm_mday as u32)
    }
}

fn c(r: u8, g: u8, b: u8) -> ratatui::style::Color {
    theme::mix((r, g, b), (r, g, b), 1.0)
}

/// Draws `o` on Clawd whose body's top-left is (x0, y0). Solid parts are
/// background-colored cells, so there are no seams.
fn draw_outfit(buf: &mut Buffer, x0: u16, y0: u16, o: Outfit) {
    let put = |buf: &mut Buffer, dx: u16, dy: i16, sym: &str, st: Style| {
        buf.set_string(x0 + dx, (y0 as i16 + dy) as u16, sym, st);
    };
    let fill = |buf: &mut Buffer, xs: std::ops::RangeInclusive<u16>, dy: i16, col| {
        for dx in xs {
            buf.set_string(x0 + dx, (y0 as i16 + dy) as u16, " ", Style::new().bg(col));
        }
    };
    let white = c(0xF4, 0xF4, 0xF5);
    match o {
        Outfit::Santa => {
            let red = c(0xD6, 0x33, 0x3A);
            for dx in 6..=9 {
                put(buf, dx, -3, "▄", Style::new().fg(red));
            }
            fill(buf, 5..=11, -2, red);
            put(buf, 12, -2, "▄", Style::new().fg(red));
            put(buf, 13, -2, "●", Style::new().fg(white));
            for dx in 3..=12 {
                put(buf, dx, -1, "▄", Style::new().fg(white).bg(red));
            }
        }
        Outfit::Party => {
            let blue = c(0x4F, 0x7C, 0xF0);
            let gold = c(0xF5, 0xC5, 0x42);
            put(buf, 8, -3, "●", Style::new().fg(gold));
            fill(buf, 7..=9, -2, blue);
            put(buf, 8, -2, " ", Style::new().bg(gold));
            fill(buf, 6..=10, -1, blue);
            put(buf, 7, -1, " ", Style::new().bg(gold));
            put(buf, 9, -1, " ", Style::new().bg(gold));
        }
        Outfit::Beanie => {
            let teal = c(0x2B, 0x9C, 0x8F);
            put(buf, 7, -2, "▄", Style::new().fg(white));
            put(buf, 8, -2, "▄", Style::new().fg(white));
            put(buf, 3, -1, "▄", Style::new().fg(teal));
            fill(buf, 4..=11, -1, teal);
            put(buf, 12, -1, "▄", Style::new().fg(teal));
            for dx in 3..=12 {
                put(buf, dx, 0, "▀", Style::new().fg(white).bg(theme::clawd()));
            }
        }
        Outfit::Flower => {
            let pink = c(0xF2, 0x7F, 0xB5);
            let gold = c(0xF5, 0xC5, 0x42);
            let green = c(0x4C, 0xB8, 0x6A);
            put(buf, 10, -2, "▄", Style::new().fg(pink));
            put(buf, 11, -2, "▄", Style::new().fg(gold).bg(pink));
            put(buf, 12, -2, "▄", Style::new().fg(pink));
            put(buf, 11, -1, "▀", Style::new().fg(pink).bg(green));
        }
        Outfit::Shades => {
            let dark = c(0x14, 0x14, 0x18);
            fill(buf, 3..=5, 1, dark);
            fill(buf, 10..=12, 1, dark);
            for dx in 6..=9 {
                put(buf, dx, 1, "▀", Style::new().fg(dark).bg(theme::clawd()));
            }
            put(buf, 3, 1, "▘", Style::new().fg(c(0x9A, 0x9A, 0xA6)).bg(dark));
        }
        Outfit::Witch => {
            let hat = c(0x3B, 0x2A, 0x55);
            let band = c(0xF0, 0x8A, 0x3C);
            put(buf, 8, -3, "▄", Style::new().fg(hat));
            put(buf, 9, -3, "▄", Style::new().fg(hat));
            fill(buf, 6..=9, -2, hat);
            fill(buf, 5..=10, -1, band);
            for dx in (2..=4).chain(11..=13) {
                put(buf, dx, -1, "▄", Style::new().fg(hat));
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn render(m: Mood, t: f32) -> Buffer {
        let area = Rect::new(0, 0, W + 2, H + 3);
        let mut b = Buffer::empty(area);
        draw(&mut b, area, m, t, None, None);
        b
    }

    #[test]
    fn reactions_move_him() {
        let area = Rect::new(0, 0, W + 2, H + 3);
        let mut still = Buffer::empty(area);
        draw(&mut still, area, Mood::Awake, 1.0, None, None);
        let mut hop = Buffer::empty(area);
        draw(&mut hop, area, Mood::Awake, 1.0, Some((Reaction::Hop, 0.5)), None);
        // Top body row moves up by one.
        assert_ne!(still[(6, 3)].bg, Color::Reset);
        assert_ne!(hop[(6, 2)].bg, Color::Reset);
        assert_eq!(still[(6, 2)].bg, Color::Reset);
        let mut wave = Buffer::empty(area);
        draw(&mut wave, area, Mood::Awake, 1.0, Some((Reaction::Wave, 0.0)), None);
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
    fn outfits_follow_the_calendar() {
        assert_eq!(Outfit::for_date(12, 3), Some(Outfit::Santa));
        assert_eq!(Outfit::for_date(11, 18), Some(Outfit::Santa));
        assert_eq!(Outfit::for_date(11, 3), Some(Outfit::Beanie));
        assert_eq!(Outfit::for_date(1, 1), Some(Outfit::Party));
        assert_eq!(Outfit::for_date(7, 4), Some(Outfit::Shades));
        assert_eq!(Outfit::for_date(10, 31), Some(Outfit::Witch));
        assert_eq!(Outfit::for_date(9, 10), None);
        // Every outfit fits inside the space Clawd is drawn in.
        for o in Outfit::ALL {
            let area = Rect::new(0, 0, W + 4, H + 4);
            let mut b = Buffer::empty(area);
            draw(&mut b, area, Mood::Awake, 1.0, None, Some(o));
        }
    }

    #[test]
    fn alert_shows_mark() {
        let b = render(Mood::Alert, 0.1);
        assert!((0..W + 2).any(|x| b[(x, 2)].symbol() == "!"));
    }
}
