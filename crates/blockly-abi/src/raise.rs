//! The inverse cast: stored bytes → Blockly blocks.
//!
//! # Why this exists
//!
//! Lowering was always one-way, so the only durable form a program could take
//! was a *Blockly save* — the projection. That is backwards for a substrate
//! whose thesis is that the 512-byte node IS the program: it made the editor's
//! format the source of truth and the bytes a derived artefact.
//!
//! With a raise, the bytes become storable and the editor's JSON becomes what
//! it should be — a rendering, produced on demand at the membrane. A reference
//! program can then ship as the nodes themselves.
//!
//! # The algorithm is the lowering, read backwards
//!
//! Lowering is a post-order walk: operands before the operator, statements in
//! sequence, nested bodies by reference. Raising replays that on a stack:
//!
//! - pop [`shared_core::stack_arity`] operands — they are the calls already emitted, so
//!   they are on the stack in the order the block declares its inputs;
//! - take the first [`shared_core::body_refs`] value bytes as FUNCTION INDICES (the same
//!   `values[..n]` carving `branches_of` reads) and raise each as a statement
//!   input;
//! - if the call pushes a result it is an expression → push it back; otherwise
//!   it is a statement → append it to the chain.
//!
//! Nothing here guesses: an operation the core does not cover, or a byte no
//! vocabulary names, is refused. A raise that invented a block would produce a
//! program the cast then rejects, which is worse than an error.

use crate::{BlockRecord, FieldValue};
use ogar_loco::vocabulary::shared_core;
use ogar_loco::{FnIndex, FunctionBody};

/// Why a stored program could not be raised back to blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaiseError {
    /// No vocabulary names this function, so no block type can be chosen.
    UnknownFunction(u8),
    /// The shared core does not cover this function's arity — refused rather
    /// than guessed, for the same reason lowering refuses it.
    Uncovered(u8),
    /// A call wanted more operands than the stack held: the body is malformed.
    StackUnderflow(u8),
    /// A body reference names a function the program does not contain.
    DanglingReference(u8),
    /// A menu-bearing function carries an index outside the menu's static
    /// prefix — a project-interned entry this raise has no table for.
    UnknownOption(u8, u8),
}

impl core::fmt::Display for RaiseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnknownFunction(b) => write!(f, "no vocabulary names function {b:#04x}"),
            Self::Uncovered(b) => write!(f, "function {b:#04x} has no covered arity"),
            Self::StackUnderflow(b) => write!(f, "function {b:#04x} wanted absent operands"),
            Self::DanglingReference(i) => write!(f, "body reference {i} names no function"),
            Self::UnknownOption(b, i) => {
                write!(
                    f,
                    "function {b:#04x} carries option {i}, outside its static menu"
                )
            }
        }
    }
}

impl core::error::Error for RaiseError {}

/// The block type a function raises to, and its dropdown code if it needs one.
///
/// One function can have several block spellings — Blockly's
/// `math_arithmetic[ADD]` and Scratch's `operator_add` are the same `ADD`. The
/// raise picks ONE, and prefers the Scratch spelling because that is the
/// vocabulary whose blocks the demo defines; the choice is presentation, and
/// the round-trip test is what proves it lowers back to the same bytes either
/// way.
#[must_use]
pub fn block_for(f: FnIndex) -> Option<(&'static str, Option<&'static str>)> {
    // Device mints first — the byte names exactly one block.
    if let Some((name, ..)) = crate::scratch::device_by_byte(f.0) {
        return Some((name, None));
    }
    // Then the Scratch spelling of a shared-core operation.
    if let Some((name, _)) = crate::scratch::SCRATCH_CORE.iter().find(|&&(_, x)| x == f) {
        return Some((name, None));
    }
    // Literals and the few shapes Scratch reaches through a Blockly block.
    match f {
        FnIndex::NUMBER => Some(("math_number", None)),
        _ => None,
    }
}

