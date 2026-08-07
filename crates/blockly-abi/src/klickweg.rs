//! Klickwege addressing — clicking a block is an **address**, never a handler.
//!
//! # The trap this exists to avoid (a2ui charter T2)
//!
//! *"Behavior never rides the surface."* An `onClick: <lambda>` in a component
//! tree is the same hijack as `DEFINE EVENT … WHEN … THEN …` in DDL: it puts
//! behaviour on the transport. The a2ui charter's answer is that a click
//! travels as `ActionInvoke { key, action_ordinal, args }` — an **ordinal into
//! the class's action set** — and the behaviour is a property of the Core node
//! the address resolves to.
//!
//! This module is the block-editor half of that: given a workspace and the id
//! of the block a user clicked, produce the address. It never produces a
//! callback, a closure, or a script fragment, and it cannot — nothing here
//! returns anything invocable.
//!
//! # Why the ordinal is the CALL INDEX and not a block counter
//!
//! The ordinal has to name something that survives the trip. A block id is
//! editor state; a position is presentation; a "nth block in the XML" ordering
//! is neither, because Blockly's own tree order is not the program's order.
//!
//! The call index is the one number that is already the program: it is where
//! [`lower_script`](crate::lower_script) put that block's
//! [`Call`](ogar_loco::Call) in the body, so
//! `raise_calls(body)[ordinal]` IS the clicked block's function, by
//! construction rather than by convention. That makes the address checkable
//! against the ABI instead of merely consistent with itself — and it is
//! exactly what the falsifier
//! (`an_ordinal_indexes_the_clicked_blocks_own_call`) asserts.
//!
//! Consequence worth stating: the traversal here MUST match the cast's
//! post-order. It is not "the same idea implemented twice" — it walks the same
//! shape for the same reason, and the test compares the two rather than
//! trusting them to agree.
//!
//! # What a consumer does with this
//!
//! `a2ui-server`'s `KlickwegEdge` wants `{from_key, class_id, ordinal,
//! predicate, seq}`. Three of those come from here ([`BlockAddress`]);
//! `from_key` is the function node's own GUID and `seq` is the session's
//! monotonic counter — both owned by the session, not by the cast. Nothing in
//! this crate depends on `a2ui-server`: the block editor produces addresses,
//! the desktop consumes them, and neither imports the other.

use crate::palette;
use ogar_loco::LaneShape;

use crate::{BlockRecord, codebook};

/// Where a click lands, in the ABI's own coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockAddress {
    /// The V3 render classid — canon-high, `concept << 16 | app_prefix`.
    pub class_id: u32,
    /// The clicked block's **call index** in the lowered body. This is the
    /// address; `raise_calls(body)[ordinal]` is its call.
    pub ordinal: u32,
    /// The edge label — the Blockly block type, with its dropdown code when
    /// the code participates in resolution.
    ///
    /// A label, not a lookup key: the ordinal is what resolves, and the
    /// predicate is what a human reads in a trace. Deriving it from the block
    /// type keeps it honest without minting a second vocabulary (charter T1).
    pub predicate: String,
}

/// The Klickwege edge label for a block: its type, plus the code when the code
/// selected the function.
///
/// A `ValueParam` code is deliberately NOT appended — it is an argument, and
/// putting an argument in the edge label would make two clicks on the same
/// action read as two different actions.
#[must_use]
pub fn predicate_for(block: &BlockRecord) -> String {
    let code = block.dropdown_code();
    match (code, codebook::resolve(&block.ty, code)) {
        (Some(c), Some(m)) if m.role == codebook::CodeRole::Selector => {
            format!("{}[{c}]", block.ty)
        }
        _ => block.ty.clone(),
    }
}

/// Every block's address, in call order.
///
/// The returned vector is indexed BY ordinal: element `i` is the block whose
/// call sits at index `i`. Walking post-order — operands before their
/// operator, then `next` — because that is what the cast emits.
#[must_use]
pub fn addresses(top: &BlockRecord, app_prefix: u16) -> Vec<(String, BlockAddress)> {
    // The PALETTE concept (0x1717) — which vocabulary reads the call bytes.
    // Not the node's shape: that is ogar-loco's `LocoConcept::FunctionBody`
    // (0x1701), and a block editor does not own it (OGAR #255).
    let class_id = palette::render_classid(app_prefix);
    let mut out = Vec::new();
    walk_chain(top, class_id, &mut out);
    out
}

fn walk_chain(block: &BlockRecord, class_id: u32, out: &mut Vec<(String, BlockAddress)>) {
    walk_block(block, class_id, out);
    if let Some(next) = &block.next {
        walk_chain(next, class_id, out);
    }
}

