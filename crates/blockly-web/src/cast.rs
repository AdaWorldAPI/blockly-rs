//! The demo's core: one workspace save → everything the panels show.
//!
//! Deliberately a plain function over plain data, so the whole surface is
//! unit-testable without HTTP. The handler in `main.rs` is a thin wrapper.
//!
//! # What the panels prove
//!
//! - **calls** — the post-order `(function : value)` rails per function, so
//!   the stack discipline is visible (`5 + 3` reads `NUMBER:5 NUMBER:3 ADD`).
//! - **node hex** — the entry function's full 512-byte stored node, 16 bytes
//!   per row: the opaque key, the zeroed reserved slot, and the value slab
//!   with its interleaved classid gaps — the layout, not a picture of it.
//! - **text** — the projection (`render_text`), plus the round-trip badge:
//!   `parse_text(render_text(body))` must reproduce the same 360 body bytes.
//!   Statement bodies have no text projection yet; the panel says so instead
//!   of pretending.
//! - **refusals** — the point of the demo. A mutated `controls_if`, an
//!   `IF_ELSE` under `Pairs`, a wide literal with no minted pool: each is a
//!   loud, named error instead of silently-wrong stored bytes.

use blockly_abi::{FunctionNode, Program};
use blockly_abi::{lower_program, parse_text, raise_calls, render_text};
use blockly_shim::from_workspace_json;
use ogar_blockly::{FunctionBody, LaneShape};
use serde::Serialize;

/// The demo key the entry node is stored under.
///
/// The classid half is real (`BlockConcept::Content` canon-high over an app
/// prefix); the prefix `0xFF00` is a PLACEHOLDER — the real `blockly-rs`
/// prefix is the unminted operator decision M1, and the page labels it so.
#[must_use]
pub fn demo_key() -> [u8; 16] {
    let mut k = [0u8; 16];
    k[0..4].copy_from_slice(&0x1701_FF00_u32.to_le_bytes());
    k[10..16].copy_from_slice(&[0, 0, 0, 0, 0, 1]);
    k
}

/// One function of one cast script, panel-ready.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct FunctionOut {
    /// The index a body-reference byte stores.
    pub index: usize,
    /// How many calls the body holds.
    pub len: usize,
    /// Each call as `NAMEish 0xFF:vv,vv,vv` — the rails, human-readably.
    pub calls: Vec<String>,
    /// The text projection, when the body has one.
    pub text: Option<String>,
    /// Why it has none, when it has none.
    pub text_error: Option<String>,
}

/// One top-level script (one block chain) after the cast.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ScriptOut {
    /// The functions the script became (index 0 = entry).
    pub functions: Vec<FunctionOut>,
    /// The entry function's 512-byte node, one 16-byte row per line.
    pub node_hex: Vec<String>,
    /// Whether the reserved slot (slot 1) is zeroed in the stored bytes.
    pub reserved_zeroed: bool,
    /// Whether every body-reference resolves (and none names the entry).
    pub resolvable: bool,
    /// `parse_text(render_text(entry))` reproduced the same body bytes.
    /// `None` when the entry has no text projection to round-trip.
    pub roundtrip: Option<bool>,
}

/// Everything one cast produces.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CastOut {
    /// The lane shape the cast ran under.
    pub shape: String,
    /// One entry per top-level script that cast cleanly.
    pub scripts: Vec<ScriptOut>,
    /// One entry per script that was REFUSED, with the reason.
    pub errors: Vec<String>,
}

fn shape_of(name: &str) -> LaneShape {
    match name {
        "triples" => LaneShape::Triples,
        "quads" => LaneShape::Quads,
        _ => LaneShape::Pairs,
    }
}

fn format_call(c: &blockly_abi::RaisedCall) -> String {
    // Values are shape-truncated by `raise_calls`, so a Pairs call shows one
    // immediate and a Quads call three — the rail width is visible as-is.
    let vals = c
        .values
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect::<Vec<_>>()
        .join(",");
    format!("{:#04x}:{vals}", c.function.0)
}

fn function_out(index: usize, body: &FunctionBody) -> FunctionOut {
    let (text, text_error) = match render_text(body) {
        Ok(t) => (Some(t), None),
        Err(e) => (None, Some(format!("{e:?}"))),
    };
    FunctionOut {
        index,
        len: body.len(),
        calls: raise_calls(body).iter().map(format_call).collect(),
        text,
        text_error,
    }
}

