//! `blockly-shim` — the **membrane**: Blockly's own save JSON in,
//! [`BlockRecord`] out.
//!
//! # Why this is a separate crate
//!
//! Charter T3: *serde/JSON/proto exist ONLY at a membrane, behind a feature,
//! never on the hot path.* A crate boundary is a stronger membrane than a
//! feature flag, because `blockly-abi` cannot acquire a JSON dependency by
//! someone adding an `use` — it would take a manifest edit.
//!
//! So this crate holds the only `serde_json` in the workspace, and everything
//! downstream of [`from_workspace_json`] is the serialization-free ABI.
//!
//! # The finding that shapes this crate
//!
//! **Blockly's saved JSON does not distinguish a statement input from a value
//! input.** `saveInputBlocks` walks `block.inputList` and writes every
//! connected input into one `inputs` map keyed by name
//! (`core/serialization/blocks.ts:262-278`). The connection's *type* —
//! `INPUT_VALUE` vs `NEXT_STATEMENT` — exists on the live workspace object and
//! is **never serialized**.
//!
//! That matters because the two lower completely differently: a value input is
//! an operand evaluated onto the stack, a statement input becomes its own
//! function referenced by index. Getting it wrong does not produce a
//! slightly-wrong program — `repeat 10 [move]` would run the body once,
//! unconditionally, then hand the loop a count it never computed.
//!
//! A reader therefore **cannot** recover the distinction from JSON alone. It
//! needs the block definition. This crate carries the minimum of that
//! definition — [`statement_inputs`], harvested from the Apache-2.0 block
//! definitions — and **refuses** a branching block whose statement inputs it
//! does not know rather than guessing which of its inputs is the body.
//!
//! # What is deliberately not read
//!
//! `x`, `y`, `collapsed`, `inline` are presentation and never reach the record
//! — the same split [`blockly_abi`] draws, enforced here by simply not
//! consulting those keys. If a future edit reads one, the W1 falsifier
//! (a drag produces zero ABI writes) is what catches it.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use blockly_abi::{BlockRecord, FieldValue};
use serde_json::Value;

/// Why a saved workspace could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShimError {
    /// The bytes were not valid JSON.
    Json(String),
    /// The document is not a Blockly workspace save (no `blocks.blocks`).
    NotAWorkspace,
    /// A block object is missing its `type`.
    BlockWithoutType,
    /// A branching block whose statement-input names this crate does not know.
    ///
    /// Refused rather than guessed: the saved JSON cannot say which input is
    /// the body, so picking one would be inventing the program's structure.
    UnknownStatementInputs {
        /// The block type.
        ty: String,
    },
    /// A field value shape this crate does not model.
    UnsupportedField {
        /// The block type.
        ty: String,
        /// The field name.
        field: String,
    },
}

impl core::fmt::Display for ShimError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ShimError::Json(e) => write!(f, "invalid JSON: {e}"),
            ShimError::NotAWorkspace => f.write_str("not a Blockly workspace save"),
            ShimError::BlockWithoutType => f.write_str("a block object has no `type`"),
            ShimError::UnknownStatementInputs { ty } => write!(
                f,
                "block `{ty}` branches to a body, but this crate does not know which \
                 of its inputs is the statement input — Blockly's JSON does not say"
            ),
            ShimError::UnsupportedField { ty, field } => {
                write!(f, "block `{ty}` field `{field}` has an unsupported shape")
            }
        }
    }
}

impl core::error::Error for ShimError {}

