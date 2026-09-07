//! The canonical read surface: `NodeDelta` bytes down, askama fieldview render.
//!
//! # Why this replaces the JSON panels
//!
//! The demo's original read path POSTed a workspace and got `CastOut` **JSON**
//! back — a *description* of the program, re-encoded on every keystroke. For an
//! arc whose thesis is "the blocks are a projection; the 512-byte node is the
//! program", shipping JSON is the opposite claim: it makes the description the
//! artefact and the bytes an implementation detail.
//!
//! a2ui already settled this. The canonical surface is
//! [`NodeDelta`](ogar_a2ui_frame::NodeDelta) down and `ActionInvoke` up
//! (charter T2/T3, OGAR `docs/A2UI-SCREEN-ADDRESSING-PROPOSAL.md`), with the
//! render happening from a ClassView projection through askama. Both bricks
//! live in the same OGAR checkout this repo already clones, and
//! `ogar-a2ui-frame` is byte-native: its serde derive is feature-gated and
//! left OFF, so `Frame::to_le_bytes` IS the wire format.
//!
//! So the program now leaves the server as the stored bytes it already is:
//!
//! ```text
//!   before   workspace ──▶ cast ──▶ CastOut ──▶ serde_json ──▶ browser
//!   after    workspace ──▶ cast ──▶ node bytes ──▶ NodeDelta::to_le_bytes ──▶ browser
//! ```
//!
//! # What legitimately stays JSON
//!
//! Blockly is a JavaScript editor: the workspace save arriving from the
//! browser, the toolbox, and the generated block definitions are all JSON
//! because the *editor* speaks JSON. That is the membrane the charter permits
//! — one boundary, named, at the edge. What is gone is JSON on the path that
//! carries the PROGRAM.

use blockly_abi::{FunctionNode, Program};
use ogar_a2ui_frame::{Frame, NodeDelta};
use ogar_render_askama::field_view::{ActionRef, FieldView, render_field_view};

/// Build the `NodeDelta` for one cast function node.
///
/// `mask_words` marks every field the node carries as changed, because a fresh
/// cast IS the whole node — a delta against nothing. The values are the stored
/// bytes verbatim: this function never re-encodes, it forwards what
/// [`FunctionNode::to_le_bytes`] already produced.
#[must_use]
pub fn node_delta(key: [u8; 16], node_bytes: &[u8]) -> NodeDelta {
    // One bit per 16-byte slot of the 512-byte node: 32 slots, one u64 word.
    let slots = node_bytes.len().div_ceil(16);
    let mask = if slots >= 64 {
        u64::MAX
    } else {
        (1u64 << slots) - 1
    };
    NodeDelta {
        key,
        mask_words: vec![mask],
        values: node_bytes.to_vec(),
    }
}

/// The whole cast program as a byte frame — the canonical wire form.
///
/// Returns `Frame::NodeDelta` for the entry node, already `to_le_bytes`'d.
/// A caller writes these bytes to the socket; nothing serializes.
#[must_use]
pub fn program_frame_bytes(key: [u8; 16], prog: &Program) -> Vec<u8> {
    let node = FunctionNode::new(key, *prog.entry());
    Frame::NodeDelta(node_delta(key, &node.to_le_bytes())).to_le_bytes()
}

