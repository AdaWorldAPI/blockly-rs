//! Bake a template's JSON into its STORED NODES.
//!
//! `cargo run -p blockly-store --example bake_template`
//!
//! Emits `crates/blockly-shim/templates/<name>.nodes`: each script's
//! functions as 512-byte V3 rows, concatenated. The JSON stays only as the
//! authoring form.
//!
//! **The committed `.nodes` fixtures are STALE with respect to this
//! example**: they were baked before the class was registered on the V3
//! tail, so their keys carry the old `0x1717_FF00` classid and a V1
//! `family:identity` tail. Nothing reads them wrongly —
//! `templates::raise_nodes` reconstructs BODIES and never looks at a key,
//! which is why the bake-reproduces-the-JSON test stays green either way —
//! but re-running this example is owed before the keys can be trusted as
//! addresses. It was not run in the session that made the change.
//!
//! It lives in `blockly-store` rather than beside the templates because
//! baking a node means MINTING ITS KEY, and minting is this crate's job —
//! the version that lived in `blockly-shim` spelled the key out
//! (`key[0..4] = classid; key[15] = i`), which is byte math on a layout the
//! substrate owns. That spelling also put the function index in the MOST
//! significant byte of the 24-bit identity (`identity = i << 16`), so the
//! keys in a pre-existing bake differ from the minted ones. The BODIES are
//! untouched, which is what `the_baked_nodes_reproduce_the_authoring_json_
//! byte_for_byte` compares.
use blockly_store::{CLASSID, mint_key};
use ogar_loco::LaneShape;

fn main() {
    let cid = CLASSID;
    for (name, json) in blockly_shim::templates::ALL {
        let scripts = blockly_shim::from_workspace_json(json).expect("parses");
        let mut out: Vec<u8> = Vec::new();
        let mut counts: Vec<u8> = Vec::new();
        for s in &scripts {
            let prog = blockly_shim::templates::cast(LaneShape::Pairs, s).expect("casts");
            counts.push(u8::try_from(prog.functions.len()).expect("bounded"));
            let rows = blockly_store::ProgramRows::from_program(&prog, cid).expect("lays out");
            out.extend_from_slice(rows.as_le_bytes());
        }
        // Header: one byte of script count, then each script's function count.
        let mut file = vec![u8::try_from(scripts.len()).expect("bounded")];
        file.extend_from_slice(&counts);
        file.extend_from_slice(&out);
        let path = format!("crates/blockly-shim/templates/{name}.nodes");
        std::fs::write(&path, &file).expect("write");
        println!(
            "{path}: {} bytes ({} scripts, {:?} functions, key0 {:02x?})",
            file.len(),
            scripts.len(),
            counts,
            mint_key(cid, 0).as_bytes()
        );
    }
}
