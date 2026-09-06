//! Measure the interpreter: ns per executed call on the real Pong (keys)
//! scene, release build.
//!
//! `cargo run --release -p blockly-shim --example bench_run [rounds] [slice]`
//!
//! The scene's `forever` scripts consume their whole slice every round, so
//! calls ≈ rounds × scheduled scripts × slice; the exact count is read from
//! the machine's budget accounting via `Scene::calls_executed` when present.
use blockly_shim::templates;
use ogar_loco::LaneShape;
use std::time::Instant;

fn main() {
    let mut args = std::env::args().skip(1);
    let rounds: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(2000);
    let slice: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(500);

    let scripts = templates::raise_nodes(templates::PONG_KEYS_NODES).expect("raises");
    let progs: Vec<_> = scripts
        .iter()
        .map(|s| templates::cast(LaneShape::Pairs, s).expect("casts"))
        .collect();
    let bodies: Vec<&[ogar_loco::FunctionBody]> =
        progs.iter().map(|p| p.functions.as_slice()).collect();
    let scheduled = bodies.len();

    // Warm once, then time.
    let mut warm =
        blockly_run::Scene::new(blockly_run::Stage::pong(), bodies.clone()).with_key_sweep(30);
    warm.run(50, slice).expect("runs");

    let t = Instant::now();
    let mut scene = blockly_run::Scene::new(blockly_run::Stage::pong(), bodies).with_key_sweep(30);
    scene.run(rounds, slice).expect("runs");
    let dt = t.elapsed();
    let calls = u64::from(rounds) * scheduled as u64 * u64::from(slice);
    println!(
        "rounds {rounds} × scripts {scheduled} × slice {slice} ≈ {calls} calls in {:.1} ms → {:.1} ns/call ({} trace frames, ball x {:.1})",
        dt.as_secs_f64() * 1e3,
        dt.as_secs_f64() * 1e9 / calls as f64,
        scene.trace().len(),
        scene.stage.sprites[0].x
    );
}