/// Render the program's surface as HTML through the upstream askama brick.
///
/// This is the ClassView projection: each function becomes one addressed
/// field whose `position` is its index (its layout address), whose predicate
/// is the stable key behind the label, and whose value is the call rail. The
/// page displays what the projection produced — it does not receive a JSON
/// document and lay it out itself.
///
/// # Errors
///
/// Propagates askama's render error.
pub fn render_surface(key: [u8; 16], prog: &Program, shape: &str) -> Result<String, askama::Error> {
    let fields: Vec<FieldView> = prog
        .functions
        .iter()
        .enumerate()
        .map(|(i, body)| FieldView {
            position: u8::try_from(i).unwrap_or(u8::MAX),
            label: format!("fn {i}"),
            predicate: format!("loco:function/{i}"),
            value: blockly_abi::raise_calls(body)
                .iter()
                .map(|c| {
                    let name = blockly_abi::scratch::SCRATCH_DEVICE
                        .iter()
                        .find(|&&(_, b, ..)| b == c.function.0)
                        .map(|&(n, ..)| n.to_string())
                        .or_else(|| {
                            ogar_loco::vocabulary::shared_core::name(c.function).map(str::to_string)
                        })
                        .unwrap_or_else(|| format!("{:#04x}", c.function.0));
                    format!("{name}:{}", c.values.first().copied().unwrap_or(0))
                })
                .collect::<Vec<_>>()
                .join("  "),
        })
        .collect();

    // Actions travel by ADDRESS (charter T2) — an ordinal into the class's
    // ActionDef set, never an inline handler on the surface.
    let actions = [ActionRef {
        ordinal: 0,
        label: "recast".to_string(),
    }];

    let key_hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
    render_field_view(
        blockly_abi::palette::PALETTE_CONCEPT,
        // The full canon-high classid, so the surface names the address it
        // renders for rather than only the concept half.
        &format!("blockly/program {:#010x} ({shape})", render_classid()),
        &key_hex,
        "cast program",
        &fields,
        &actions,
    )
}

