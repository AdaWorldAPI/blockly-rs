//! Measure how much of a real `.sb3` project's JSON this pipeline casts.
//!
//! `cargo run -p blockly-shim --example sb3_coverage -- <project.json> [more.json ...]`
//!
//! Not a test: a MEASUREMENT tool, run against real Scratch project JSON to
//! see how far `blockly_shim::sb3` + `blockly_abi::lower_program_in` reach
//! before refusing, and why they refuse where they do.

use blockly_abi::{BlockRecord, CastError};
use blockly_shim::sb3::{Sb3Project, Sb3Target, from_project_json, target_basin};
use std::collections::BTreeMap;

/// Recursively count the `BlockRecord` nodes in a subtree: itself, its
/// nested `inputs`, its `statements`, and its `next` chain.
fn records(b: &BlockRecord) -> usize {
    1 + b.inputs.iter().map(|(_, c)| records(c)).sum::<usize>()
        + b.statements.iter().map(|(_, c)| records(c)).sum::<usize>()
        + b.next.as_deref().map_or(0, records)
}

/// The `CastError` variant's leading identifier, e.g. `"UnknownOpcode"` from
/// `UnknownOpcode { ty: .. }`.
fn variant_name(e: &CastError) -> String {
    let s = format!("{e:?}");
    s.split(|c: char| c == '{' || c == '(' || c.is_whitespace())
        .next()
        .unwrap_or("?")
        .to_string()
}

/// The block type named inside a `CastError`, when the variant carries one.
///
/// `TooManyFunctions` and `Body` carry no `ty` field at all — those fall
/// back to `"?"`.
fn error_ty(e: &CastError) -> &str {
    match e {
        CastError::UnknownOpcode { ty, .. }
        | CastError::WideLiteral { ty, .. }
        | CastError::UnresolvedRef { ty, .. }
        | CastError::MutatorUnsupported { ty }
        | CastError::UnencodedValueParam { ty, .. }
        | CastError::ConstantPool { ty, .. }
        | CastError::UnexpectedStatements { ty, .. }
        | CastError::ShapeTooNarrow { ty, .. }
        | CastError::TooManyValues { ty, .. } => ty,
        CastError::TooManyFunctions { .. } | CastError::Body(_) => "?",
    }
}

/// Walk a subtree collecting every distinct block `type` it and its
/// descendants carry.
fn collect_opcodes(b: &BlockRecord, into: &mut BTreeMap<String, usize>) {
    *into.entry(b.ty.clone()).or_insert(0) += 1;
    for (_, c) in &b.inputs {
        collect_opcodes(c, into);
    }
    for (_, c) in &b.statements {
        collect_opcodes(c, into);
    }
    if let Some(n) = &b.next {
        collect_opcodes(n, into);
    }
}

/// Whether a block type is castable at all, independent of any particular
/// script's shape — a codebook/scratch-vocabulary lookup, not a cast.
fn opcode_resolvable(ty: &str) -> bool {
    if blockly_abi::codebook::resolve(ty, None).is_some() {
        return true;
    }
    if blockly_abi::scratch::resolve_scratch(ty, None).is_some() {
        return true;
    }
    // `operator_mathop` resolves only with a selector code attached; probe
    // one real one rather than reporting a mathop reporter as unresolvable
    // for lacking a code it always carries in practice.
    if ty == "operator_mathop" && blockly_abi::scratch::resolve_scratch(ty, Some("abs")).is_some() {
        return true;
    }
    false
}

/// Per-file coverage counters, accumulated while walking a project's targets.
#[derive(Default)]
struct Counts {
    targets: usize,
    stages: usize,
    scripts: usize,
    scripts_ok: usize,
    blocks_total: usize,
    sb3_objects: usize,
    blocks_ok: usize,
    blocks_refused: usize,
    refusals: BTreeMap<(String, String), usize>,
    opcodes: BTreeMap<String, usize>,
}