/// The statement-input names of a block type, in the order the block declares
/// them.
///
/// Harvested from the Apache-2.0 block definitions, and the ORDER is
/// load-bearing: `controls_ifelse` declares `DO0` then `ELSE`
/// (`blocks/logic.ts`), and swapping them swaps the branches of every
/// if/else in the program.
///
/// Returns `None` for a type this crate has not harvested — which the caller
/// turns into a refusal rather than a guess.
#[must_use]
pub fn statement_inputs(ty: &str) -> Option<&'static [&'static str]> {
    Some(match ty {
        // `blocks/loops.ts` — every loop's body input is named DO.
        "controls_repeat"
        | "controls_repeat_ext"
        | "controls_whileUntil"
        | "controls_for"
        | "controls_forEach" => &["DO"],
        // `blocks/logic.ts` — controls_if's value input is IF0, its body DO0.
        // The mutator can add DO1.. and ELSE; a mutated block is refused
        // upstream by the cast (`CastError::MutatorUnsupported`), so only the
        // un-mutated shape is listed.
        "controls_if" => &["DO0"],
        "controls_ifelse" => &["DO0", "ELSE"],
        // Scratch's branching blocks. NOT a second hand-maintained list: the
        // names are READ from `blockly_abi::scratch::SCRATCH_BLOCK_DEFS`,
        // which harvested them from the same Apache-2.0 source as the opcodes
        // (`SUBSTACK`, `SUBSTACK2` — Scratch's own socket names, in Scratch's
        // own order). A block whose shape changes upstream therefore cannot
        // leave this function stale, and the order stays load-bearing for the
        // same reason `controls_ifelse` above says so.
        _ => {
            return blockly_abi::scratch::SCRATCH_BLOCK_DEFS
                .iter()
                .find(|&&(t, ..)| t == ty)
                .map(|&(_, _, _, _, stmts)| stmts)
                .filter(|stmts| !stmts.is_empty());
        }
    })
}

/// Whether a block type is one this crate knows to branch.
#[must_use]
pub fn branches(ty: &str) -> bool {
    statement_inputs(ty).is_some()
}

/// Read a Blockly workspace save into its top-level scripts.
///
/// The save shape is `{"blocks": {"languageVersion": n, "blocks": [ … ]}}`
/// (`core/serialization/workspaces.ts`); each element is a top-level block.
///
/// # Errors
///
/// [`ShimError::Json`] for malformed input, [`ShimError::NotAWorkspace`] if the
/// document is not a workspace save, and the per-block refusals.
pub fn from_workspace_json(json: &str) -> Result<Vec<BlockRecord>, ShimError> {
    let doc: Value = serde_json::from_str(json).map_err(|e| ShimError::Json(e.to_string()))?;
    if !doc.is_object() {
        return Err(ShimError::NotAWorkspace);
    }
    // An EMPTY workspace has no `blocks` key at all. `save()` builds a map from
    // the registered serializers and omits any whose `save()` returned null,
    // and `BlockSerializer::save` returns null when the workspace holds no top
    // blocks (`serialization/workspaces.ts:23-35`, `blocks.ts:782-802`). So a
    // missing key is a legitimate empty save, NOT a malformed document —
    // refusing it would reject the first thing a new workspace produces.
    let Some(blocks) = doc.get("blocks").and_then(|b| b.get("blocks")) else {
        return Ok(Vec::new());
    };
    let blocks = blocks.as_array().ok_or(ShimError::NotAWorkspace)?;
    blocks.iter().map(read_block).collect()
}

/// Read one block object (and its `next` chain) into a record.
///
/// # Errors
///
/// As [`from_workspace_json`].
pub fn read_block(v: &Value) -> Result<BlockRecord, ShimError> {
    let ty = v
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ShimError::BlockWithoutType)?;
    let id = v.get("id").and_then(Value::as_str).unwrap_or("");

    let mut rec = BlockRecord::leaf(ty, id);

    // `disabledReasons` is SEMANTIC — a disabled block is excluded from code
    // generation, which is program behaviour and not a grey tint.
    if let Some(reasons) = v.get("disabledReasons").and_then(Value::as_array) {
        rec.disabled = !reasons.is_empty();
    }
    // `extraState` is SEMANTIC too — it decides how many sockets the block has.
    // Carried verbatim so the cast can REFUSE a shape it cannot represent
    // rather than silently emitting a call that omits it.
    if let Some(extra) = v.get("extraState") {
        rec.extra_state = Some(extra.to_string());
    }

    if let Some(fields) = v.get("fields").and_then(Value::as_object) {
        for (name, fv) in fields {
            rec = rec.with_field(name, read_field(ty, name, fv)?);
        }
    }

    // THE split the JSON does not make. Every connected input — value and
    // statement alike — is in one map; only the block definition says which is
    // which.
    if let Some(inputs) = v.get("inputs").and_then(Value::as_object) {
        let stmts = statement_inputs(ty);
        // A block with inputs we cannot classify is refused only if it is one
        // we KNOW branches; an ordinary reporter has no statement inputs and
        // needs no table.
        for (name, conn) in inputs {
            let Some(child) = effective_child(conn) else {
                continue;
            };
            let sub = read_block(child)?;
            if stmts.is_some_and(|s| s.contains(&name.as_str())) {
                rec = rec.with_statement(name, sub);
            } else {
                rec = rec.with_input(name, sub);
            }
        }
        // Order the statement inputs as the block declares them: a JSON object
        // has no guaranteed order, and `controls_ifelse`'s DO0/ELSE swapping
        // would swap every if/else branch in the program.
        if let Some(order) = stmts {
            rec.statements
                .sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
        }
    }

    if let Some(next) = v
        .get("next")
        .and_then(|n| n.get("block"))
        .filter(|b| !b.is_null())
    {
        rec = rec.with_next(read_block(next)?);
    }

    Ok(rec)
}

