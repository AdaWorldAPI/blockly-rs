//! Cast a built-in template and print what it becomes.
//! `cargo run -p blockly-shim --example template_dump -- pong`
use blockly_shim::templates;
fn main() {
    let name = std::env::args().nth(1).unwrap_or_else(|| "pong".into());
    let json = templates::ALL
        .iter()
        .find(|(n, _)| *n == name)
        .expect("unknown template")
        .1;
    let scripts = blockly_shim::from_workspace_json(json).expect("parses");
    for (i, s) in scripts.iter().enumerate() {
        let prog = blockly_abi::lower_program(ogar_loco::LaneShape::Pairs, s).expect("casts");
        println!(
            "script {i} ({}) -> {} function(s)",
            s.ty,
            prog.functions.len()
        );
        for (fi, body) in prog.functions.iter().enumerate() {
            let calls: Vec<String> = blockly_abi::raise_calls(body)
                .iter()
                .map(|c| {
                    let n = blockly_abi::scratch::SCRATCH_DEVICE
                        .iter()
                        .find(|&&(_, b, ..)| b == c.function.0)
                        .map(|&(n, ..)| n.to_string())
                        .or_else(|| {
                            ogar_loco::vocabulary::shared_core::name(c.function).map(str::to_string)
                        })
                        .unwrap_or_else(|| format!("{:#04x}", c.function.0));
                    format!("{n}:{}", c.values[0])
                })
                .collect();
            println!("  fn {fi}  {}", calls.join("  "));
        }
    }
}
