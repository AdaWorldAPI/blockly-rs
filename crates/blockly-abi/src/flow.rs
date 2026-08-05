//! Control flow — where a call **references** another function instead of
//! consuming an operand.
//!
//! # The distinction the expression core never needed
//!
//! Until now every call did one thing with the stack: pop `arity` operands,
//! push a result. `ADD` pops two. `NUMBER` pops none. That was enough because
//! nothing above the expression core existed.
//!
//! Control flow breaks the symmetry. `repeat 10 [ … ]` consumes **one** stack
//! operand (the count) and **references** a body. The body is not an operand:
//! it is not on the stack, it was not evaluated before the call, and popping it
//! would silently reattribute whatever *was* on the stack.
//!
//! So a call has two independent quantities, and conflating them is the bug
//! this module exists to prevent:
//!
//! | | what it is | where it lives |
//! |---|---|---|
//! | [`stack_arity`] | operands evaluated before the call | the stack |
//! | [`body_refs`] | function indices this call branches to | the call's **value bytes** |
//!
//! # Why a body is a reference and not an inline run
//!
//! Locked in W0 (`ogar-blockly` #236): *nesting by reference — a function index
//! names another function's node. No `END`, no jump offset.* That is SB3's own
//! model, and it is what keeps the node fixed-size: a loop body of any length
//! costs its parent exactly one byte.
//!
//! The alternative — splicing the body inline with a terminator — would make a
//! call's width depend on its contents, which is the same defect that killed
//! literal-as-call-run for the constant pool: editing inside a loop would shift
//! every later call in the enclosing function.
//!
//! # `FOREVER` has no Blockly block, and that is not an oversight
//!
//! It is Scratch's `control_forever`. The table covers it because the palette
//! does and `scratch-rs` reaches it from `.sb3`; the Blockly codebook simply has
//! no type that resolves to it.

use ogar_blockly::FnIndex;

/// How many operands a call pops from the stack.
///
/// `None` means the projection does not cover this function — refused rather
/// than guessed, because a wrong arity does not produce a slightly-wrong
/// result: it desynchronizes the stack and reattributes every later operand.
///
/// For control flow this counts **only** the evaluated operands. A loop body is
/// not among them; see [`body_refs`].
#[must_use]
pub fn stack_arity(f: FnIndex) -> Option<u8> {
    Some(match f {
        // Bodies only — nothing evaluated first.
        FnIndex::FOREVER => 0,
        // One condition or count, then a body.
        FnIndex::IF
        | FnIndex::IF_ELSE
        | FnIndex::REPEAT
        | FnIndex::WHILE
        | FnIndex::REPEAT_UNTIL
        | FnIndex::FOR_EACH => 1,
        // from, to, by — then a body.
        FnIndex::FOR_RANGE => 3,
        // Leave the enclosing loop / iteration. No operand, no body.
        FnIndex::BREAK | FnIndex::CONTINUE => 0,
        _ => return None,
    })
}

/// How many of a call's value bytes are **function indices** it branches to.
///
/// Zero for every expression call, so the existing surface is unaffected.
/// `IF_ELSE` is the only two, which is why it needs a shape wider than
/// [`Pairs`](ogar_blockly::LaneShape::Pairs) — see [`min_shape`].
#[must_use]
pub fn body_refs(f: FnIndex) -> u8 {
    match f {
        FnIndex::IF
        | FnIndex::REPEAT
        | FnIndex::WHILE
        | FnIndex::REPEAT_UNTIL
        | FnIndex::FOREVER
        | FnIndex::FOR_EACH
        | FnIndex::FOR_RANGE => 1,
        FnIndex::IF_ELSE => 2,
        _ => 0,
    }
}

/// Whether this function is control flow at all — i.e. it references a body.
///
/// `BREAK` and `CONTINUE` are control flow in the language sense but reference
/// nothing, so they are deliberately **not** included: this predicate answers
/// "does lowering this call require emitting another function?", which is the
/// only question the cast asks.
#[must_use]
pub fn branches(f: FnIndex) -> bool {
    body_refs(f) > 0
}

