//! Measure the participation mask: a scene of N key-hat scripts (one per
//! key option, round-robin) plus a few always-on scripts, ONE key held.
//!
//! `cargo run --release -p blockly-shim --example bench_wake [scripts] [rounds] [slice]`
//!
//! Reports the per-round cost and how many scripts the mask actually wakes,
//! so the saving is read off the awake count rather than assumed. Without
//! the mask every one of the N scripts would run its slice every round.
use blockly_abi::{BlockRecord, FieldValue, lower_program};
use ogar_loco::LaneShape;
use std::time::Instant;

fn num(v: u8) -> BlockRecord {
    BlockRecord::leaf("math_number", "n").with_field("NUM", FieldValue::Byte(v))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(512);
    let rounds: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(2000);
    let slice: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);

    let keys = blockly_abi::menus::menu_by_id(1).expect("KEY_OPTION");
    // Every key option except `any` and the empty selection, so each hat
    // names ONE key and only one key's hats wake.
    let options: Vec<&str> = keys
        .options
        .iter()
        .copied()
        .filter(|k| *k != "any")
        .collect();
    let progs: Vec<_> = (0..n)
        .map(|i| {
            let key = options[i % options.len()];
            let script = BlockRecord::leaf("event_whenkeypressed", "h")
                .with_field("KEY_OPTION", FieldValue::Code(key.to_string()))
                .with_next(BlockRecord::leaf("control_forever", "f").with_statement(
                    "SUBSTACK",
                    BlockRecord::leaf("motion_changexby", "m").with_input("DX", num(1)),
                ));
            lower_program(LaneShape::Pairs, &script).expect("casts")
        })
        .collect();
    let bodies: Vec<&[ogar_loco::FunctionBody]> =
        progs.iter().map(|p| p.functions.as_slice()).collect();

    let held = blockly_abi::menus::encode(keys, options[0]);
    let stage = blockly_run::Stage {
        sprites: vec![blockly_run::Sprite::default(); n],
        key: held,
        ..blockly_run::Stage::default()
    };

    let mut scene = blockly_run::Scene::new(stage, bodies);
    let awake = scene.awake_count(held, None);
    let t = Instant::now();
    scene.run(rounds, slice).expect("runs");
    let dt = t.elapsed();
    let calls = u64::from(rounds) * awake as u64 * u64::from(slice);
    println!(
        "scripts {n} · awake per round {awake} ({:.1}%) · rounds {rounds} × slice {slice}: {:.1} ms total, {:.2} µs/round, {:.1} ns per executed call",
        100.0 * awake as f64 / n as f64,
        dt.as_secs_f64() * 1e3,
        dt.as_secs_f64() * 1e6 / f64::from(rounds),
        dt.as_secs_f64() * 1e9 / calls as f64,
    );
}
