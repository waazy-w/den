use ratatui::style::{Color, Modifier, Style};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};

static TRUECOLOR: OnceLock<bool> = OnceLock::new();

pub fn init() {
    let tc = std::env::var("COLORTERM")
        .map(|v| v.contains("truecolor") || v.contains("24bit"))
        .unwrap_or(false)
        || matches!(
            std::env::var("TERM_PROGRAM").as_deref(),
            Ok("iTerm.app" | "WezTerm" | "ghostty" | "vscode" | "WarpTerminal")
        );
    let _ = TRUECOLOR.set(tc);
}

pub type Rgb = (u8, u8, u8);

/// One color scheme. `bg` None keeps the terminal's own background.
pub struct Palette {
    pub name: &'static str,
    pub blurb: &'static str,
    pub bg: Option<Rgb>,
    pub text: Rgb,
    pub dim: Rgb,
    pub faint: Rgb,
    pub rule: Rgb,
    pub panel_bg: Rgb,
    pub hi_bg: Rgb,
    pub sel_bg: Rgb,
    pub accent_soft: Rgb,
    pub accent: Rgb,
    pub accent_hi: Rgb,
    pub ok: Rgb,
    pub warn: Rgb,
    pub err: Rgb,
    pub code_fg: Rgb,
    pub code_bg: Rgb,
    pub add_bg: Rgb,
    pub del_bg: Rgb,
    pub add_fg: Rgb,
    pub del_fg: Rgb,
    pub flash: Rgb,
    /// Text drawn on top of accent-colored badges.
    pub on_accent: Rgb,
    pub clawd: Rgb,
    pub eye: Rgb,
}