fn walk_block(block: &BlockRecord, class_id: u32, out: &mut Vec<(String, BlockAddress)>) {
    for (_, operand) in &block.inputs {
        walk_block(operand, class_id, out);
    }
    // `out.len()` before the push IS the call index, because the cast pushes
    // its call at exactly this point in the same walk.
    let ordinal = u32::try_from(out.len()).unwrap_or(u32::MAX);
    out.push((
        block.id.clone(),
        BlockAddress {
            class_id,
            ordinal,
            predicate: predicate_for(block),
        },
    ));
}

/// The address of one clicked block, by id.
///
/// Returns `None` for an id the script does not contain — a click on a block
/// that is not in this function is refused rather than resolved to ordinal 0,
/// which would fire the wrong action.
#[must_use]
pub fn address_of(top: &BlockRecord, block_id: &str, app_prefix: u16) -> Option<BlockAddress> {
    addresses(top, app_prefix)
        .into_iter()
        .find(|(id, _)| id == block_id)
        .map(|(_, a)| a)
}

/// Whether a shape can address every block in the script — i.e. the program
/// fits, so every ordinal names a real call.
///
/// An address past the body's capacity is not an address; it is a number that
/// resolves to nothing. Callers check this before handing addresses out.
#[must_use]
pub fn addressable(top: &BlockRecord, shape: LaneShape) -> bool {
    top.block_count() <= shape.calls_per_function()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldValue, LaneShape, lower_script, raise_calls};

    /// A stand-in render prefix — deliberately NOT a real allocation.
    ///
    /// `blockly-rs` has **no minted app prefix yet** (an operator decision,
    /// tracked in OGAR `docs/BLOCK-EDITOR-PLAN.md`), so nothing in this crate
    /// may name one: `app_prefix` is a PARAMETER on every public function
    /// here, and this constant exists only so the tests have a value to pass.
    ///
    /// It was `0x1000` until a codex review on OGAR #238 pointed at the risk
    /// of a plan that reads "done" teaching someone to hardcode a colliding
    /// value — and `0x1000` turned out to be exactly that: `ogar-vocab`'s
    /// `ports.rs` RESERVES it for the V3-adoption monitor marker, with a test
    /// asserting it "must never be allocatable as a port's `APP_PREFIX`". The
    /// warning was about a hypothetical; the hypothetical was already here.
    ///
    /// `0xFF00` is chosen to be obviously unreal so it cannot be mistaken for
    /// the answer when the mint lands.
    const APP: u16 = 0xFF00;

    fn five_plus_three() -> BlockRecord {
        BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input(
                "A",
                BlockRecord::leaf("math_number", "a").with_field("NUM", FieldValue::Byte(5)),
            )
            .with_input(
                "B",
                BlockRecord::leaf("math_number", "b").with_field("NUM", FieldValue::Byte(3)),
            )
    }

    #[test]
    fn an_ordinal_indexes_the_clicked_blocks_own_call() {
        // THE falsifier. The address is only an address if it lands on the
        // right call — so it is checked against the ABI, not against itself.
        // A pre-order walk here (parent before operands) would still produce
        // unique, plausible-looking ordinals and would fail this.
        let script = five_plus_three();
        let body = lower_script(LaneShape::Pairs, &script).unwrap();
        let calls = raise_calls(&body);

        for (id, addr) in addresses(&script, APP) {
            let call = &calls[addr.ordinal as usize];
            let expected = match id.as_str() {
                "a" | "b" => ogar_loco::FnIndex::NUMBER,
                "root" => ogar_loco::FnIndex::ADD,
                other => panic!("unexpected block {other}"),
            };
            assert_eq!(
                call.function, expected,
                "block {id} addressed the wrong call"
            );
        }

        // Anti-vacuity: the operands must NOT be at the same ordinal as their
        // operator, or "every address is right" would be trivially satisfiable
        // by a table of zeroes.
        let a = address_of(&script, "a", APP).unwrap().ordinal;
        let b = address_of(&script, "b", APP).unwrap().ordinal;
        let root = address_of(&script, "root", APP).unwrap().ordinal;
        assert_eq!((a, b, root), (0, 1, 2));
        // …and it really is post-order: the operator comes LAST.
        assert!(root > a && root > b);
    }

    #[test]
    fn the_ordinals_are_dense_and_agree_with_the_call_count() {
        let script = five_plus_three();
        let addrs = addresses(&script, APP);
        assert_eq!(addrs.len(), script.block_count());
        for (i, (_, a)) in addrs.iter().enumerate() {
            assert_eq!(a.ordinal as usize, i, "ordinals must be dense");
        }
        let body = lower_script(LaneShape::Pairs, &script).unwrap();
        assert_eq!(addrs.len(), body.len());
    }

    #[test]
    fn a_statement_chain_addresses_after_its_operands() {
        // `next` is the statement sequence and is walked AFTER the block's own
        // call — so a two-statement script must not interleave.
        let script = BlockRecord::leaf("text_print", "p1")
            .with_input(
                "TEXT",
                BlockRecord::leaf("math_number", "n1").with_field("NUM", FieldValue::Byte(1)),
            )
            .with_next(BlockRecord::leaf("text_print", "p2").with_input(
                "TEXT",
                BlockRecord::leaf("math_number", "n2").with_field("NUM", FieldValue::Byte(2)),
            ));
        let ord = |id: &str| address_of(&script, id, APP).unwrap().ordinal;
        assert_eq!((ord("n1"), ord("p1"), ord("n2"), ord("p2")), (0, 1, 2, 3));

        // Cross-checked against the ABI, so this is not just self-consistent.
        let calls = raise_calls(&lower_script(LaneShape::Pairs, &script).unwrap());
        assert_eq!(calls[ord("n1") as usize].values[0], 1);
        assert_eq!(calls[ord("n2") as usize].values[0], 2);
    }

    #[test]
    fn a_click_on_a_block_outside_the_script_is_refused() {
        // Silence twin: an unknown id must yield None, NOT ordinal 0 — which
        // would fire the first action in the function.
        let script = five_plus_three();
        assert_eq!(address_of(&script, "not_here", APP), None);
        assert!(address_of(&script, "root", APP).is_some());
    }

    #[test]
    fn the_classid_is_canon_high_under_the_app_prefix() {
        let addr = address_of(&five_plus_three(), "root", APP).unwrap();
        // 0x1717 = the PALETTE concept (which vocabulary reads the call
        // bytes), NOT 0x1701 = ogar-loco's function-body SHAPE. This test
        // pinned 0x1701 before OGAR #255 put 0x17 back with the substrate;
        // a block editor addressing by the shape concept was the ownership
        // inversion that ruling corrected.
        assert_eq!(addr.class_id, 0x1717_FF00);
        assert_eq!(addr.class_id >> 16, 0x1717, "concept must be the HIGH half");
        assert_ne!(
            addr.class_id >> 16,
            u32::from(ogar_loco::LocoConcept::FunctionBody.concept_id()),
            "a palette address must never claim the substrate's shape concept"
        );
        assert_eq!(addr.class_id & 0xFFFF, u32::from(APP));
        // …and it is NOT the reserved V3-adoption marker.
        assert_ne!(addr.class_id & 0xFFFF, 0x1000);
        // A different app renders the same concept differently — proving the
        // prefix is live and not baked.
        let other = address_of(&five_plus_three(), "root", 0x2000).unwrap();
        assert_ne!(other.class_id, addr.class_id);
        assert_eq!(other.class_id >> 16, addr.class_id >> 16);
    }

    #[test]
    fn a_selector_code_labels_the_edge_but_an_argument_code_does_not() {
        // Two-sided on the one judgment this module makes. A Selector code IS
        // part of which action fired; a ValueParam code is an argument, and
        // putting it in the label would make two clicks on one action read as
        // two different actions.
        let add = BlockRecord::leaf("math_arithmetic", "x")
            .with_field("OP", FieldValue::Code("ADD".into()));
        assert_eq!(predicate_for(&add), "math_arithmetic[ADD]");

        let pi = BlockRecord::leaf("math_constant", "y")
            .with_field("CONSTANT", FieldValue::Code("PI".into()));
        let e = BlockRecord::leaf("math_constant", "y")
            .with_field("CONSTANT", FieldValue::Code("E".into()));
        assert_eq!(predicate_for(&pi), "math_constant");
        assert_eq!(
            predicate_for(&pi),
            predicate_for(&e),
            "an argument must not change the edge label"
        );
        // …and the Selector case really does discriminate, or the first
        // assertion above would pass for a function that ignores codes.
        let sub = BlockRecord::leaf("math_arithmetic", "x")
            .with_field("OP", FieldValue::Code("MINUS".into()));
        assert_ne!(predicate_for(&add), predicate_for(&sub));
    }

    #[test]
    fn addressability_tracks_the_shapes_real_budget() {
        let script = five_plus_three();
        assert!(addressable(&script, LaneShape::Pairs));
        // `with_next` REPLACES the successor, so a chain of length n is built
        // from the tail inward — appending in a forward loop would silently
        // yield a 2-block script and make this test vacuous.
        let chain = |n: usize| {
            let mut b = BlockRecord::leaf("text_print", format!("p{}", n - 1));
            for i in (0..n - 1).rev() {
                b = BlockRecord::leaf("text_print", format!("p{i}")).with_next(b);
            }
            assert_eq!(b.block_count(), n, "the fixture must really be n blocks");
            b
        };

        // A program larger than the shape's budget is NOT addressable — every
        // ordinal past capacity resolves to nothing.
        let quads = LaneShape::Quads.calls_per_function();
        assert!(!addressable(&chain(quads + 1), LaneShape::Quads));
        // Exactly at capacity still is — so the bound is `<=`, not `<`.
        assert!(addressable(&chain(quads), LaneShape::Quads));
        // Two-sided across shapes: the wider budget admits what the narrower
        // one refuses.
        assert!(addressable(&chain(quads + 1), LaneShape::Pairs));
    }
}