/// Whether a call yields a value the next call can consume.
///
/// Two sources, because the two halves of the palette own different answers:
/// the shared core answers for its own bytes, and a device byte's answer is
/// its declared [`Shape`](crate::scratch::Shape) — harvested from the same
/// source as the opcode, so it cannot drift from the block a user drags.
#[must_use]
pub fn pushes(f: FnIndex) -> bool {
    use crate::scratch::Shape;
    if let Some((name, ..)) = crate::scratch::device_by_byte(f.0) {
        return crate::scratch::SCRATCH_BLOCK_DEFS
            .iter()
            .find(|&&(t, ..)| t == name)
            .is_some_and(|&(_, _, shape, ..)| matches!(shape, Shape::Reporter | Shape::Boolean));
    }
    shared_core::pushes_result(f) == Some(true)
}

/// Raise one function body, and everything it references, into a block chain.
///
/// Returns the chain's head — the first statement, with the rest hanging off
/// `next`.
///
/// # Errors
///
/// See [`RaiseError`]. Every variant is a refusal, never a substitution.
pub fn raise_body(
    functions: &[FunctionBody],
    index: usize,
) -> Result<Option<BlockRecord>, RaiseError> {
    let body = functions
        .get(index)
        .ok_or(RaiseError::DanglingReference(index as u8))?;

    let mut stack: Vec<BlockRecord> = Vec::new();
    let mut chain: Vec<BlockRecord> = Vec::new();

    for (ci, call) in crate::raise_calls(body).iter().enumerate() {
        let f = call.function;
        let (ty, code) = block_for(f).ok_or(RaiseError::UnknownFunction(f.0))?;

        let refs = usize::from(shared_core::body_refs(f));
        let arity = usize::from(
            shared_core::stack_arity(f)
                .or_else(|| crate::scratch::device_by_byte(f.0).map(|(_, a, _)| a))
                .ok_or(RaiseError::Uncovered(f.0))?,
        );

        if stack.len() < arity {
            return Err(RaiseError::StackUnderflow(f.0));
        }
        let operands: Vec<BlockRecord> = stack.split_off(stack.len() - arity);

        // Socket names come from the harvested table, so a raised program
        // plugs its operands into the same sockets the definitions declare.
        let (value_names, stmt_names) = crate::scratch::SCRATCH_BLOCK_DEFS
            .iter()
            .find(|&&(t, ..)| t == ty)
            .map_or((&[][..], &[][..]), |&(_, _, _, v, s)| (v, s));

        // Value operands and statement bodies go into SEPARATE fields —
        // `BlockRecord` keeps the two-quantity split in the type itself, and
        // merging them here would reintroduce exactly the conflation the ABI's
        // two tables exist to prevent.
        let mut inputs: Vec<(String, BlockRecord)> = Vec::new();
        for (i, operand) in operands.into_iter().enumerate() {
            let name = value_names
                .get(i)
                .map_or_else(|| format!("V{i}"), |n| (*n).to_string());
            inputs.push((name, operand));
        }
        // Body references are the FIRST `refs` value bytes — the same carving
        // `branches_of` reads, not a second convention.
        let mut statements: Vec<(String, BlockRecord)> = Vec::new();
        for r in 0..refs {
            let target = usize::from(call.values.get(r).copied().unwrap_or(0));
            let name = stmt_names
                .get(r)
                .map_or_else(|| format!("S{r}"), |n| (*n).to_string());
            if let Some(sub) = raise_body(functions, target)? {
                statements.push((name, sub));
            }
        }

        let mut fields: Vec<(String, FieldValue)> = Vec::new();
        if let Some(c) = code {
            fields.push(("OP".to_string(), FieldValue::Code(c.to_string())));
        }
        if f == FnIndex::NUMBER {
            // The literal rides in the value byte AFTER any body refs.
            let n = call.values.get(refs).copied().unwrap_or(0);
            fields.push(("NUM".to_string(), FieldValue::Byte(n)));
        }
        // A dropdown rides in the same slot, as a codebook index. Decoded
        // through the static prefix; a byte past it is a project's own entry
        // (a sprite name the palette cannot know) and is refused, not guessed.
        if let Some((field, menu)) = crate::menus::menu_for_block(ty) {
            let b = call.values.get(refs).copied().unwrap_or(0);
            let code = crate::menus::decode(menu, b).ok_or(RaiseError::UnknownOption(f.0, b))?;
            fields.push((field.to_string(), FieldValue::Code(code.to_string())));
        }

        let record = BlockRecord {
            ty: ty.to_string(),
            id: format!("r{index}_{ci}"),
            fields,
            inputs,
            statements,
            next: None,
            extra_state: None,
            disabled: false,
        };

        // An expression pushes; a statement joins the chain.
        //
        // For shared-core bytes that answer is the substrate's own. For DEVICE
        // bytes the substrate deliberately says nothing — they are the
        // palette's, and the palette already records it as the block's SHAPE
        // (a reporter or boolean reporter yields a value; a statement or hat
        // does not). Reading `shared_core::pushes_result` alone silently
        // treated every device reporter as a statement, which left
        // `sensing_touchingobject`'s result off the stack and made the `if`
        // above it underflow — measured, not hypothetical.
        if pushes(f) {
            stack.push(record);
        } else {
            chain.push(record);
        }
    }

    // Anything still on the stack is a dangling operand — a value nothing
    // consumed. Keep it as a top-level statement rather than dropping it:
    // silently losing a block would make the raise lossy.
    chain.extend(stack);

    // Chain the statements through `next`, back to front.
    let mut head: Option<BlockRecord> = None;
    for mut s in chain.into_iter().rev() {
        s.next = head.map(Box::new);
        head = Some(s);
    }
    Ok(head)
}

