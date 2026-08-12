//! Bake a template's JSON into its STORED NODES.
//!
//! `cargo run -p blockly-shim --example bake_template`
//!
//! Emits `templates/<name>.nodes`:each script's functions as 512-byte nodes,
//! concatenated. The JSON stays only as the authoring form.
use blockly_abi::{FunctionNode, lower_program};
use ogar_loco::LaneShape;

fn main() {
    for (name, json) in blockly_shim::templates::ALL {
        let scripts = blockly_shim::from_workspace_json(json).expect("parses");
        let mut out: Vec<u8> = Vec::new();
        let mut counts: Vec<u8> = Vec::new();
        for s in &scripts {
            let prog = lower_program(LaneShape::Pairs, s).expect("casts");
            counts.push(u8::try_from(prog.functions.len()).expect("bounded"));
            for (i, body) in prog.functions.iter().enumerate() {
                let mut key = [0u8; 16];
                key[0..4].copy_from_slice(&0x1717_FF00_u32.to_le_bytes());
                key[15] = u8::try_from(i).expect("bounded");
                out.extend_from_slice(&FunctionNode::new(key, *body).to_le_bytes());
            }
        }
        // Header: one byte of script count, then each script's function count.
        let mut file = vec![u8::try_from(scripts.len()).expect("bounded")];
        file.extend_from_slice(&counts);
        file.extend_from_slice(&out);
        let path = format!("crates/blockly-shim/templates/{name}.nodes");
        std::fs::write(&path, &file).expect("write");
        println!(
            "{path}: {} bytes ({} scripts, {:?} functions)",
            file.len(),
            scripts.len(),
            counts
        );
    }
}
