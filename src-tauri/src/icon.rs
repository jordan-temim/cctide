//! Runtime tray-icon rendering: the "CC" gauge whose two C's fill with the live
//! session (left) and weekly (right) usage.
//!
//! Two platform renders (tray icons differ a lot across OSes):
//! - **macOS**: wide, monochrome *template* (auto-tinted). Fill is shown by a
//!   thick filled arc over a thin track; alert is conveyed by blinking (the
//!   filled arc is dropped on the `blink_off` frame).
//! - **Windows/Linux**: square, colour. Each C's filled arc is tinted by its
//!   alert tier (neutral/green/orange/red) over a grey track; no blink.
//!
//! Geometry mirrors `scripts/gen-icon.mjs` (arc 40°..320°, two C centres,
//! 3×3 supersampling). Zero external deps.

use std::f64::consts::PI;

const A0: f64 = 40.0 * PI / 180.0; // bottom-right tip
const A1: f64 = 320.0 * PI / 180.0; // top-right tip
const SWEEP: f64 = A1 - A0;
const TAU: f64 = 2.0 * PI;

pub struct IconParams {
    pub session_fill: f64, // 0..1
    pub weekly_fill: f64,  // 0..1
    pub session_tier: u8,  // 0..3
    pub weekly_tier: u8,   // 0..3
    pub blink_off: bool,   // macOS blink frame: drop the filled arc
    pub disabled: bool,    // tracking off: tracks only + diagonal slash
}

pub struct RenderedIcon {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Windows/Linux colour per alert tier.
fn tier_color(tier: u8) -> [u8; 3] {
    match tier {
        1 => [46, 158, 107], // green
        2 => [224, 138, 59], // orange
        3 => [208, 57, 43],  // red
        _ => [217, 119, 87], // neutral terracotta
    }
}

struct Geom {
    w: u32,
    h: u32,
    r: f64,
    t: f64,
    cy: f64,
    cx_left: f64,
    cx_right: f64,
    mono: bool,
    track: [u8; 3],
}

#[cfg(target_os = "macos")]
fn geom() -> Geom {
    // Wide canvas so the C's fill the menu-bar height.
    Geom {
        w: 440,
        h: 256,
        r: 92.0,
        t: 36.0,
        cy: 128.0,
        cx_left: 120.0,
        cx_right: 320.0,
        mono: true,
        track: [0, 0, 0],
    }
}

#[cfg(not(target_os = "macos"))]
fn geom() -> Geom {
    // Square canvas for the (square) Windows notification area.
    Geom {
        w: 256,
        h: 256,
        r: 52.0,
        t: 20.0,
        cy: 128.0,
        cx_left: 80.0,
        cx_right: 176.0,
        mono: false,
        track: [120, 120, 124],
    }
}

struct C {
    cx: f64,
    fill: f64,
    color: [u8; 3],
}

/// Colour+alpha for a single sample point, or transparent.
fn sample(px: f64, py: f64, g: &Geom, cs: &[C; 2], blink_off: bool, disabled: bool) -> [f64; 4] {
    for c in cs {
        let dx = px - c.cx;
        let dy = py - g.cy;
        let dist = (dx * dx + dy * dy).sqrt();
        let mut a = dy.atan2(dx);
        if a < 0.0 {
            a += TAU;
        }
        if !(A0..=A1).contains(&a) {
            continue;
        }
        let t = (a - A0) / SWEEP;
        if g.mono {
            let suppress_fill = blink_off || disabled;
            let on_fill = !suppress_fill && t <= c.fill && (dist - g.r).abs() <= g.t / 2.0;
            let on_track = (dist - g.r).abs() <= g.t * 0.22;
            if on_fill || on_track {
                return [0.0, 0.0, 0.0, 255.0];
            }
        } else if (dist - g.r).abs() <= g.t / 2.0 {
            let col = if !disabled && t <= c.fill {
                c.color
            } else {
                g.track
            };
            return [col[0] as f64, col[1] as f64, col[2] as f64, 255.0];
        }
    }
    // Diagonal slash from top-right to bottom-left when disabled.
    if disabled {
        let w = g.w as f64;
        let h = g.h as f64;
        let len = (w * w + h * h).sqrt();
        let dist = (px * h + py * w - w * h).abs() / len;
        if dist <= g.t * 0.22 {
            let col = g.track;
            return [col[0] as f64, col[1] as f64, col[2] as f64, 255.0];
        }
    }
    [0.0, 0.0, 0.0, 0.0]
}

/// Renders the icon for the given params.
pub fn render(p: &IconParams) -> RenderedIcon {
    let g = geom();
    let cs = [
        C {
            cx: g.cx_left,
            fill: p.session_fill.clamp(0.0, 1.0),
            color: tier_color(p.session_tier),
        },
        C {
            cx: g.cx_right,
            fill: p.weekly_fill.clamp(0.0, 1.0),
            color: tier_color(p.weekly_tier),
        },
    ];

    let ss = 3usize;
    let mut rgba = vec![0u8; (g.w * g.h * 4) as usize];
    for y in 0..g.h {
        for x in 0..g.w {
            let (mut r, mut gg, mut b, mut al) = (0.0, 0.0, 0.0, 0.0);
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = x as f64 + (sx as f64 + 0.5) / ss as f64;
                    let fy = y as f64 + (sy as f64 + 0.5) / ss as f64;
                    let s = sample(fx, fy, &g, &cs, p.blink_off, p.disabled);
                    r += s[0] * s[3];
                    gg += s[1] * s[3];
                    b += s[2] * s[3];
                    al += s[3];
                }
            }
            let n = (ss * ss) as f64;
            let o = ((y * g.w + x) * 4) as usize;
            if al > 0.0 {
                rgba[o] = (r / al).round() as u8;
                rgba[o + 1] = (gg / al).round() as u8;
                rgba[o + 2] = (b / al).round() as u8;
            }
            rgba[o + 3] = (al / n).round() as u8;
        }
    }

    RenderedIcon {
        rgba,
        width: g.w,
        height: g.h,
    }
}
