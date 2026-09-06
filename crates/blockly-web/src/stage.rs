//! Render a scene as an ANIMATED SVG — server-side, from the run's own trace.
//!
//! # Why animated, and why in the SVG
//!
//! A run that reports only its final stage draws one frame, and one frame of a
//! moving program is indistinguishable from a program that does nothing. The
//! deployed demo showed exactly that: a frozen coordinate system.
//!
//! The fix is not a client-side simulation — that would be a second
//! interpreter in JavaScript, i.e. the thing this whole arc exists to avoid.
//! Instead the run records a TRACE, and the SVG animates it natively with
//! `<animate>` keyframes. One request, no JSON, no client loop: the server
//! projects, the browser plays what it was given.

use blockly_run::{Look, Stage};

// Colours as constants: a `#rrggbb` literal inside a `format!` string parses
// as a reserved prefix (Rust 2021), so they are named rather than inlined.
const BALL: &str = "#4C97FF";
const PADDLE: &str = "#FFAB19";
const BG: &str = "#f9f9f9";
const EDGE: &str = "#88888844";
const GRID: &str = "#88888822";

/// Scratch coords (centre origin, y up) → SVG (top-left origin, y down).
fn to_svg(s: &Stage, x: f32, y: f32) -> (f32, f32) {
    (x + s.half_w, s.half_h - y)
}

/// Join one sprite's coordinate across the whole trace, as SVG `values`.
fn track(trace: &[Stage], i: usize, axis_x: bool) -> String {
    trace
        .iter()
        .map(|s| {
            let sp = s.sprites.get(i).cloned().unwrap_or_default();
            let (cx, cy) = to_svg(s, sp.x, sp.y);
            format!("{:.1}", if axis_x { cx } else { cy })
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// Draw the scene, animating every sprite along the recorded trace.
///
/// `secs` is the wall-clock duration the animation plays over — a property of
/// the VIEWING, never of the program.
#[must_use]
pub fn svg(trace: &[Stage], secs: f32) -> String {
    let Some(last) = trace.last() else {
        return String::new();
    };
    let w = last.half_w * 2.0;
    let h = last.half_h * 2.0;
    let animate = trace.len() > 1;

    let mut actors = String::new();
    for (i, sp) in last.sprites.iter().enumerate() {
        if !sp.visible {
            continue;
        }
        let (cx, cy) = to_svg(last, sp.x, sp.y);
        let (xs, ys) = (track(trace, i, true), track(trace, i, false));
        // `repeatCount=indefinite` loops the recorded run so the stage keeps
        // moving instead of freezing on the last frame the moment it ends.
        let anim = |attr: &str, vals: &str| {
            if animate {
                format!(
                    "<animate attributeName=\"{attr}\" values=\"{vals}\" \
                     dur=\"{secs}s\" repeatCount=\"indefinite\"/>"
                )
            } else {
                String::new()
            }
        };
        match sp.look {
            Look::Ball => {
                let r = (9.0 * sp.size / 100.0).max(3.0);
                actors.push_str(&format!(
                    "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"{BALL}\">{}{}</circle>",
                    anim("cx", &xs),
                    anim("cy", &ys)
                ));
            }
            Look::Paddle => {
                // A paddle is tall and thin, and it is drawn from its CENTRE,
                // so the animated y is the same quantity the ball uses.
                let (pw, ph) = (10.0, 64.0 * sp.size / 100.0);
                actors.push_str(&format!(
                    "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{pw}\" height=\"{ph:.1}\" rx=\"4\" fill=\"{PADDLE}\">{}{}</rect>",
                    cx - pw / 2.0,
                    cy - ph / 2.0,
                    anim("x", &track_offset(trace, i, true, -pw / 2.0)),
                    anim("y", &track_offset(trace, i, false, -ph / 2.0))
                ));
            }
        }
    }

    format!(
        "<svg viewBox=\"0 0 {w} {h}\" width=\"100%\" \
         style=\"max-height:260px;background:{BG};border:1px solid {EDGE};border-radius:6px\">\
         <line x1=\"{hw}\" y1=\"0\" x2=\"{hw}\" y2=\"{h}\" stroke=\"{GRID}\"/>\
         <line x1=\"0\" y1=\"{hh}\" x2=\"{w}\" y2=\"{hh}\" stroke=\"{GRID}\"/>\
         {actors}</svg>",
        hw = last.half_w,
        hh = last.half_h
    )
}

/// [`track`] shifted by a constant — rects are positioned by their corner.
fn track_offset(trace: &[Stage], i: usize, axis_x: bool, delta: f32) -> String {
    trace
        .iter()
        .map(|s| {
            let sp = s.sprites.get(i).cloned().unwrap_or_default();
            let (cx, cy) = to_svg(s, sp.x, sp.y);
            format!("{:.1}", (if axis_x { cx } else { cy }) + delta)
        })
        .collect::<Vec<_>>()
        .join(";")
}

/// The run's numbers, as readable HTML beside the picture.
///
/// Outside the SVG on purpose: inside, text inherits the viewBox scaling and
/// a 480-wide stage in a side panel renders it illegibly small.
#[must_use]
pub fn stats(last: &Stage, frames: usize) -> String {
    let ball = last.sprites.first().cloned().unwrap_or_default();
    format!(
        "<div class=\"stage-stats\"><code>ball {:.0},{:.0}</code> \
         <code>dir {:.0}°</code> <code>paddle y {:.0}</code> \
         <code>score {:.0}</code> <code>t {:.2}s</code> \
         <code>{frames} frames</code></div>",
        ball.x,
        ball.y,
        ball.direction,
        last.sprites.get(1).map_or(0.0, |p| p.y),
        last.var(0),
        last.timer
    )
}