/// Raise a whole stored program — its entry function and everything it
/// references — into one top-level block chain.
///
/// # Errors
///
/// See [`RaiseError`].
pub fn raise_program(functions: &[FunctionBody]) -> Result<Option<BlockRecord>, RaiseError> {
    raise_body(functions, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_loco::LaneShape;

    /// Raising and re-lowering reproduces the SAME BYTES.
    ///
    /// This is the only test that makes the raise trustworthy. Comparing block
    /// trees would compare my reconstruction against my own expectation; going
    /// back through `lower_program` compares against the SUBSTRATE — if the
    /// raise mis-orders operands, drops a body reference, or picks a block
    /// whose arity differs, the re-lowered bytes diverge and this fails.
    ///
    /// It is also what lets a template ship as bytes: the editor's JSON is
    /// then provably a rendering of the stored program rather than a second
    /// source of truth.
    fn round_trips(record: &crate::BlockRecord, shape: LaneShape) {
        let original = crate::lower_program(shape, record).expect("the fixture casts");
        let raised = raise_program(&original.functions)
            .expect("raises")
            .expect("a non-empty program raises to a block");
        let again = crate::lower_program(shape, &raised).expect("the raised program re-casts");

        assert_eq!(
            original.functions.len(),
            again.functions.len(),
            "function count changed across the round trip"
        );
        for (i, (a, b)) in original
            .functions
            .iter()
            .zip(again.functions.iter())
            .enumerate()
        {
            assert_eq!(
                a.as_body_bytes(),
                b.as_body_bytes(),
                "function {i} differs after raise + re-lower"
            );
        }
    }

    #[test]
    fn an_expression_round_trips_through_the_stored_bytes() {
        // 5 + 3 — operands then operator, the post-order the ABI specifies.
        let n = |id: &str, v: u8| crate::BlockRecord {
            ty: "math_number".into(),
            id: id.into(),
            fields: vec![("NUM".into(), crate::FieldValue::Byte(v))],
            inputs: vec![],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        let add = crate::BlockRecord {
            ty: "operator_add".into(),
            id: "add".into(),
            fields: vec![],
            inputs: vec![("NUM1".into(), n("a", 5)), ("NUM2".into(), n("b", 3))],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        round_trips(&add, LaneShape::Pairs);
    }

    #[test]
    fn nested_control_flow_round_trips_with_its_body_references() {
        // forever { move 4 } — a body reference, which is the shape a flat
        // call list cannot express and the one most likely to be raised wrong.
        let steps = crate::BlockRecord {
            ty: "math_number".into(),
            id: "s".into(),
            fields: vec![("NUM".into(), crate::FieldValue::Byte(4))],
            inputs: vec![],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        let mv = crate::BlockRecord {
            ty: "motion_movesteps".into(),
            id: "mv".into(),
            fields: vec![],
            inputs: vec![("STEPS".into(), steps)],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        let forever = crate::BlockRecord {
            ty: "control_forever".into(),
            id: "f".into(),
            fields: vec![],
            inputs: vec![],
            statements: vec![("SUBSTACK".into(), mv)],
            next: None,
            extra_state: None,
            disabled: false,
        };
        round_trips(&forever, LaneShape::Pairs);
    }

    /// A dropdown round-trips as a codebook index — inline field AND shadow
    /// block — and comes back as the harvested code, not a number.
    #[test]
    fn dropdowns_round_trip_as_codebook_indices() {
        let leaf = |ty: &str, id: &str, field: &str, code: &str| crate::BlockRecord {
            ty: ty.into(),
            id: id.into(),
            fields: vec![(field.into(), crate::FieldValue::Code(code.into()))],
            inputs: vec![],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        // when [up arrow] key pressed → if <key [space] pressed?> then go to front
        let pressed = crate::BlockRecord {
            ty: "sensing_keypressed".into(),
            id: "kp".into(),
            fields: vec![],
            inputs: vec![(
                "KEY_OPTION".into(),
                leaf("sensing_keyoptions", "ko", "KEY_OPTION", "space"),
            )],
            statements: vec![],
            next: None,
            extra_state: None,
            disabled: false,
        };
        let front = leaf("looks_gotofrontback", "fb", "FRONT_BACK", "back");
        let cond = crate::BlockRecord {
            ty: "control_if".into(),
            id: "if".into(),
            fields: vec![],
            inputs: vec![("CONDITION".into(), pressed)],
            statements: vec![("SUBSTACK".into(), front)],
            next: None,
            extra_state: None,
            disabled: false,
        };
        let mut hat = leaf("event_whenkeypressed", "hat", "KEY_OPTION", "up arrow");
        hat.next = Some(Box::new(cond));
        round_trips(&hat, LaneShape::Pairs);

        // And the raised tree carries the CODES, not bytes — the JSON a page
        // renders must name the option the user chose.
        let prog = crate::lower_program(LaneShape::Pairs, &hat).unwrap();
        let raised = raise_program(&prog.functions).unwrap().unwrap();
        assert_eq!(raised.ty, "event_whenkeypressed");
        assert_eq!(
            raised.field("KEY_OPTION"),
            Some(&crate::FieldValue::Code("up arrow".into()))
        );
        let back = raised.next.as_ref().unwrap().statements[0]
            .1
            .field("FRONT_BACK");
        assert_eq!(back, Some(&crate::FieldValue::Code("back".into())));
        // The stored byte really is the codebook index, not a literal.
        assert_eq!(
            crate::raise_calls(&prog.functions[0])[0].values[0],
            2,
            "up arrow = index 2"
        );

        // An index past the static prefix is refused, not invented.
        let body = ogar_loco::FunctionBody::from_calls(
            LaneShape::Pairs,
            &[ogar_loco::Call::with_values(
                FnIndex(crate::scratch::device("event_whenkeypressed").unwrap().0),
                [200, 0, 0],
            )],
        )
        .unwrap();
        assert!(matches!(
            raise_body(&[body], 0),
            Err(RaiseError::UnknownOption(_, 200))
        ));
    }

    /// The raise refuses rather than inventing a block.
    #[test]
    fn an_unnamed_function_is_refused_not_guessed() {
        // 0xFE is inside the palette range but nothing minted it.
        assert_eq!(block_for(FnIndex(0xFE)), None);
        let body = ogar_loco::FunctionBody::from_calls(
            LaneShape::Pairs,
            &[ogar_loco::Call::new(FnIndex(0xFE))],
        )
        .unwrap();
        assert_eq!(
            raise_body(&[body], 0),
            Err(RaiseError::UnknownFunction(0xFE))
        );
        // Can-fire half: a minted byte DOES resolve, so the refusal above
        // discriminates instead of rejecting everything.
        assert_eq!(
            block_for(FnIndex(0x90)).map(|(t, _)| t),
            Some("motion_movesteps")
        );
    }
}