/// The narrowest [`LaneShape`](ogar_blockly::LaneShape) that can hold this
/// call's value bytes.
///
/// `IF_ELSE` carries two body references, so it cannot be stored under `Pairs`
/// — a one-byte immediate would truncate the else arm into nothing, and the
/// program would run its then-branch and silently skip the else. The cast
/// refuses rather than narrowing; this is what it consults.
#[must_use]
pub fn min_shape(f: FnIndex) -> ogar_blockly::LaneShape {
    use ogar_blockly::LaneShape;
    match body_refs(f) {
        0 | 1 => LaneShape::Pairs,
        _ => LaneShape::Triples,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_blockly::LaneShape;

    #[test]
    fn a_body_reference_is_not_a_stack_operand() {
        // THE distinction. `repeat` pops the COUNT (one operand) and
        // references a body. If the body were counted as an operand, nesting
        // would pop something that was never pushed and every earlier operand
        // would shift by one.
        assert_eq!(stack_arity(FnIndex::REPEAT), Some(1));
        assert_eq!(body_refs(FnIndex::REPEAT), 1);
        // `forever` proves the two are genuinely independent: zero operands,
        // one body. A single conflated number cannot express it.
        assert_eq!(stack_arity(FnIndex::FOREVER), Some(0));
        assert_eq!(body_refs(FnIndex::FOREVER), 1);
        // …and the mirror: an expression pops operands and references nothing.
        assert_eq!(body_refs(FnIndex::ADD), 0);
        assert_eq!(body_refs(FnIndex::SQRT), 0);
    }

    #[test]
    fn if_else_carries_two_bodies_and_therefore_needs_a_wider_shape() {
        assert_eq!(body_refs(FnIndex::IF_ELSE), 2);
        assert_eq!(stack_arity(FnIndex::IF_ELSE), Some(1));
        // Under Pairs the else arm would be truncated away and the program
        // would run the then-branch and silently skip the else — the exact
        // "looks complete and is not" failure the ABI refuses elsewhere.
        assert_eq!(min_shape(FnIndex::IF_ELSE), LaneShape::Triples);
        // Two-sided: one-body forms fit Pairs, so the requirement is specific
        // to IF_ELSE rather than a blanket widening of all control flow.
        assert_eq!(min_shape(FnIndex::IF), LaneShape::Pairs);
        assert_eq!(min_shape(FnIndex::REPEAT), LaneShape::Pairs);
        assert_eq!(min_shape(FnIndex::ADD), LaneShape::Pairs);
    }

    #[test]
    fn branches_answers_does_lowering_need_another_function() {
        // BREAK and CONTINUE are control flow in the language sense but
        // reference no body, so the cast emits no extra function for them.
        // Including them here would make `lower_program` look for a statement
        // input that a break block does not have.
        assert!(branches(FnIndex::REPEAT));
        assert!(branches(FnIndex::IF_ELSE));
        assert!(!branches(FnIndex::BREAK));
        assert!(!branches(FnIndex::CONTINUE));
        assert!(!branches(FnIndex::ADD));
        // …but they ARE covered, so a program containing one still lowers.
        assert_eq!(stack_arity(FnIndex::BREAK), Some(0));
        assert_eq!(stack_arity(FnIndex::CONTINUE), Some(0));
    }

    #[test]
    fn the_two_tables_agree_on_what_is_covered() {
        // A function with a stack arity but no body-ref entry (or vice versa)
        // would lower half-correctly. Every control-flow opcode the palette
        // names must appear consistently in both.
        for f in [
            FnIndex::IF,
            FnIndex::IF_ELSE,
            FnIndex::REPEAT,
            FnIndex::WHILE,
            FnIndex::REPEAT_UNTIL,
            FnIndex::FOREVER,
            FnIndex::FOR_EACH,
            FnIndex::FOR_RANGE,
        ] {
            assert!(stack_arity(f).is_some(), "{f:?} has no stack arity");
            assert!(branches(f), "{f:?} should reference a body");
        }
        // Silence twin: uncovered control flow stays uncovered rather than
        // being quietly assigned a plausible shape. WAIT/STOP/RETURN are real
        // palette entries this wave does not model.
        for f in [
            FnIndex::WAIT,
            FnIndex::WAIT_UNTIL,
            FnIndex::STOP,
            FnIndex::RETURN,
        ] {
            assert_eq!(stack_arity(f), None, "{f:?} must stay refused");
        }
    }
}