pub const PALETTES: &[Palette] = &[
    Palette {
        name: "dark",
        blurb: "the default · uses your terminal's background",
        bg: None,
        text: (0xF4, 0xF4, 0xF5),
        dim: (0xA1, 0xA1, 0xAA),
        faint: (0x52, 0x52, 0x5B),
        rule: (0x3C, 0x3C, 0x44),
        panel_bg: (0x23, 0x23, 0x29),
        hi_bg: (0x42, 0x42, 0x4B),
        sel_bg: (0x36, 0x36, 0x3E),
        accent_soft: (0x8A, 0x8A, 0x93),
        accent: (0xEB, 0x7A, 0x4F),
        accent_hi: (0xFF, 0xC8, 0xA8),
        ok: (0x4A, 0xDE, 0x80),
        warn: (0xE5, 0xC0, 0x7B),
        err: (0xF8, 0x71, 0x71),
        code_fg: (0xE8, 0xA0, 0x87),
        code_bg: (0x1C, 0x1C, 0x20),
        add_bg: (0x16, 0x2E, 0x20),
        del_bg: (0x3A, 0x1A, 0x1C),
        add_fg: (0x22, 0xC5, 0x5E),
        del_fg: (0xF8, 0x71, 0x71),
        flash: (0xEC, 0xFD, 0xF3),
        on_accent: (0x18, 0x18, 0x1B),
        clawd: (0xCD, 0x6E, 0x58),
        eye: (0x10, 0x10, 0x12),
    },
    Palette {
        name: "claude",
        blurb: "warm dark with Claude orange",
        bg: Some((0x1F, 0x1B, 0x18)),
        text: (0xF5, 0xEF, 0xE9),
        dim: (0xB5, 0xA8, 0x9C),
        faint: (0x5E, 0x54, 0x4C),
        rule: (0x3A, 0x31, 0x2B),
        panel_bg: (0x29, 0x23, 0x1F),
        hi_bg: (0x4A, 0x3D, 0x34),
        sel_bg: (0x38, 0x2E, 0x28),
        accent_soft: (0xC2, 0x8B, 0x6E),
        accent: (0xD9, 0x77, 0x57),
        accent_hi: (0xFF, 0xC4, 0xA6),
        ok: (0x8F, 0xD1, 0x8F),
        warn: (0xF0, 0xC0, 0x6A),
        err: (0xF0, 0x7A, 0x6A),
        code_fg: (0xF0, 0xA4, 0x84),
        code_bg: (0x1A, 0x16, 0x13),
        add_bg: (0x1E, 0x2F, 0x20),
        del_bg: (0x3B, 0x1F, 0x1A),
        add_fg: (0x6C, 0xC9, 0x7E),
        del_fg: (0xF0, 0x7A, 0x6A),
        flash: (0xFF, 0xF3, 0xEA),
        on_accent: (0x1F, 0x1B, 0x18),
        clawd: (0xD9, 0x77, 0x57),
        eye: (0x1F, 0x1B, 0x18),
    },
    Palette {
        name: "cream",
        blurb: "Claude's paper light",
        bg: Some((0xF5, 0xF1, 0xEA)),
        text: (0x2B, 0x25, 0x20),
        dim: (0x6B, 0x60, 0x56),
        faint: (0xAE, 0xA4, 0x99),
        rule: (0xE0, 0xD8, 0xCD),
        panel_bg: (0xEE, 0xE8, 0xDE),
        hi_bg: (0xDD, 0xD3, 0xC5),
        sel_bg: (0xE7, 0xDF, 0xD3),
        accent_soft: (0x8A, 0x7C, 0x70),
        accent: (0xC9, 0x64, 0x42),
        accent_hi: (0xF0, 0xA0, 0x80),
        ok: (0x2F, 0x8F, 0x4E),
        warn: (0xA8, 0x6E, 0x0E),
        err: (0xC2, 0x37, 0x2F),
        code_fg: (0xA8, 0x4E, 0x2C),
        code_bg: (0xEC, 0xE5, 0xD9),
        add_bg: (0xDD, 0xF0, 0xDD),
        del_bg: (0xF7, 0xDE, 0xD8),
        add_fg: (0x2B, 0x7A, 0x40),
        del_fg: (0xB0, 0x30, 0x28),
        flash: (0x2B, 0x7A, 0x40),
        on_accent: (0xFF, 0xFF, 0xFF),
        clawd: (0xD9, 0x77, 0x57),
        eye: (0x2B, 0x25, 0x20),
    },
    Palette {
        name: "midnight",
        blurb: "deep navy, softer contrast",
        bg: Some((0x0E, 0x11, 0x1B)),
        text: (0xE6, 0xE9, 0xF2),
        dim: (0x93, 0x9B, 0xB4),
        faint: (0x4A, 0x52, 0x6B),
        rule: (0x26, 0x2C, 0x3F),
        panel_bg: (0x16, 0x1B, 0x2A),
        hi_bg: (0x2E, 0x36, 0x50),
        sel_bg: (0x22, 0x29, 0x3F),
        accent_soft: (0x7C, 0x86, 0xA2),
        accent: (0xF0, 0x8A, 0x5D),
        accent_hi: (0xFF, 0xD0, 0xB0),
        ok: (0x5E, 0xE0, 0x9A),
        warn: (0xF2, 0xC9, 0x6D),
        err: (0xFF, 0x7A, 0x85),
        code_fg: (0xF2, 0xB0, 0x90),
        code_bg: (0x13, 0x17, 0x24),
        add_bg: (0x12, 0x2B, 0x26),
        del_bg: (0x36, 0x18, 0x24),
        add_fg: (0x34, 0xD3, 0x99),
        del_fg: (0xFF, 0x7A, 0x85),
        flash: (0xE8, 0xFF, 0xF4),
        on_accent: (0x0E, 0x11, 0x1B),
        clawd: (0xD8, 0x77, 0x5E),
        eye: (0x0E, 0x11, 0x1B),
    },
    Palette {
        name: "ocean",
        blurb: "deep sea blue",
        bg: Some((0x0B, 0x16, 0x1F)),
        text: (0xE3, 0xEE, 0xF5),
        dim: (0x8F, 0xA8, 0xB8),
        faint: (0x42, 0x58, 0x68),
        rule: (0x1D, 0x30, 0x3E),
        panel_bg: (0x10, 0x1E, 0x2A),
        hi_bg: (0x24, 0x3C, 0x4E),
        sel_bg: (0x18, 0x2C, 0x3A),
        accent_soft: (0x76, 0x95, 0xA8),
        accent: (0x4F, 0xB6, 0xF0),
        accent_hi: (0xBF, 0xE6, 0xFF),
        ok: (0x5E, 0xE0, 0xB0),
        warn: (0xF2, 0xC9, 0x6D),
        err: (0xFF, 0x7A, 0x85),
        code_fg: (0x8F, 0xD3, 0xF5),
        code_bg: (0x0E, 0x1B, 0x26),
        add_bg: (0x10, 0x2E, 0x2A),
        del_bg: (0x35, 0x18, 0x22),
        add_fg: (0x34, 0xD3, 0x99),
        del_fg: (0xFF, 0x7A, 0x85),
        flash: (0xE8, 0xFF, 0xF4),
        on_accent: (0x0B, 0x16, 0x1F),
        clawd: (0xD8, 0x77, 0x5E),
        eye: (0x0B, 0x16, 0x1F),
    },
    Palette {
        name: "forest",
        blurb: "deep green, calm",
        bg: Some((0x11, 0x17, 0x12)),
        text: (0xE6, 0xEE, 0xE8),
        dim: (0x98, 0xA9, 0x9C),
        faint: (0x4A, 0x5A, 0x4E),
        rule: (0x28, 0x34, 0x2B),
        panel_bg: (0x18, 0x20, 0x19),
        hi_bg: (0x31, 0x40, 0x36),
        sel_bg: (0x23, 0x30, 0x26),
        accent_soft: (0x82, 0x97, 0x8A),
        accent: (0x6B, 0xD0, 0x8C),
        accent_hi: (0xC8, 0xF5, 0xD5),
        ok: (0xB5, 0xE8, 0x7A),
        warn: (0xE8, 0xC8, 0x6E),
        err: (0xF2, 0x7B, 0x72),
        code_fg: (0x9F, 0xDB, 0xB2),
        code_bg: (0x14, 0x1B, 0x16),
        add_bg: (0x17, 0x33, 0x1F),
        del_bg: (0x3A, 0x1D, 0x1C),
        add_fg: (0x4A, 0xD0, 0x7A),
        del_fg: (0xF2, 0x7B, 0x72),
        flash: (0xEC, 0xFD, 0xF3),
        on_accent: (0x11, 0x17, 0x12),
        clawd: (0xCD, 0x6E, 0x58),
        eye: (0x10, 0x12, 0x10),
    },
    Palette {
        name: "rose",
        blurb: "soft pink",
        bg: Some((0x1C, 0x14, 0x18)),
        text: (0xF6, 0xEA, 0xEF),
        dim: (0xB8, 0x9C, 0xA8),
        faint: (0x5E, 0x47, 0x51),
        rule: (0x3A, 0x28, 0x31),
        panel_bg: (0x25, 0x1A, 0x20),
        hi_bg: (0x4A, 0x32, 0x3E),
        sel_bg: (0x36, 0x25, 0x2E),
        accent_soft: (0xB0, 0x88, 0x9A),
        accent: (0xF2, 0x7F, 0xB5),
        accent_hi: (0xFF, 0xD0, 0xE6),
        ok: (0x7F, 0xDB, 0xA4),
        warn: (0xF2, 0xC9, 0x6D),
        err: (0xFF, 0x76, 0x7E),
        code_fg: (0xF5, 0xA8, 0xC8),
        code_bg: (0x18, 0x11, 0x15),
        add_bg: (0x18, 0x30, 0x24),
        del_bg: (0x40, 0x1A, 0x22),
        add_fg: (0x5E, 0xD0, 0x8E),
        del_fg: (0xFF, 0x76, 0x7E),
        flash: (0xFF, 0xF0, 0xF6),
        on_accent: (0x1C, 0x14, 0x18),
        clawd: (0xD9, 0x77, 0x57),
        eye: (0x1C, 0x14, 0x18),
    },
    Palette {
        name: "dusk",
        blurb: "twilight purple",
        bg: Some((0x15, 0x13, 0x1F)),
        text: (0xEE, 0xEA, 0xF8),
        dim: (0xA3, 0x9C, 0xC0),
        faint: (0x50, 0x4A, 0x6A),
        rule: (0x2C, 0x28, 0x40),
        panel_bg: (0x1D, 0x1A, 0x2A),
        hi_bg: (0x3A, 0x34, 0x55),
        sel_bg: (0x29, 0x25, 0x3D),
        accent_soft: (0x94, 0x8C, 0xB4),
        accent: (0xB0, 0x8C, 0xFF),
        accent_hi: (0xE2, 0xD6, 0xFF),
        ok: (0x6E, 0xE0, 0xA8),
        warn: (0xF2, 0xC9, 0x6D),
        err: (0xFF, 0x7A, 0x90),
        code_fg: (0xC9, 0xB2, 0xFF),
        code_bg: (0x19, 0x16, 0x24),
        add_bg: (0x14, 0x2C, 0x28),
        del_bg: (0x38, 0x18, 0x2A),
        add_fg: (0x48, 0xD0, 0x9A),
        del_fg: (0xFF, 0x7A, 0x90),
        flash: (0xF4, 0xF0, 0xFF),
        on_accent: (0x15, 0x13, 0x1F),
        clawd: (0xD9, 0x77, 0x57),
        eye: (0x15, 0x13, 0x1F),
    },
    Palette {
        name: "light",
        blurb: "paper white, for bright rooms",
        bg: Some((0xFA, 0xFA, 0xF8)),
        text: (0x1C, 0x1C, 0x21),
        dim: (0x5C, 0x5C, 0x68),
        faint: (0xA3, 0xA3, 0xAD),
        rule: (0xDD, 0xDD, 0xE2),
        panel_bg: (0xF0, 0xF0, 0xEE),
        hi_bg: (0xDA, 0xDA, 0xE0),
        sel_bg: (0xE6, 0xE6, 0xEA),
        accent_soft: (0x76, 0x76, 0x82),
        accent: (0xD2, 0x5E, 0x33),
        accent_hi: (0xF5, 0xA0, 0x78),
        ok: (0x15, 0x9A, 0x48),
        warn: (0xB4, 0x74, 0x0A),
        err: (0xD0, 0x2B, 0x2B),
        code_fg: (0xB0, 0x48, 0x20),
        code_bg: (0xF1, 0xF0, 0xEC),
        add_bg: (0xDB, 0xF3, 0xE2),
        del_bg: (0xFB, 0xE2, 0xE2),
        add_fg: (0x15, 0x80, 0x3D),
        del_fg: (0xB9, 0x1C, 0x1C),
        flash: (0x15, 0x80, 0x3D),
        on_accent: (0xFF, 0xFF, 0xFF),
        clawd: (0xCD, 0x6E, 0x58),
        eye: (0x1A, 0x1A, 0x1A),
    },
    Palette {
        name: "contrast",
        blurb: "high contrast, black background",
        bg: Some((0x00, 0x00, 0x00)),
        text: (0xFF, 0xFF, 0xFF),
        dim: (0xD4, 0xD4, 0xD8),
        faint: (0x8E, 0x8E, 0x96),
        rule: (0x6B, 0x6B, 0x73),
        panel_bg: (0x14, 0x14, 0x17),
        hi_bg: (0x4A, 0x4A, 0x52),
        sel_bg: (0x30, 0x30, 0x36),
        accent_soft: (0xC0, 0xC0, 0xC8),
        accent: (0xFF, 0x8C, 0x52),
        accent_hi: (0xFF, 0xD6, 0xBA),
        ok: (0x5C, 0xFF, 0x96),
        warn: (0xFF, 0xD8, 0x3A),
        err: (0xFF, 0x6B, 0x6B),
        code_fg: (0xFF, 0xB8, 0x94),
        code_bg: (0x0C, 0x0C, 0x0E),
        add_bg: (0x0B, 0x3A, 0x1E),
        del_bg: (0x4A, 0x10, 0x14),
        add_fg: (0x5C, 0xFF, 0x96),
        del_fg: (0xFF, 0x6B, 0x6B),
        flash: (0xFF, 0xFF, 0xFF),
        on_accent: (0x00, 0x00, 0x00),
        clawd: (0xE0, 0x7A, 0x60),
        eye: (0x00, 0x00, 0x00),
    },
    Palette {
        name: "mono",
        blurb: "grayscale, no color but Clawd",
        bg: None,
        text: (0xEE, 0xEE, 0xEE),
        dim: (0xA8, 0xA8, 0xA8),
        faint: (0x55, 0x55, 0x55),
        rule: (0x3A, 0x3A, 0x3A),
        panel_bg: (0x22, 0x22, 0x22),
        hi_bg: (0x44, 0x44, 0x44),
        sel_bg: (0x33, 0x33, 0x33),
        accent_soft: (0x8C, 0x8C, 0x8C),
        accent: (0xFF, 0xFF, 0xFF),
        accent_hi: (0xFF, 0xFF, 0xFF),
        ok: (0xC8, 0xE6, 0xC9),
        warn: (0xE0, 0xD2, 0xA8),
        err: (0xF0, 0xA8, 0xA8),
        code_fg: (0xD4, 0xD4, 0xD4),
        code_bg: (0x1C, 0x1C, 0x1C),
        add_bg: (0x1F, 0x2A, 0x20),
        del_bg: (0x2E, 0x1E, 0x1E),
        add_fg: (0xB8, 0xE0, 0xBC),
        del_fg: (0xF0, 0xA8, 0xA8),
        flash: (0xFF, 0xFF, 0xFF),
        on_accent: (0x11, 0x11, 0x11),
        clawd: (0xCD, 0x6E, 0x58),
        eye: (0x10, 0x10, 0x10),
    },
];

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn p() -> &'static Palette {
    &PALETTES[CURRENT.load(Ordering::Relaxed).min(PALETTES.len() - 1)]
}

