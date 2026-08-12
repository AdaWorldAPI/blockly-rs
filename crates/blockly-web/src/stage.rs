//! Render a run's stage as SVG — server-side, like every other surface here.
//!
//! The stage is drawn from the [`Stage`](blockly_run::Stage) an actual RUN
//! produced, so what the page shows is the result of executing the stored
//! bytes rather than an animation of the block layout. Rendering server-side
//! keeps the same posture as `/api/surface`: the server projects, the client
//! displays.

use blockly_run::Stage;

// Colours as constants: a `#rrggbb` literal inside a `format!` string is
// parsed as a reserved prefix (Rust 2021), so they are named rather than
// inlined — and naming them is better anyway.
const BALL: &str = "#4C97FF";
const INK: &str = "#333333";
const BG: &str = "#f9f9f9";
const EDGE: &str = "#88888844";
const GRID: &str = "#88888822";

/// Draw the stage. Scratch coordinates (centre origin, y up) map to SVG's
/// top-left origin with y down, which is the one conversion this does.
#[must_use]
pub fn svg(stage: &Stage, ran: bool) -> String {
    let w = stage.half_w * 2.0;
    let h = stage.half_h * 2.0;
    let cx = stage.x + stage.half_w;
    let cy = stage.half_h - stage.y;
    let r = (8.0 * stage.size / 100.0).max(2.0);

    // The sprite carries its heading, so a reader can see the direction the
    // motion ops are actually maintaining.
    let rad = stage.direction.to_radians();
    let (hx, hy) = (cx + rad.sin() * r * 2.2, cy - rad.cos() * r * 2.2);

    let body = if stage.visible {
        format!(
            "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"{BALL}\"/>\n  \
             <line x1=\"{cx:.1}\" y1=\"{cy:.1}\" x2=\"{hx:.1}\" y2=\"{hy:.1}\" \
             stroke=\"{INK}\" stroke-width=\"1.5\"/>"
        )
    } else {
        String::new()
    };

    // The stats live OUTSIDE the SVG. Inside, they inherit the viewBox
    // scaling — a 480-wide stage squeezed into a panel renders 11px text at
    // roughly a third of that, which is what the deployed page showed:
    // present, and unreadable. HTML below the picture stays at page size.
    let note = if ran {
        format!(
            "<code>x {:.0}</code> <code>y {:.0}</code> <code>dir {:.0}°</code> \
             <code>var {:.0}</code> <code>t {:.2}s</code>",
            stage.x, stage.y, stage.direction, stage.var, stage.timer
        )
    } else {
        "<span class=\"muted\">not run</span>".to_string()
    };

    format!(
        "<svg viewBox=\"0 0 {w} {h}\" width=\"100%\" \
         style=\"max-height:240px;background:{BG};border:1px solid {EDGE};border-radius:6px\">\n  \
         <line x1=\"{hw}\" y1=\"0\" x2=\"{hw}\" y2=\"{h}\" stroke=\"{GRID}\"/>\n  \
         <line x1=\"0\" y1=\"{hh}\" x2=\"{w}\" y2=\"{hh}\" stroke=\"{GRID}\"/>\n  \
         {body}\n\
         </svg>\n\
         <div class=\"stage-stats\">{note}</div>",
        hw = stage.half_w,
        hh = stage.half_h
    )
}
