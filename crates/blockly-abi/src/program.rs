//! A **program** — the several functions a script with control flow becomes.
//!
//! # Why one script is not one function
//!
//! [`lower_script`](crate::lower_script) casts a block chain into a single
//! [`FunctionBody`]. That is exact for straight-line expressions and wrong the
//! moment a loop appears, because W0 locked nesting **by reference**: a loop
//! body is another function's node, named by index. No `END` marker, no jump
//! offset.
//!
//! So `repeat 10 [ move ]` is **two** functions — the caller and the body —
//! and the caller spends one value byte naming the second.
//!
//! That is what keeps a node fixed-size: a body of any length costs its parent
//! exactly one byte. Splicing the body inline with a terminator would make a
//! call's width depend on its contents, so editing inside a loop would shift
//! every later call in the enclosing function. It is the same defect that ruled
//! out literal-as-call-run for the constant pool, and it would break the W1
//! one-write gate for exactly the same reason.
//!
//! # Function 0 is the entry, and indices are stable
//!
//! [`Program::functions`] is indexed by the byte a caller stores. Function `0`
//! is the script's own body. Bodies are appended in the order the walk reaches
//! them, so an index, once assigned, does not move — which is what makes it
//! safe to store.
//!
//! Index `0` is therefore a **real** function rather than a zero-fallback, and
//! that is deliberate: nothing references function 0 (the entry is entered, not
//! branched to), so `0` in a body-reference byte would be a bug rather than a
//! sentinel — [`Program::references_are_resolvable`] is what says so.
//!
//! # What this does not do
//!
//! It does not mint keys. A stored program is N [`FunctionNode`](crate::FunctionNode)s
//! and each needs a GUID; minting is OGAR's and the `blockly-rs` app prefix is
//! an unminted operator decision. So a `Program` carries bodies and the caller
//! supplies keys — the same boundary [`crate::node`] draws.

use ogar_blockly::{Call, FunctionBody, LaneShape};

use crate::{BlockRecord, CastError, call_for, flow};

/// One script's functions. Index `0` is the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    /// The bodies, indexed by the byte a body-reference stores.
    pub functions: Vec<FunctionBody>,
}

impl Program {
    /// The entry body.
    #[must_use]
    pub fn entry(&self) -> &FunctionBody {
        &self.functions[0]
    }

    /// How many functions the script became.
    #[must_use]
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Whether the program holds no functions. Never true for a lowered
    /// script — the entry always exists.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }

    /// Every body-reference names a function that exists, and none names the
    /// entry.
    ///
    /// Both halves matter. An out-of-range index is a dangling branch; a
    /// reference to function `0` is a loop back into the entry, which the cast
    /// never emits and which would be an unbounded recursion if honoured.
    #[must_use]
    pub fn references_are_resolvable(&self) -> bool {
        for body in &self.functions {
            for call in body.calls() {
                let n = flow::body_refs(call.function);
                for slot in 0..usize::from(n) {
                    let idx = usize::from(call.values[slot]);
                    if idx == 0 || idx >= self.functions.len() {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Cast a script — including its control flow — into a program.
///
/// Straight-line scripts produce exactly one function, byte-identical to what
/// [`lower_script`](crate::lower_script) emits, so this is a strict widening
/// rather than a second cast.
///
/// # Errors
///
/// As [`lower_script`](crate::lower_script), plus
/// [`CastError::ShapeTooNarrow`] when a call's body references cannot fit the
/// shape's immediate width, and [`CastError::TooManyFunctions`] past 255.
pub fn lower_program(shape: LaneShape, top: &BlockRecord) -> Result<Program, CastError> {
    // The entry is reserved before the walk so that bodies discovered inside it
    // get indices 1.., and the entry keeps 0 no matter what order they appear.
    let mut functions: Vec<Option<FunctionBody>> = vec![None];
    let entry = lower_chain_into(shape, top, &mut functions)?;
    functions[0] = Some(entry);
    Ok(Program {
        functions: functions
            .into_iter()
            .map(|f| f.expect("every reserved slot is filled before return"))
            .collect(),
    })
}

/// Walk a statement chain into one body, appending any referenced bodies.
fn lower_chain_into(
    shape: LaneShape,
    block: &BlockRecord,
    functions: &mut Vec<Option<FunctionBody>>,
) -> Result<FunctionBody, CastError> {
    let mut body = FunctionBody::new(shape);
    let mut cursor = Some(block);
    while let Some(b) = cursor {
        lower_block_into(shape, b, &mut body, functions)?;
        cursor = b.next.as_deref();
    }
    Ok(body)
}

/// Emit one block: operands first (post-order), then the block's own call —
/// with its statement inputs lowered into separate functions first, so their
/// indices exist by the time the call that names them is built.
fn lower_block_into(
    shape: LaneShape,
    block: &BlockRecord,
    body: &mut FunctionBody,
    functions: &mut Vec<Option<FunctionBody>>,
) -> Result<(), CastError> {
    for (_, operand) in &block.inputs {
        lower_block_into(shape, operand, body, functions)?;
    }

    let mut call = call_for(block, None)?;

    if !block.statements.is_empty() {
        let refs = flow::body_refs(call.function);
        if refs == 0 {
            return Err(CastError::UnexpectedStatements {
                ty: block.ty.clone(),
                found: block.statements.len(),
            });
        }
        // A body reference is stored as an immediate, so the shape must be able
        // to hold as many as this call needs. IF_ELSE under Pairs would
        // truncate the else arm into nothing — the program would run the then
        // branch and silently skip the else, which is worse than refusing.
        if usize::from(refs) > shape.values_per_call() {
            return Err(CastError::ShapeTooNarrow {
                ty: block.ty.clone(),
                needed: usize::from(refs),
                shape,
            });
        }
        for slot in 0..usize::from(refs) {
            let Some((_, sub)) = block.statements.get(slot) else {
                // Fewer statement inputs than the opcode branches to — an
                // `if/else` with no else arm. Left as index 0, which
                // `references_are_resolvable` reports rather than silently
                // treating as a branch to the entry.
                continue;
            };
            // Reserve the index BEFORE lowering, so a parent's index is always
            // LOWER than its children's.
            //
            // Correcting an earlier claim here: this does not prevent a
            // collision. Lowering first and pushing after is also bijective —
            // it just numbers depth-first, so an inner body takes 1 and its
            // parent 2. Verified by injection: swapping the order left every
            // test passing, which is how the overstatement was caught. What
            // the reservation actually buys is a readable invariant — a
            // reference always points FORWARD — and that is what
            // `a_parents_index_precedes_its_childrens` pins.
            let idx = functions.len();
            if idx > 255 {
                return Err(CastError::TooManyFunctions { count: idx });
            }
            functions.push(None);
            let sub_body = lower_chain_into(shape, sub, functions)?;
            functions[idx] = Some(sub_body);
            call.values[slot] = u8::try_from(idx).expect("bounded above");
        }
    }

    body.push(call)?;
    Ok(())
}

/// Read a body's calls back as `(index, call)` pairs, resolving which value
/// bytes are branches.
///
/// Useful to a consumer walking a program: it says *which* bytes are function
/// indices without re-deriving [`flow::body_refs`].
#[must_use]
pub fn branches_of(body: &FunctionBody) -> Vec<(usize, Call, Vec<u8>)> {
    body.calls()
        .enumerate()
        .filter_map(|(i, c)| {
            let n = usize::from(flow::body_refs(c.function));
            (n > 0).then(|| (i, c, c.values[..n].to_vec()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FieldValue, lower_script, raise_calls};
    use ogar_blockly::FnIndex;

    const S: LaneShape = LaneShape::Pairs;

    fn number(id: &str, v: u8) -> BlockRecord {
        BlockRecord::leaf("math_number", id).with_field("NUM", FieldValue::Byte(v))
    }

    /// `repeat 10 [ print 1 ]`
    fn repeat_ten() -> BlockRecord {
        BlockRecord::leaf("controls_repeat", "loop")
            .with_input("TIMES", number("t", 10))
            .with_statement(
                "DO",
                BlockRecord::leaf("text_print", "p").with_input("TEXT", number("n", 1)),
            )
    }

    #[test]
    fn a_loop_becomes_two_functions_and_the_body_is_referenced_not_inlined() {
        let prog = lower_program(S, &repeat_ten()).unwrap();
        assert_eq!(prog.len(), 2, "caller + body");

        let entry = raise_calls(prog.entry());
        // The count is evaluated (an operand); the body is not.
        assert_eq!(
            entry.len(),
            2,
            "NUMBER then REPEAT — the body is NOT inline"
        );
        assert_eq!(entry[0].function, FnIndex::NUMBER);
        assert_eq!(entry[0].values[0], 10);
        assert_eq!(entry[1].function, FnIndex::REPEAT);
        // …and REPEAT's immediate is the body's INDEX.
        assert_eq!(entry[1].values[0], 1);

        // Function 1 is the body, and it really holds the body's calls.
        let sub = raise_calls(&prog.functions[1]);
        assert_eq!(sub.len(), 2);
        assert_eq!(sub[0].values[0], 1);
        assert_eq!(sub[1].function, FnIndex::PRINT);

        // ANTI-VACUITY: the body's calls must NOT also appear in the entry. An
        // implementation that inlined AND appended would satisfy every
        // assertion above.
        assert!(
            !entry.iter().any(|c| c.function == FnIndex::PRINT),
            "the body leaked into the caller — it was inlined, not referenced"
        );
    }

    #[test]
    fn a_straight_line_script_is_byte_identical_to_the_single_function_cast() {
        // Strict widening: `lower_program` must not change what already worked,
        // or every existing falsifier is measuring a different code path.
        let script = BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input("A", number("a", 5))
            .with_input("B", number("b", 3));
        let one = lower_script(S, &script).unwrap();
        let prog = lower_program(S, &script).unwrap();
        assert_eq!(prog.len(), 1, "no control flow, no extra function");
        assert_eq!(prog.entry().as_body_bytes(), one.as_body_bytes());
    }

    #[test]
    fn a_parents_index_precedes_its_childrens() {
        // The index is reserved BEFORE the body is lowered, so an inner loop
        // cannot take the slot its parent is about to claim. Without that
        // reservation the inner body would land on the outer's index and the
        // outer loop would branch to itself.
        let inner = BlockRecord::leaf("controls_repeat", "inner")
            .with_input("TIMES", number("i", 2))
            .with_statement(
                "DO",
                BlockRecord::leaf("text_print", "ip").with_input("TEXT", number("iv", 7)),
            );
        let outer = BlockRecord::leaf("controls_repeat", "outer")
            .with_input("TIMES", number("o", 3))
            .with_statement("DO", inner);

        let prog = lower_program(S, &outer).unwrap();
        assert_eq!(prog.len(), 3, "entry + outer body + inner body");
        assert!(prog.references_are_resolvable());

        let entry = raise_calls(prog.entry());
        let outer_ref = entry.last().unwrap().values[0];
        let outer_body = raise_calls(&prog.functions[usize::from(outer_ref)]);
        let inner_ref = outer_body.last().unwrap().values[0];
        assert_ne!(outer_ref, inner_ref, "two bodies must not share an index");
        assert!(
            outer_ref < inner_ref,
            "a reference must point FORWARD: parent {outer_ref} should precede child {inner_ref}"
        );
        assert_eq!(
            raise_calls(&prog.functions[usize::from(inner_ref)])[0].values[0],
            7,
            "the inner index must resolve to the INNER body"
        );
    }

    #[test]
    fn every_reference_resolves_and_none_points_at_the_entry() {
        let prog = lower_program(S, &repeat_ten()).unwrap();
        assert!(prog.references_are_resolvable());

        // Two-sided, and both halves are real failure modes. A dangling index:
        let mut dangling = prog.clone();
        let mut calls = raise_calls(&dangling.functions[0])
            .into_iter()
            .map(|c| Call::with_values(c.function, [c.values[0], 0, 0]))
            .collect::<Vec<_>>();
        calls.last_mut().unwrap().values[0] = 9;
        dangling.functions[0] = FunctionBody::from_calls(S, &calls).unwrap();
        assert!(
            !dangling.references_are_resolvable(),
            "a dangling branch must be caught"
        );

        // …and a branch back into the entry, which would be unbounded.
        calls.last_mut().unwrap().values[0] = 0;
        dangling.functions[0] = FunctionBody::from_calls(S, &calls).unwrap();
        assert!(
            !dangling.references_are_resolvable(),
            "a branch to the entry must be caught"
        );
    }

    #[test]
    fn if_else_is_refused_under_a_shape_that_would_truncate_its_else_arm() {
        // Two bodies need two immediates. Under Pairs the else arm would be
        // truncated to nothing and the program would run the then branch and
        // silently skip the else — worse than refusing, because it looks like
        // it worked.
        let ite = BlockRecord::leaf("controls_ifelse", "ite")
            .with_input("IF0", number("c", 1))
            .with_statement(
                "DO0",
                BlockRecord::leaf("text_print", "a").with_input("TEXT", number("x", 1)),
            )
            .with_statement(
                "ELSE",
                BlockRecord::leaf("text_print", "b").with_input("TEXT", number("y", 2)),
            );

        assert!(matches!(
            lower_program(LaneShape::Pairs, &ite),
            Err(CastError::ShapeTooNarrow { needed: 2, .. })
        ));
        // Two-sided: the wider shape accepts it, and BOTH arms become real,
        // distinct functions.
        let prog = lower_program(LaneShape::Triples, &ite).unwrap();
        assert_eq!(prog.len(), 3);
        assert!(prog.references_are_resolvable());
        let call = raise_calls(prog.entry()).pop().unwrap();
        assert_ne!(call.values[0], call.values[1], "then and else must differ");
    }

    #[test]
    fn statements_on_a_block_that_branches_to_nothing_are_refused() {
        // A statement input on `math_number` means the record and the codebook
        // disagree about the block's shape. Silently dropping it would discard
        // a whole body.
        let wrong = number("n", 1).with_statement("DO", BlockRecord::leaf("text_print", "p"));
        assert!(matches!(
            lower_program(S, &wrong),
            Err(CastError::UnexpectedStatements { found: 1, .. })
        ));
        // Two-sided: the same block without the statement lowers fine.
        assert!(lower_program(S, &number("n", 1)).is_ok());
    }

    #[test]
    fn branches_of_reports_which_bytes_are_function_indices() {
        let prog = lower_program(S, &repeat_ten()).unwrap();
        let b = branches_of(prog.entry());
        assert_eq!(b.len(), 1, "one branching call in the entry");
        let (idx, call, targets) = &b[0];
        assert_eq!(*idx, 1, "it is the second call");
        assert_eq!(call.function, FnIndex::REPEAT);
        assert_eq!(targets, &vec![1u8]);
        // Silence twin: a straight-line body reports NO branches, so this is
        // not a function that always finds something.
        assert!(branches_of(&prog.functions[1]).is_empty());
    }
}