/// The classid this surface renders for — the SAME address the stored key
/// carries, never a second spelling of it.
///
/// It is the substrate's registered `CLASSID_BLOCKS_V3`: canon `0x1717` in
/// the high half, the V3 generation marker `0x1000` in the custom half. It
/// used to compose over a local `0xFF00` app-prefix placeholder, which was
/// harmless only for as long as nothing compared the rendered address to
/// the minted one. A real app prefix is still the unminted operator
/// decision M1 — `0x1000` is not one, and cannot become one: `ogar-vocab`
/// reserves it for the V3-adoption monitor with a test asserting it "must
/// never be allocatable as a port's `APP_PREFIX`".
#[must_use]
pub const fn render_classid() -> u32 {
    blockly_store::CLASSID
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_a2ui_frame::Frame;

    const ARITH: &str = r#"{"blocks":{"languageVersion":0,"blocks":[
        {"type":"math_arithmetic","id":"root","fields":{"OP":"ADD"},
         "inputs":{
           "A":{"block":{"type":"math_number","id":"a","fields":{"NUM":5}}},
           "B":{"block":{"type":"math_number","id":"b","fields":{"NUM":3}}}}}]}}"#;

    /// The frame carries the STORED BYTES, not a re-encoding of them.
    ///
    /// This is the whole claim. A frame that merely round-trips through
    /// `to_le_bytes`/`from_le_bytes` proves the codec works; what must be true
    /// here is stronger — the bytes the client receives are byte-identical to
    /// what `FunctionNode::to_le_bytes` produced, so the wire form IS the
    /// storage form rather than a second encoding that happens to agree today.
    #[test]
    fn the_frame_forwards_the_stored_node_bytes_unchanged() {
        let prog = crate::cast::first_program(ARITH, "pairs").expect("casts");
        let key = crate::cast::demo_key();
        let stored = FunctionNode::new(key, *prog.entry()).to_le_bytes();

        let wire = program_frame_bytes(key, &prog);
        let Frame::NodeDelta(delta) = Frame::from_le_bytes(&wire).expect("decodes") else {
            panic!("expected a NodeDelta");
        };

        assert_eq!(delta.key, key);
        assert_eq!(
            delta.values,
            stored.to_vec(),
            "the frame must forward the stored node, not re-encode it"
        );
        assert_eq!(delta.values.len(), ogar_loco::node::NODE_BYTES);
        // Every 16-byte slot of the node is marked changed — a fresh cast is
        // the whole node, a delta against nothing.
        assert_eq!(delta.mask_words, vec![(1u64 << 32) - 1]);
    }

    /// No JSON on the program path — asserted on the actual bytes.
    ///
    /// A frame that had quietly become JSON would still decode and still pass
    /// a round-trip test. This checks the wire bytes are not text: the stored
    /// node's classid prefix must appear as raw little-endian bytes, and the
    /// payload must not start with a JSON delimiter.
    #[test]
    fn the_wire_form_is_bytes_and_not_a_json_document() {
        let prog = crate::cast::first_program(ARITH, "pairs").expect("casts");
        let wire = program_frame_bytes(crate::cast::demo_key(), &prog);

        assert!(!wire.is_empty());
        assert_ne!(wire[0], b'{', "the program path must not carry JSON");
        assert_ne!(wire[0], b'[', "the program path must not carry JSON");
        // The key's classid rides as raw LE bytes. Derived from the address
        // itself rather than spelled as a literal: an earlier cut hardcoded
        // the retired `0xFF00` placeholder here and kept asserting it after
        // `render_classid` was unified onto the minted address, so the test
        // pinned a prefix the key had stopped carrying.
        let le = blockly_store::CLASSID.to_le_bytes();
        assert_eq!(
            le,
            [0x00, 0x10, 0x17, 0x17],
            "canon-high 0x1717 over 0x1000"
        );
        assert!(
            wire.windows(4).any(|w| w == le),
            "the canon-high classid must appear as raw LE bytes on the wire"
        );
        // Silence twin: the retired placeholder must NOT be on the wire. A
        // window check that matched anything four bytes long would pass the
        // half above; this proves it discriminates.
        assert!(
            !wire.windows(4).any(|w| w == [0x00, 0xFF, 0x17, 0x17]),
            "the retired 0xFF00 render placeholder must not ride the wire"
        );
    }

    /// The surface renders through the upstream askama brick, addressed.
    #[test]
    fn the_rendered_surface_is_a_classview_projection_with_addresses() {
        let prog = crate::cast::first_program(ARITH, "pairs").expect("casts");
        let html = render_surface(crate::cast::demo_key(), &prog, "Pairs").expect("renders");

        // Everything below is an ADDRESS on the surface, which is the claim:
        // "don't push pixels, address the screen".
        //
        // The palette concept, canon-high — the class the client resolves its
        // ClassView/template codebook against.
        assert!(html.contains(r#"data-class-id="0x1717""#), "{html}");
        // The node key addresses the whole surface.
        assert!(html.contains("data-key="), "{html}");
        // Each field carries its mask POSITION — the layout address a
        // `WideFieldMask` indexes, so a client repaints exactly the positions
        // a NodeDelta marked, without interpreting any name.
        assert!(html.contains(r#"data-field-pos="0""#), "{html}");
        // Behaviour travels by ORDINAL (charter trap T2) — never an inline
        // handler on the surface.
        assert!(html.contains(r#"data-action-ordinal="0""#), "{html}");
        // The projected value made it through.
        assert!(html.contains("ADD"), "the rail must render: {html}");
        // The rendered address IS the minted one — the unification
        // `render_classid`'s own doc describes. Asserted two-sided: it must
        // equal the stored classid AND must no longer be the local `0xFF00`
        // placeholder it used to compose, or "never a second spelling of it"
        // would be prose with nothing behind it.
        assert_eq!(render_classid(), blockly_store::CLASSID);
        assert_eq!(render_classid(), 0x1717_1000);
        assert_ne!(render_classid(), 0x1717_FF00);

        // Silence twin: the surface must NOT carry behaviour. If a handler
        // attribute ever appears here, T2 has been violated and the address
        // discipline is decoration.
        assert!(
            !html.contains("onclick"),
            "the surface must not carry behaviour"
        );
        assert!(
            !html.contains("<script"),
            "the surface must not carry behaviour"
        );
    }
}