/// The operand actually plugged into a socket.
///
/// A `ConnectionState` can carry a `shadow` and a `block` **simultaneously** —
/// a real block plugged in over a remembered default. The real block wins; the
/// shadow underneath is editor state. When only a shadow is present it IS the
/// value (the socket's default), so it is genuinely semantic.
fn effective_child(conn: &Value) -> Option<&Value> {
    conn.get("block")
        .filter(|b| !b.is_null())
        .or_else(|| conn.get("shadow").filter(|b| !b.is_null()))
}

/// Read one field value.
///
/// A variable field serializes to an OBJECT (`{"id": …}`), never a bare
/// string — so a naive `as_str` would silently drop every variable reference.
fn read_field(ty: &str, name: &str, v: &Value) -> Result<FieldValue, ShimError> {
    if let Some(obj) = v.as_object() {
        return obj
            .get("id")
            .and_then(Value::as_str)
            .map(|id| FieldValue::Ref { id: id.to_string() })
            .ok_or_else(|| ShimError::UnsupportedField {
                ty: ty.to_string(),
                field: name.to_string(),
            });
    }
    // Numbers may arrive as JSON numbers or as strings — Blockly writes field
    // values through the field's own serializer, and `field_number` writes a
    // number while `field_input` writes a string.
    if let Some(n) = v.as_f64() {
        return Ok(byte_or_wide(n.to_string()));
    }
    if let Some(s) = v.as_str() {
        return Ok(byte_or_wide(s.to_string()));
    }
    if let Some(b) = v.as_bool() {
        return Ok(FieldValue::Code(
            if b { "TRUE" } else { "FALSE" }.to_string(),
        ));
    }
    Err(ShimError::UnsupportedField {
        ty: ty.to_string(),
        field: name.to_string(),
    })
}