/// Switches theme by name; returns false if there is no such theme.
pub fn set(name: &str) -> bool {
    match PALETTES.iter().position(|t| t.name.eq_ignore_ascii_case(name)) {
        Some(i) => {
            CURRENT.store(i, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

pub fn current_index() -> usize {
    CURRENT.load(Ordering::Relaxed)
}

pub fn set_index(i: usize) {
    CURRENT.store(i.min(PALETTES.len() - 1), Ordering::Relaxed);
}

/// The background to paint, as a color (terminal default for "dark").
pub fn bg() -> Color {
    p().bg.map_or(Color::Reset, |(r, g, b)| rgb(r, g, b))
}

/// The background as RGB, for blends (assumes a near-black terminal when unset).
pub fn bg_rgb() -> Rgb {
    p().bg.unwrap_or((0x18, 0x18, 0x1B))
}

pub fn faint_rgb() -> Rgb { p().faint }
pub fn accent_rgb() -> Rgb { p().accent }
pub fn accent_hi_rgb() -> Rgb { p().accent_hi }
pub fn ok_rgb() -> Rgb { p().ok }
pub fn warn_rgb() -> Rgb { p().warn }
pub fn flash_rgb() -> Rgb { p().flash }

pub fn truecolor() -> bool {
    *TRUECOLOR.get().unwrap_or(&true)
}

/// Linear blend from `a` (t=0) to `b` (t=1).
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    rgb(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
}

/// Fades every truecolor foreground in `area` toward the faint tone.
pub fn fade(buf: &mut ratatui::buffer::Buffer, area: ratatui::layout::Rect, t: f32) {
    if t >= 1.0 || !truecolor() {
        return;
    }
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buf.cell_mut((x, y))
                && let Color::Rgb(r, g, b) = cell.fg
            {
                cell.fg = mix(faint_rgb(), (r, g, b), t);
            }
        }
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Color {
    if truecolor() {
        Color::Rgb(r, g, b)
    } else {
        Color::Indexed(to_ansi256(r, g, b))
    }
}

fn to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let (qr, qg, qb) = (q(r), q(g), q(b));
    let cube = 16 + 36 * qr + 6 * qg + qb;
    let lvl = [0u8, 95, 135, 175, 215, 255];
    let cube_err = dist((lvl[qr as usize], lvl[qg as usize], lvl[qb as usize]), (r, g, b));
    let avg = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let gi = if avg > 238 { 23 } else { avg.saturating_sub(3) / 10 };
    let gv = 8 + 10 * gi;
    let gray_err = dist((gv, gv, gv), (r, g, b));
    if gray_err < cube_err { 232 + gi } else { cube }
}

fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2) as u32;
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

fn c(x: Rgb) -> Color {
    rgb(x.0, x.1, x.2)
}

pub fn text() -> Color { c(p().text) }
pub fn dim() -> Color { c(p().dim) }
pub fn faint() -> Color { c(p().faint) }
pub fn rule() -> Color { c(p().rule) }
pub fn panel_bg() -> Color { c(p().panel_bg) }
pub fn hi_bg() -> Color { c(p().hi_bg) }
pub fn accent_soft() -> Color { c(p().accent_soft) }
pub fn accent() -> Color { c(p().accent) }
pub fn ok() -> Color { c(p().ok) }
pub fn warn() -> Color { c(p().warn) }
pub fn err() -> Color { c(p().err) }
pub fn code_fg() -> Color { c(p().code_fg) }
pub fn code_bg() -> Color { c(p().code_bg) }
pub fn add_bg() -> Color { c(p().add_bg) }
pub fn del_bg() -> Color { c(p().del_bg) }
pub fn add_fg() -> Color { c(p().add_fg) }
pub fn del_fg() -> Color { c(p().del_fg) }
pub fn sel_bg() -> Color { c(p().sel_bg) }
pub fn on_accent() -> Color { c(p().on_accent) }
pub fn clawd() -> Color { c(p().clawd) }
pub fn eye() -> Color { c(p().eye) }

pub fn s(fg: Color) -> Style { Style::new().fg(fg) }
pub fn bold(fg: Color) -> Style { Style::new().fg(fg).add_modifier(Modifier::BOLD) }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ansi256_maps_extremes() {
        assert_eq!(to_ansi256(0, 0, 0), 16);
        assert_eq!(to_ansi256(255, 255, 255), 231);
        assert!(to_ansi256(0x71, 0x71, 0x7A) >= 232);
    }
}
