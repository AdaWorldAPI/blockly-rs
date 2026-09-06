//! RUN a real `.sb3` project's JSON: cast every target's scripts against its
//! own registers, then execute — and measure what the runtime refuses.
//!
//! `cargo run --release -p blockly-shim --example sb3_run -- <project.json> [rounds] [slice]`
//!
//! Two measurements, kept apart because they answer different questions:
//! 1. **Per-script**: each cast script runs alone for one slice; the first
//!    refusal (an `Unimplemented` byte, a missing register entry, …) is
//!    tallied by name. This is runtime COVERAGE — which operations a real
//!    project reaches that the interpreter does not perform.
//! 2. **Whole scene**: every script in one `Scene` with per-sprite
//!    registers, owners bound, run for `rounds`; reports how far it got.
use blockly_abi::LoweringContext;
use blockly_shim::sb3::{from_project_json, target_basin};
use ogar_loco::LaneShape;
use std::collections::BTreeMap;

fn op_name(b: u8) -> String {
    if let Some((n, ..)) = blockly_abi::scratch::device_by_byte(b) {
        return n.to_string();
    }
    ogar_loco::vocabulary::shared_core::name(ogar_loco::FnIndex(b))
        .map_or_else(|| format!("{b:#04x}"), str::to_string)
}

fn err_key(e: &blockly_run::RunError) -> String {
    use blockly_run::RunError as E;
    match e {
        E::Unimplemented(b) => format!("Unimplemented {}", op_name(*b)),
        E::Uncovered(b) => format!("Uncovered {}", op_name(*b)),
        E::StackUnderflow(b) => format!("StackUnderflow {}", op_name(*b)),
        E::NotAList(b) => format!("NotAList {}", op_name(*b)),
        E::DanglingReference(i) => format!("DanglingReference {i}"),
        E::UnknownProcedure(i) => format!("UnknownProcedure {i}"),
        E::MissingConstant(i) => format!("MissingConstant {i}"),
        E::UnknownText(i) => format!("UnknownText {i}"),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: sb3_run <project.json> [rounds] [slice]")?;
    let rounds: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(120);
    let slice: u32 = args.next().and_then(|a| a.parse().ok()).unwrap_or(200);
    let json = std::fs::read_to_string(&path)?;
    let project = from_project_json(&json)?;

    // One basin + one pool per target; every cast script remembers its owner.
    let basins: Vec<_> = project
        .targets
        .iter()
        .map(|t| target_basin(&project, t))
        .collect();
    let mut ctxs: Vec<LoweringContext> = project
        .targets
        .iter()
        .map(|_| LoweringContext::placeholder())
        .collect();
    let mut progs = Vec::new();
    let mut owners = Vec::new();
    let mut cast_refused = 0usize;
    for (ti, t) in project.targets.iter().enumerate() {
        for s in &t.scripts {
            match blockly_abi::lower_program_with_pool(
                LaneShape::Triples,
                s,
                &basins[ti],
                &mut ctxs[ti],
            ) {
                Ok(p) => {
                    progs.push(p);
                    owners.push(ti);
                }
                Err(_) => cast_refused += 1,
            }
        }
    }
    println!(
        "targets {}  scripts cast {}  cast refused {}",
        project.targets.len(),
        progs.len(),
        cast_refused
    );

    // ── 1. per-script runtime coverage ──────────────────────────────────
    // Every definition is callable, scoped to its owner sprite — otherwise
    // the tally is dominated by `UnknownProcedure`, an artifact of running
    // a script alone rather than a gap in the interpreter.
    let procs: Vec<blockly_run::Procedure> = progs
        .iter()
        .zip(&owners)
        .filter_map(|(p, &o)| {
            blockly_run::Procedure::of_script(&p.functions)
                .map(|pr| blockly_run::Procedure { owner: o, ..pr })
        })
        .collect();
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut ran_clean = 0usize;
    for (p, &owner) in progs.iter().zip(&owners) {
        if blockly_run::Procedure::of_script(&p.functions).is_some() {
            ran_clean += 1; // a definition runs when called, never alone
            continue;
        }
        let stage = blockly_run::Stage {
            sprites: vec![blockly_run::Sprite::default(); project.targets.len()],
            current: owner,
            ..blockly_run::Stage::default()
        };
        let mut m = blockly_run::Machine::resuming(&p.functions, slice, stage, owner)
            .with_procs(&procs)
            .with_scoped_procs(true)
            .with_basin(&basins[owner])
            .with_pool(&ctxs[owner].pool);
        match m.run() {
            Ok(()) => ran_clean += 1,
            Err(e) => *tally.entry(err_key(&e)).or_insert(0) += 1,
        }
    }
    println!(
        "per-script: {} of {} run a full slice without refusal ({:.1}%)",
        ran_clean,
        progs.len(),
        100.0 * ran_clean as f64 / progs.len().max(1) as f64
    );
    let mut rows: Vec<_> = tally.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (k, n) in rows.iter().take(25) {
        println!("   {n:>5}  {k}");
    }

    // ── 2. the whole scene, per-sprite registers, owners bound ──────────
    let bodies: Vec<&[ogar_loco::FunctionBody]> =
        progs.iter().map(|p| p.functions.as_slice()).collect();
    let stage = blockly_run::Stage {
        sprites: vec![blockly_run::Sprite::default(); project.targets.len()],
        ..blockly_run::Stage::default()
    };
    let mut scene = blockly_run::Scene::new(stage, bodies).with_owners(&owners);
    for (i, (b, c)) in basins.iter().zip(&ctxs).enumerate() {
        scene = scene.with_sprite_registers(i, b, &c.pool);
    }
    let t = std::time::Instant::now();
    let mut done = 0u32;
    let mut first_err = None;
    for _ in 0..rounds {
        match scene.run(1, slice) {
            Ok(()) => done += 1,
            Err(e) => {
                first_err = Some(e);
                break;
            }
        }
    }
    println!(
        "scene: {done}/{rounds} rounds in {:.1} ms{}",
        t.elapsed().as_secs_f64() * 1e3,
        first_err.map_or(String::new(), |e| format!(" — stopped: {}", err_key(&e)))
    );
    Ok(())
}