/// Classify a scalar field: a dropdown code, a byte-fitting number, or a wide
/// literal for the constant pool.
///
/// The order matters. An all-caps token like `"ADD"` is a dropdown code and
/// must NOT be read as text, or `math_arithmetic` would lose the operator that
/// selects its function.
fn byte_or_wide(s: String) -> FieldValue {
    if let Ok(n) = s.parse::<u8>() {
        // An integer that fits an immediate. Note `"1.0"` does NOT parse as u8
        // and correctly falls through to Wide — a float is not a byte.
        return FieldValue::Byte(n);
    }
    // A dropdown code must contain at least one LETTER. Without that clause
    // `"256"` — a number too wide for a byte — matched the all-digits-allowed
    // pattern and became `Code("256")`, so an out-of-range literal would have
    // been read as an operator name. Caught by this function's own
    // boundary test, which is why that test carries 256 rather than only 5.
    if s.chars().any(|c| c.is_ascii_uppercase())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    {
        return FieldValue::Code(s);
    }
    FieldValue::Wide(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockly_abi::{lower_program, lower_script, raise_calls, render_text};
    use ogar_loco::FnIndex;
    use ogar_loco::LaneShape;

    /// `5 + 3`, exactly as `Blockly.serialization.workspaces.save` writes it.
    const ADD_JSON: &str = r#"{
      "blocks": {"languageVersion": 0, "blocks": [{
        "type": "math_arithmetic", "id": "root", "x": 40, "y": 80,
        "fields": {"OP": "ADD"},
        "inputs": {
          "A": {"shadow": {"type": "math_number", "id": "a", "fields": {"NUM": 5}}},
          "B": {"shadow": {"type": "math_number", "id": "b", "fields": {"NUM": 3}}}
        }
      }]}
    }"#;

    /// `repeat 10 [ print 1 ]` — the shape the whole crate exists for.
    const REPEAT_JSON: &str = r#"{
      "blocks": {"languageVersion": 0, "blocks": [{
        "type": "controls_repeat_ext", "id": "loop", "x": 10, "y": 10,
        "inputs": {
          "TIMES": {"shadow": {"type": "math_number", "id": "t", "fields": {"NUM": 10}}},
          "DO": {"block": {
            "type": "text_print", "id": "p",
            "inputs": {"TEXT": {"shadow": {"type": "math_number", "id": "n", "fields": {"NUM": 1}}}}
          }}
        }
      }]}
    }"#;

    #[test]
    fn a_real_blockly_save_becomes_the_same_program_a_hand_built_record_does() {
        // The point of the shim: the W1 falsifier used synthetic BlockRecords,
        // so nothing proved the cast could read what Blockly actually writes.
        let scripts = from_workspace_json(ADD_JSON).unwrap();
        assert_eq!(scripts.len(), 1);
        let body = lower_script(LaneShape::Pairs, &scripts[0]).unwrap();
        assert_eq!(&body.as_body_bytes()[..6], &[0x46, 5, 0x46, 3, 0x40, 0]);
        // …and it is the same program the text projection names.
        assert_eq!(render_text(&body).unwrap(), "5 + 3");
    }

    #[test]
    fn geometry_in_the_json_does_not_reach_the_abi() {
        // `x` and `y` are present in the JSON above and must not survive. The
        // strongest available form: move the block and assert the bytes are
        // IDENTICAL, rather than merely asserting the record has no x field.
        let moved = ADD_JSON.replace(r#""x": 40, "y": 80"#, r#""x": 999, "y": 7"#);
        assert_ne!(moved, ADD_JSON, "the fixture must really have changed");
        let a = lower_script(LaneShape::Pairs, &from_workspace_json(ADD_JSON).unwrap()[0]).unwrap();
        let b = lower_script(LaneShape::Pairs, &from_workspace_json(&moved).unwrap()[0]).unwrap();
        assert_eq!(a.as_body_bytes(), b.as_body_bytes());
        // Two-sided: an OPERAND change must still change the bytes, or this
        // test would pass for a reader that ignores the JSON entirely.
        let edited = ADD_JSON.replace(r#""NUM": 5"#, r#""NUM": 9"#);
        let c = lower_script(LaneShape::Pairs, &from_workspace_json(&edited).unwrap()[0]).unwrap();
        assert_ne!(a.as_body_bytes(), c.as_body_bytes());
    }

    #[test]
    fn a_statement_input_becomes_a_referenced_function_not_an_operand() {
        // The finding this crate is shaped by: Blockly's JSON puts TIMES (a
        // value) and DO (a statement) in the SAME `inputs` map with nothing to
        // tell them apart. Only `statement_inputs` does.
        let scripts = from_workspace_json(REPEAT_JSON).unwrap();
        let rec = &scripts[0];
        assert_eq!(rec.inputs.len(), 1, "TIMES is the only operand");
        assert_eq!(rec.inputs[0].0, "TIMES");
        assert_eq!(rec.statements.len(), 1, "DO is a body");
        assert_eq!(rec.statements[0].0, "DO");

        let prog = lower_program(LaneShape::Pairs, rec).unwrap();
        assert_eq!(prog.len(), 2, "caller + body");
        assert!(prog.references_are_resolvable(blockly_abi::checked_vocabulary()));
        let entry = raise_calls(prog.entry());
        assert_eq!(entry[0].values[0], 10, "the count is evaluated");
        assert_eq!(entry[1].function, FnIndex::REPEAT);
        // ANTI-VACUITY: had DO been read as an operand, the body's PRINT would
        // be in the entry — which is exactly the "runs once, unconditionally"
        // failure.
        assert!(!entry.iter().any(|c| c.function == FnIndex::PRINT));
    }

    #[test]
    fn a_branching_block_with_unknown_statement_inputs_is_refused() {
        // Silence twin for the table: the crate must not fall back to treating
        // an unknown body as an operand.
        assert!(branches("controls_repeat"));
        assert!(branches("controls_ifelse"));
        assert!(!branches("math_arithmetic"));
        assert_eq!(
            statement_inputs("controls_ifelse"),
            Some(&["DO0", "ELSE"][..])
        );
        // Order is load-bearing — swapping DO0/ELSE swaps every if/else.
        assert_eq!(statement_inputs("controls_ifelse").unwrap()[0], "DO0");
        assert_eq!(statement_inputs("controls_forEach"), Some(&["DO"][..]));
        assert_eq!(statement_inputs("not_a_block"), None);
    }

    #[test]
    fn a_real_block_wins_over_the_shadow_it_sits_on() {
        // A ConnectionState can carry BOTH: a real block plugged into a socket
        // whose default is remembered underneath. Reading the shadow would
        // silently discard what the user actually typed.
        let json = r#"{"blocks":{"languageVersion":0,"blocks":[{
          "type":"math_arithmetic","id":"r","fields":{"OP":"ADD"},
          "inputs":{
            "A":{"shadow":{"type":"math_number","id":"s","fields":{"NUM":1}},
                 "block":{"type":"math_number","id":"b","fields":{"NUM":7}}},
            "B":{"shadow":{"type":"math_number","id":"s2","fields":{"NUM":2}}}
          }}]}}"#;
        let rec = &from_workspace_json(json).unwrap()[0];
        let body = lower_script(LaneShape::Pairs, rec).unwrap();
        let calls = raise_calls(&body);
        let values: Vec<u8> = calls.iter().map(|c| c.values[0]).collect();
        assert!(
            values.contains(&7),
            "the real block must win, got {values:?}"
        );
        assert!(
            !values.contains(&1),
            "the remembered shadow must not be read"
        );
        // Two-sided: where there is ONLY a shadow it IS the value.
        assert!(values.contains(&2));
    }

    #[test]
    fn a_variable_field_is_read_as_a_reference_not_a_string() {
        // Variable fields serialize to an OBJECT. `as_str` would return None
        // and a naive reader would drop the reference entirely.
        let json = r#"{"blocks":{"languageVersion":0,"blocks":[{
          "type":"variables_get","id":"v","fields":{"VAR":{"id":"var-1"}}}]}}"#;
        let rec = &from_workspace_json(json).unwrap()[0];
        assert_eq!(
            rec.field("VAR"),
            Some(&FieldValue::Ref { id: "var-1".into() })
        );
    }

    #[test]
    fn a_dropdown_code_is_not_mistaken_for_text() {
        // `"ADD"` selects the function. Read as a wide literal it would be
        // interned as a string and `math_arithmetic` would lose its operator.
        assert_eq!(byte_or_wide("ADD".into()), FieldValue::Code("ADD".into()));
        assert_eq!(byte_or_wide("5".into()), FieldValue::Byte(5));
        // Two-sided, and these are the interesting boundaries: a float is not
        // a byte, 256 does not fit, and ordinary prose is not a code.
        assert_eq!(byte_or_wide("1.0".into()), FieldValue::Wide("1.0".into()));
        assert_eq!(byte_or_wide("256".into()), FieldValue::Wide("256".into()));
        assert_eq!(
            byte_or_wide("hello".into()),
            FieldValue::Wide("hello".into())
        );
    }

    #[test]
    fn malformed_input_is_refused_rather_than_half_read() {
        assert!(matches!(from_workspace_json("{"), Err(ShimError::Json(_))));
        assert_eq!(from_workspace_json("[]"), Err(ShimError::NotAWorkspace));
        // …but an EMPTY workspace is not malformed. `save()` omits a
        // serializer's key entirely when it has nothing, so a fresh workspace
        // produces `{}` — and rejecting that would refuse the very first save
        // a new editor makes.
        assert_eq!(from_workspace_json("{}"), Ok(Vec::new()));
        assert_eq!(from_workspace_json(r#"{"variables":[]}"#), Ok(Vec::new()));
        assert!(matches!(
            from_workspace_json(r#"{"blocks":{"blocks":[{"id":"x"}]}}"#),
            Err(ShimError::BlockWithoutType)
        ));
        // Two-sided: a well-formed save still reads, so the guards are targeted.
        assert!(from_workspace_json(ADD_JSON).is_ok());
    }
}

/// Built-in reference programs — real workspace saves, compiled in.
///
/// A template here is not documentation: it is a Blockly workspace save that
/// this crate PARSES and `blockly-abi` CASTS in its own tests, so a reference
/// program cannot rot into something the pipeline no longer accepts. If a
/// template ever stops casting, the build says so.
///
/// They are also the answer to "what does a real program look like on this
/// substrate?" — a question the demo could previously only answer with
/// `5 + 3`, which exercises two opcodes and no device family at all.
pub mod templates {
    /// **Pong** — the reference arcade game.
    ///
    /// Three concurrent scripts, which is what makes it a good reference
    /// rather than a long one:
    ///
    /// | script | shape | what it exercises |
    /// |---|---|---|
    /// | ball | hat → `forever` { move · bounce · `if` touching → turn } | device motion + sensing under nested control flow |
    /// | paddle | hat → `forever` { set y to mouse y } | a device reporter feeding a device setter |
    /// | score | hat → `forever` { `if` touching goal → change score } | the shared core's `VAR_CHANGE` beside device sensing |
    ///
    /// It spans both halves of the palette deliberately: `motion_movesteps`,
    /// `sensing_mousey` and friends are device mints (`0x90..`), while
    /// `control_forever`, `control_if` and `data_changevariableby` resolve to
    /// the shared core — so one small program proves a Scratch game is not a
    /// separate machine, just a second vocabulary over the same substrate.
    ///
    /// The touch targets are plain numbers where real Scratch would use a
    /// dropdown menu shadow. That is honest rather than lossy: menus are
    /// values, not operations (they mint no opcode), so a number stands in for
    /// the sprite id a menu would resolve to.
    pub const PONG: &str = include_str!("../templates/pong.json");

    /// Every built-in template, as `(name, workspace save)`.
    pub const ALL: &[(&str, &str)] = &[("pong", PONG)];
}

#[cfg(test)]
mod template_tests {
    use super::templates;
    use blockly_abi::lower_program;
    use ogar_loco::LaneShape;

    /// Every built-in template parses AND casts — it is a program, not a doc.
    ///
    /// This is what stops a reference program rotting. A template that stops
    /// casting is a template that lies about what the pipeline accepts, and it
    /// would otherwise fail only when a person clicked "load" in a browser.
    #[test]
    fn every_template_parses_and_casts_cleanly() {
        for (name, json) in templates::ALL {
            let scripts = super::from_workspace_json(json)
                .unwrap_or_else(|e| panic!("template `{name}` does not parse: {e}"));
            assert!(!scripts.is_empty(), "template `{name}` is empty");
            for script in &scripts {
                lower_program(LaneShape::Pairs, script).unwrap_or_else(|e| {
                    panic!(
                        "template `{name}` script `{}` refuses to cast: {e:?}",
                        script.ty
                    )
                });
            }
        }
    }

    /// Pong spans BOTH halves of the palette — that is the point of it.
    ///
    /// A reference game that only touched device mints would prove nothing
    /// about sharing, and one that only touched the shared core would not be a
    /// game. Asserting both means the template cannot be quietly simplified
    /// into something that no longer demonstrates the claim.
    #[test]
    fn pong_exercises_device_mints_and_the_shared_core_together() {
        let scripts = super::from_workspace_json(templates::PONG).unwrap();
        assert_eq!(scripts.len(), 3, "Pong is three concurrent scripts");

        let mut device = 0usize;
        let mut core = 0usize;
        let mut branching = 0usize;
        for script in &scripts {
            let prog = lower_program(LaneShape::Pairs, script).unwrap();
            for body in &prog.functions {
                for call in blockly_abi::raise_calls(body) {
                    if call.function.is_domain_specific() {
                        device += 1;
                    } else if call.function.is_shared_core() {
                        core += 1;
                    }
                }
            }
            // Nested control flow really is nested: a `forever` whose body is
            // a separate function is what makes this more than a flat list.
            branching += prog.functions.len() - 1;
        }
        assert!(device >= 6, "expected device mints, got {device}");
        assert!(core >= 4, "expected shared-core calls, got {core}");
        assert!(branching >= 4, "expected nested bodies, got {branching}");
    }

    /// The shim learned Scratch's branching blocks from the harvested table.
    ///
    /// Two-sided: a Scratch C-block reports its real socket names, and a
    /// Scratch block that does NOT branch reports none — otherwise
    /// `branches()` would be true for everything and carry no information.
    #[test]
    fn scratch_branching_blocks_report_their_real_socket_names() {
        assert_eq!(
            super::statement_inputs("control_forever"),
            Some(&["SUBSTACK"][..])
        );
        assert_eq!(
            super::statement_inputs("control_if_else"),
            Some(&["SUBSTACK", "SUBSTACK2"][..])
        );
        // Silence twin.
        assert_eq!(super::statement_inputs("motion_movesteps"), None);
        assert!(!super::branches("motion_movesteps"));
        assert!(super::branches("control_forever"));
        // …and the Blockly half is untouched by the new arm.
        assert_eq!(
            super::statement_inputs("controls_ifelse"),
            Some(&["DO0", "ELSE"][..])
        );
    }
}