fn script_out(prog: &Program) -> ScriptOut {
    let entry = prog.entry();
    let node = FunctionNode::new(demo_key(), *entry);
    let bytes = node.to_le_bytes();
    let node_hex = bytes
        .chunks(16)
        .map(|row| {
            row.iter()
                .map(|b| format!("{b:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    let roundtrip = render_text(entry).ok().map(|t| {
        parse_text(&t, entry.shape())
            .map(|back| back.as_body_bytes() == entry.as_body_bytes())
            .unwrap_or(false)
    });
    ScriptOut {
        functions: prog
            .functions
            .iter()
            .enumerate()
            .map(|(i, b)| function_out(i, b))
            .collect(),
        node_hex,
        reserved_zeroed: FunctionNode::reserved_is_zeroed(&bytes),
        resolvable: prog.references_are_resolvable(blockly_abi::checked_vocabulary()),
        roundtrip,
    }
}

/// Cast a Blockly workspace save under a lane shape.
///
/// Never panics and never fails as a whole: per-script refusals land in
/// [`CastOut::errors`] with the block type and the reason, because the
/// refusals ARE the demo. A malformed save (not a workspace JSON at all)
/// yields one error and zero scripts.
#[must_use]
pub fn cast_workspace(json: &str, shape_name: &str) -> CastOut {
    let shape = shape_of(shape_name);
    let mut out = CastOut {
        shape: format!("{shape:?}"),
        scripts: Vec::new(),
        errors: Vec::new(),
    };
    let records = match from_workspace_json(json) {
        Ok(r) => r,
        Err(e) => {
            out.errors.push(format!("workspace refused: {e:?}"));
            return out;
        }
    };
    for record in &records {
        match lower_program(shape, record) {
            Ok(prog) => out.scripts.push(script_out(&prog)),
            Err(e) => out
                .errors
                .push(format!("script '{}' refused: {e:?}", record.ty)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `5 + 3` as Blockly's own save JSON (the exact shape `save()` emits).
    const ARITH: &str = r#"{"blocks":{"languageVersion":0,"blocks":[
        {"type":"math_arithmetic","id":"root","fields":{"OP":"ADD"},
         "inputs":{
           "A":{"block":{"type":"math_number","id":"a","fields":{"NUM":5}}},
           "B":{"block":{"type":"math_number","id":"b","fields":{"NUM":3}}}}}]}}"#;

    /// `if true { print 1 } else { print 2 }` — two body references.
    const IF_ELSE: &str = r#"{"blocks":{"languageVersion":0,"blocks":[
        {"type":"controls_ifelse","id":"ite",
         "inputs":{
           "IF0":{"block":{"type":"logic_boolean","id":"c","fields":{"BOOL":"TRUE"}}},
           "DO0":{"block":{"type":"text_print","id":"p1","inputs":{
             "TEXT":{"block":{"type":"math_number","id":"n1","fields":{"NUM":1}}}}}},
           "ELSE":{"block":{"type":"text_print","id":"p2","inputs":{
             "TEXT":{"block":{"type":"math_number","id":"n2","fields":{"NUM":2}}}}}}}}]}}"#;

    #[test]
    fn an_expression_casts_to_calls_text_node_and_a_green_roundtrip() {
        let out = cast_workspace(ARITH, "pairs");
        assert_eq!(out.errors, Vec::<String>::new());
        assert_eq!(out.scripts.len(), 1);
        let s = &out.scripts[0];
        assert_eq!(s.functions.len(), 1, "no control flow, one function");
        let f = &s.functions[0];
        // The stack discipline, visible: NUMBER:5, NUMBER:3, ADD.
        assert_eq!(f.calls, vec!["0x46:05", "0x46:03", "0x40:00"]);
        assert_eq!(f.text.as_deref(), Some("5 + 3"));
        assert_eq!(s.roundtrip, Some(true));
        assert!(s.resolvable);
        assert!(s.reserved_zeroed);
        // The node really is 512 bytes shown as 32 rows of 16.
        assert_eq!(s.node_hex.len(), 32);
        // Row 1 is the reserved slot and must read as zeros.
        assert_eq!(
            s.node_hex[1],
            "00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00"
        );
        // Row 2 begins the value slab: 4 classid zeros, then the first call —
        // the interleave, visible in the hex the user actually sees.
        assert!(s.node_hex[2].starts_with("00 00 00 00 46 05 46 03 40 00"));
    }

    #[test]
    fn if_else_is_refused_under_pairs_and_casts_under_triples() {
        // The refusal panel's headline case, two-sided.
        let refused = cast_workspace(IF_ELSE, "pairs");
        assert_eq!(refused.scripts.len(), 0);
        assert_eq!(refused.errors.len(), 1);
        assert!(
            refused.errors[0].contains("ShapeTooNarrow"),
            "the error must NAME the refusal: {}",
            refused.errors[0]
        );

        let ok = cast_workspace(IF_ELSE, "triples");
        assert_eq!(ok.errors, Vec::<String>::new());
        assert_eq!(ok.scripts.len(), 1);
        // Entry + then-arm + else-arm.
        assert_eq!(ok.scripts[0].functions.len(), 3);
        assert!(ok.scripts[0].resolvable);
        // A statement entry has no text projection — reported, not faked.
        assert_eq!(ok.scripts[0].functions[0].text, None);
        assert!(ok.scripts[0].functions[0].text_error.is_some());
        assert_eq!(ok.scripts[0].roundtrip, None);
    }

    #[test]
    fn an_empty_workspace_is_a_valid_empty_cast_and_garbage_is_a_named_refusal() {
        // `save()` omits absent serializers, so `{}` is a legal empty save.
        let empty = cast_workspace("{}", "pairs");
        assert_eq!(empty.scripts.len(), 0);
        assert_eq!(empty.errors, Vec::<String>::new());
        // …and non-JSON is refused with a reason, never a panic.
        let garbage = cast_workspace("not json", "pairs");
        assert_eq!(garbage.scripts.len(), 0);
        assert_eq!(garbage.errors.len(), 1);
        assert!(garbage.errors[0].starts_with("workspace refused"));
    }

    #[test]
    fn a_wide_literal_is_refused_because_the_pool_mint_is_outstanding() {
        // The M3 gate, visible in the demo: a string literal cannot enter a
        // value byte and the pool concepts are unminted, so the cast refuses.
        let text = r#"{"blocks":{"languageVersion":0,"blocks":[
            {"type":"text_print","id":"p","inputs":{
              "TEXT":{"block":{"type":"text","id":"t","fields":{"TEXT":"hello"}}}}}]}}"#;
        let out = cast_workspace(text, "pairs");
        assert_eq!(out.scripts.len(), 0);
        assert_eq!(out.errors.len(), 1, "the refusal must be visible");
    }
}
