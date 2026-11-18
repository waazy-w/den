use crate::theme;
use ratatui::style::Style;
use ratatui::text::Span;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub const FADE_IN: Duration = Duration::from_millis(260);
pub const FLASH: Duration = Duration::from_millis(600);
pub const OPEN: Duration = Duration::from_millis(160);
pub const SWITCH: Duration = Duration::from_millis(220);
pub const PULSE: Duration = Duration::from_millis(2400);

static EPOCH: OnceLock<Instant> = OnceLock::new();

fn epoch() -> Instant {
    *EPOCH.get_or_init(Instant::now)
}

pub fn secs() -> f32 {
    epoch().elapsed().as_secs_f32()
}

pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// 0→1 progress of an animation that started at `since`.
pub fn progress(since: Option<Instant>, dur: Duration) -> f32 {
    since.map_or(1.0, |s| (s.elapsed().as_secs_f32() / dur.as_secs_f32()).min(1.0))
}

pub fn active(since: Option<Instant>, dur: Duration) -> bool {
    since.is_some_and(|s| s.elapsed() < dur)
}

const SPIN: [&str; 10] = ["·", "✢", "✳", "✶", "✻", "✽", "✻", "✶", "✳", "✢"];

pub fn spinner() -> &'static str {
    SPIN[((secs() / 0.11) as usize) % SPIN.len()]
}

/// Text with a soft highlight sweeping left to right.
pub fn shimmer(text: &str) -> Vec<Span<'static>> {
    if !theme::truecolor() {
        return vec![Span::styled(text.to_string(), theme::s(theme::accent()))];
    }
    let n = text.chars().count() as f32;
    let period = 1.6;
    let pos = (secs() % period) / period * (n + 6.0) - 3.0;
    text.chars()
        .enumerate()
        .map(|(i, c)| {
            let d = (i as f32 - pos).abs();
            let glow = (1.0 - d / 3.0).max(0.0);
            Span::styled(c.to_string(), Style::new().fg(theme::mix(theme::accent_rgb(), theme::accent_hi_rgb(), glow)))
        })
        .collect()
}

/// Two soft pulses, then steady.
pub fn pulse(since: Option<Instant>) -> f32 {
    let Some(s) = since else { return 1.0 };
    let t = s.elapsed().as_secs_f32();
    if t >= PULSE.as_secs_f32() {
        return 1.0;
    }
    0.55 + 0.45 * (t * std::f32::consts::TAU / 1.2).cos()
}

/// Smoothly approaches `target`; returns the new value.
pub fn approach(cur: f32, target: f32, dt: f32, rate: f32) -> f32 {
    let k = 1.0 - (-rate * dt).exp();
    let next = cur + (target - cur) * k;
    if (target - next).abs() < 0.5 { target } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_bounds() {
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert!(ease_out(0.5) > 0.5);
    }

    #[test]
    fn approach_converges() {
        let mut v = 0.0;
        for _ in 0..200 {
            v = approach(v, 100.0, 0.016, 18.0);
        }
        assert_eq!(v, 100.0);
    }
}