/// Measure one target: cast every one of its scripts against its own basin.
/// `Triples`, because `if/else` carries two body references and `Pairs`
/// refuses it as `ShapeTooNarrow` — a shape limit, not a vocabulary gap.
fn measure_target(project: &Sb3Project, target: &Sb3Target, out: &mut Counts) {
    out.targets += 1;
    if target.is_stage {
        out.stages += 1;
    }
    let basin = target_basin(project, target);
    // `block_count` is sb3's own object count (shadows included, inline
    // primitives excluded); the block totals below count `BlockRecord` nodes
    // so that "in ok" and "total" are the same unit.
    out.sb3_objects += target.block_count;
    for script in &target.scripts {
        out.scripts += 1;
        out.blocks_total += records(script);
        collect_opcodes(script, &mut out.opcodes);
        match blockly_abi::lower_program_in(ogar_loco::LaneShape::Triples, script, &basin) {
            Ok(_) => {
                out.scripts_ok += 1;
                out.blocks_ok += records(script);
            }
            Err(e) => {
                out.blocks_refused += records(script);
                let key = (variant_name(&e), error_ty(&e).to_string());
                *out.refusals.entry(key).or_insert(0) += 1;
            }
        }
    }
}

/// One percentage, guarded against division by zero.
fn pct(n: usize, of: usize) -> f64 {
    if of == 0 {
        0.0
    } else {
        100.0 * n as f64 / of as f64
    }
}

/// Print the per-file report in the pinned shape.
fn report(name: &str, c: &Counts) {
    println!("== {name}");
    println!(
        "targets {:>8}   ({} stage, {} sprites)",
        c.targets,
        c.stages,
        c.targets - c.stages
    );
    println!(
        "scripts {:>8}   cast ok {:>8}   ({:.1}%)",
        c.scripts,
        c.scripts_ok,
        pct(c.scripts_ok, c.scripts)
    );
    println!(
        "blocks  {:>8}   in ok scripts {:>8} ({:.1}%)   in refused {:>8}   (sb3 objects {}, shadows incl.)",
        c.blocks_total,
        c.blocks_ok,
        pct(c.blocks_ok, c.blocks_total),
        c.blocks_refused,
        c.sb3_objects
    );
    println!("refusals by kind:");
    let mut rows: Vec<(&(String, String), &usize)> = c.refusals.iter().collect();
    rows.sort_by(|a, b| b.1.cmp(a.1));
    for ((variant, ty), n) in rows.into_iter().take(20) {
        println!("   {variant:<22}{ty:<24}{n:>8}");
    }
    println!("unresolvable opcodes (never castable, any script):");
    let mut unresolved: Vec<(&String, &usize)> = c
        .opcodes
        .iter()
        .filter(|(ty, _)| !opcode_resolvable(ty))
        .collect();
    unresolved.sort_by(|a, b| b.1.cmp(a.1));
    for (ty, n) in unresolved {
        println!("   {ty:<26}{n:>8}");
    }
}

/// Dump per-script head opcodes (and, for refused scripts, the error) for one
/// named target — used to inspect specific failures.
fn dump_target(target: &Sb3Target, project: &Sb3Project) {
    println!("-- dump target `{}`", target.name);
    let basin = target_basin(project, target);
    for (i, script) in target.scripts.iter().enumerate() {
        match blockly_abi::lower_program_in(ogar_loco::LaneShape::Triples, script, &basin) {
            Ok(_) => println!("   [{i}] {} ok", script.ty),
            Err(e) => println!("   [{i}] {} REFUSED: {e:?}", script.ty),
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut files: Vec<String> = Vec::new();
    let mut dump_name: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--dump" {
            dump_name = args.next();
        } else {
            files.push(a);
        }
    }

    let mut total_scripts = 0usize;
    let mut total_scripts_ok = 0usize;
    let mut total_blocks = 0usize;
    let mut total_blocks_ok = 0usize;

    for file in &files {
        let json = match std::fs::read_to_string(file) {
            Ok(j) => j,
            Err(e) => {
                println!("{file}: {e}");
                continue;
            }
        };
        let project = match from_project_json(&json) {
            Ok(p) => p,
            Err(e) => {
                println!("{file}: {e}");
                continue;
            }
        };

        let mut counts = Counts::default();
        for target in &project.targets {
            measure_target(&project, target, &mut counts);
        }
        report(file, &counts);

        if let Some(name) = &dump_name
            && let Some(t) = project.targets.iter().find(|t| &t.name == name)
        {
            dump_target(t, &project);
        }

        total_scripts += counts.scripts;
        total_scripts_ok += counts.scripts_ok;
        total_blocks += counts.blocks_total;
        total_blocks_ok += counts.blocks_ok;
    }

    println!(
        "TOTAL scripts {} ok {} ({:.1}%)  blocks {} in-ok {} ({:.1}%)",
        total_scripts,
        total_scripts_ok,
        pct(total_scripts_ok, total_scripts),
        total_blocks,
        total_blocks_ok,
        pct(total_blocks_ok, total_blocks)
    );

    Ok(())
}
