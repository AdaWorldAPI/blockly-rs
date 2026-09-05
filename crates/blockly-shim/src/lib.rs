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

pub mod assets;
pub mod sb3;

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
    /// The touch targets are `sensing_touchingobjectmenu` shadows naming
    /// `Paddle` and `Goal` — sprite names, which scratch-blocks registers as an
    /// EMPTY menu (the GUI fills it per project). They resolve through the
    /// demo's [`project_basin`]: a dynamic menu is one `FnIndex` whose operand
    /// indexes a per-project table (OGAR #295), and this is the project.
    pub const PONG: &str = include_str!("../templates/pong.json");

    /// Every built-in template, as `(name, workspace save)`.
    ///
    /// The JSON is the AUTHORING form, kept so a template stays readable and
    /// diffable in review. The artefact the demo serves is
    /// [`PONG_NODES`] — see [`raise_nodes`].
    pub const ALL: &[(&str, &str)] = &[("pong", PONG), ("pong-keys", PONG_KEYS)];

    /// **Pong, keyboard edition** — the same ball and score scripts, but the
    /// paddle reads the keyboard: `forever { if <key [up arrow] pressed?>
    /// change y by 6; if <key [down arrow] pressed?> change y by (0 - 6) }`.
    ///
    /// The keys arrive as `sensing_keyoptions` menu shadows — codebook
    /// indices into `KEY_OPTION`, not opcodes (OGAR #295 via
    /// `blockly_abi::menus`). `0 - 6` because a template literal is one
    /// immediate byte; a negative number would need the constant pool, and
    /// the subtraction says the same thing in shared-core bytes.
    pub const PONG_KEYS: &str = include_str!("../templates/pong-keys.json");

    /// Pong (keyboard edition) as its stored nodes.
    pub const PONG_KEYS_NODES: &[u8] = include_bytes!("../templates/pong-keys.nodes");

    /// Pong as its STORED NODES — the program, not the projection.
    ///
    /// Layout: one byte of script count, then one byte per script giving its
    /// function count, then the 512-byte nodes back to back. 4100 bytes for
    /// Pong's 8 functions across 3 scripts.
    ///
    /// Regenerate with
    /// `cargo run -p blockly-shim --example bake_template`; the round-trip
    /// test is what proves the bake and the JSON still agree.
    pub const PONG_NODES: &[u8] = include_bytes!("../templates/pong.nodes");

    /// Every built-in template in stored-node form.
    pub const ALL_NODES: &[(&str, &[u8])] = &[("pong", PONG_NODES), ("pong-keys", PONG_KEYS_NODES)];

    /// The demo project's basin: every static menu prefix, plus the sprite
    /// names the built-in scenes use, interned into the dynamic menus
    /// (`TOUCHING_OBJECT`, `DISTANCE_TO`, `POINT_TOWARDS`, `GOTO`, `GLIDE_TO`,
    /// `OF_OBJECT`). Built once. A real project would build its own from its
    /// own sprite list — this is what "the basin is the project" means.
    pub fn project_basin() -> &'static ogar_loco::basin::BasinCodebooks {
        use blockly_abi::menus;
        use ogar_loco::basin::BasinCodebooks;
        use std::sync::OnceLock;
        static BASIN: OnceLock<BasinCodebooks> = OnceLock::new();
        BASIN.get_or_init(|| {
            const UTF8: u32 = ogar_loco::pool::placeholder::CONST_UTF8_INLINE;
            const SPRITES: &[&str] = &["Ball", "Paddle", "Goal"];
            let mut basin = BasinCodebooks::new();
            for m in menus::SCRATCH_MENUS {
                let mut b = menus::builder(m, UTF8, menus::PLACEHOLDER_DIGEST_CLASSID)
                    .expect("prefix fits");
                let sprite_menu = matches!(
                    m.name,
                    "TOUCHING_OBJECT"
                        | "DISTANCE_TO"
                        | "POINT_TOWARDS"
                        | "GOTO"
                        | "GLIDE_TO"
                        | "OF_OBJECT"
                );
                if sprite_menu {
                    // The pointer and the edge come before any sprite, as
                    // Scratch lists them; `OF_OBJECT` offers the stage instead.
                    let fixed: &[&str] = match m.name {
                        "TOUCHING_OBJECT" => &["_mouse_", "_edge_"],
                        "DISTANCE_TO" | "POINT_TOWARDS" | "GOTO" | "GLIDE_TO" => {
                            &["_mouse_", "_random_"]
                        }
                        _ => &["_stage_"],
                    };
                    for name in fixed.iter().chain(SPRITES) {
                        b.intern(UTF8, name.as_bytes())
                            .expect("short names fit a facet");
                    }
                }
                basin.plug(b.seal()).expect("menu ids are unique");
            }
            basin
        })
    }

    /// Cast a script against the demo project's basin — [`lower_program_in`]
    /// with [`project_basin`]. Every template test and the web demo go
    /// through this, so a sprite-name menu casts the same way everywhere.
    ///
    /// # Errors
    ///
    /// As [`lower_program_in`].
    ///
    /// [`lower_program_in`]: blockly_abi::lower_program_in
    pub fn cast(
        shape: ogar_loco::LaneShape,
        top: &blockly_abi::BlockRecord,
    ) -> Result<blockly_abi::Program, blockly_abi::CastError> {
        blockly_abi::lower_program_in(shape, top, project_basin())
    }

    /// Raise stored nodes back into top-level scripts.
    ///
    /// This is what lets the template ship as bytes: the editor asks for
    /// blocks, and they are RECONSTRUCTED from the program rather than read
    /// from a parallel JSON copy that could drift from it.
    ///
    /// # Errors
    ///
    /// Propagates [`blockly_abi::raise::RaiseError`]; a malformed header
    /// yields an empty result rather than a panic.
    pub fn raise_nodes(
        bytes: &[u8],
    ) -> Result<Vec<blockly_abi::BlockRecord>, blockly_abi::raise::RaiseError> {
        use ogar_loco::node::NODE_BYTES;
        let Some((&count, rest)) = bytes.split_first() else {
            return Ok(Vec::new());
        };
        let n = usize::from(count);
        if rest.len() < n {
            return Ok(Vec::new());
        }
        let (counts, mut nodes) = rest.split_at(n);
        let mut out = Vec::new();
        for &fc in counts {
            let take = usize::from(fc) * NODE_BYTES;
            if nodes.len() < take {
                break;
            }
            let (mine, tail) = nodes.split_at(take);
            nodes = tail;
            let bodies: Vec<ogar_loco::FunctionBody> = mine
                .chunks_exact(NODE_BYTES)
                .map(|c| {
                    blockly_abi::FunctionNode::from_le_bytes(
                        c.try_into().expect("chunks_exact yields NODE_BYTES"),
                        ogar_loco::LaneShape::Pairs,
                    )
                    .body
                })
                .collect();
            if let Some(script) = blockly_abi::raise::raise_program_in(&bodies, project_basin())? {
                out.push(script);
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod template_tests {
    use super::templates;
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
                templates::cast(LaneShape::Pairs, script).unwrap_or_else(|e| {
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
            let prog = templates::cast(LaneShape::Pairs, script).unwrap();
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

#[cfg(test)]
mod raise_round_trip {
    use super::templates;
    use blockly_abi::raise::raise_program_in;
    use ogar_loco::LaneShape;

    /// Pong survives a full trip through the STORED BYTES and back.
    ///
    /// JSON → cast → bytes → raise → cast → bytes, compared byte-for-byte.
    /// This is what makes a bytes-only template safe to ship: it proves the
    /// editor's JSON is a *rendering* of the stored program, recoverable from
    /// the nodes, rather than a second source of truth that merely happens to
    /// agree.
    ///
    /// Pong is the right fixture because it is the hardest one available —
    /// three scripts, nested `forever`/`if` bodies, device mints and shared
    /// core mixed, operands under statement bodies.
    #[test]
    fn pong_survives_a_round_trip_through_its_stored_bytes() {
        let scripts = super::from_workspace_json(templates::PONG).unwrap();
        assert_eq!(scripts.len(), 3);

        for (i, script) in scripts.iter().enumerate() {
            let original = templates::cast(LaneShape::Pairs, script).expect("casts");
            let raised = raise_program_in(&original.functions, templates::project_basin())
                .expect("raises")
                .expect("non-empty");
            let again = templates::cast(LaneShape::Pairs, &raised).expect("re-casts");

            assert_eq!(
                original.functions.len(),
                again.functions.len(),
                "script {i}: function count changed"
            );
            for (f, (a, b)) in original
                .functions
                .iter()
                .zip(again.functions.iter())
                .enumerate()
            {
                assert_eq!(
                    a.as_body_bytes(),
                    b.as_body_bytes(),
                    "script {i} function {f} differs after the round trip"
                );
            }
        }
    }
}

/// The membrane's WRITE direction: records back out as Blockly's save JSON.
///
/// The read direction alone made JSON the only durable form a program could
/// take. With this, plus `blockly_abi::raise`, the stored NODES are the
/// artefact and the editor's JSON is produced on demand — a rendering, exactly
/// like the askama surface, and for the same reason.
pub mod emit {
    use super::Value;
    use blockly_abi::{BlockRecord, FieldValue};

    fn block_json(b: &BlockRecord) -> Value {
        let mut o = serde_json::Map::new();
        o.insert("type".into(), Value::String(b.ty.clone()));
        o.insert("id".into(), Value::String(b.id.clone()));
        if !b.fields.is_empty() {
            let mut f = serde_json::Map::new();
            for (k, v) in &b.fields {
                f.insert(
                    k.clone(),
                    match v {
                        FieldValue::Byte(n) => Value::from(*n),
                        FieldValue::Code(c) => Value::String(c.clone()),
                        FieldValue::Wide(w) => Value::String(w.clone()),
                        FieldValue::Ref { id } => {
                            serde_json::json!({ "id": id })
                        }
                    },
                );
            }
            o.insert("fields".into(), Value::Object(f));
        }
        // Value inputs and statement inputs share one `inputs` map on the
        // wire — Blockly's own save does not distinguish them either (that is
        // the finding this crate exists for). The distinction is recovered on
        // read via `statement_inputs`, so writing them together is faithful.
        let mut ins = serde_json::Map::new();
        for (name, child) in b.inputs.iter().chain(b.statements.iter()) {
            ins.insert(
                name.clone(),
                serde_json::json!({ "block": block_json(child) }),
            );
        }
        if !ins.is_empty() {
            o.insert("inputs".into(), Value::Object(ins));
        }
        if let Some(n) = &b.next {
            o.insert("next".into(), serde_json::json!({ "block": block_json(n) }));
        }
        Value::Object(o)
    }

    /// Render top-level scripts as a Blockly workspace save.
    ///
    /// # Where the coordinates come from
    ///
    /// They are INVENTED HERE, and that is the correct place. `x`/`y` are
    /// presentation: the ABI excludes them on purpose — *"moving a block 40
    /// pixels must not touch the ABI"* is the arc's W1 falsifier — so stored
    /// nodes carry no layout at all.
    ///
    /// Which means a raised program has nowhere to get one, and without this
    /// every top-level script lands at (0,0) and they stack into an unreadable
    /// pile. That is not hypothetical: the deployed demo rendered Pong's three
    /// scripts on top of each other.
    ///
    /// So the membrane assigns a layout when it renders, exactly as it
    /// assigns JSON — a projection detail produced on demand, never stored.
    /// The spacing is a plain column; a real editor would let the user move
    /// them, and moving them still would not touch the bytes.
    #[must_use]
    pub fn to_workspace_json(scripts: &[BlockRecord]) -> String {
        /// Horizontal inset, and the vertical step between scripts. Generous
        /// enough that a tall script (Pong's ball is a `forever` holding an
        /// `if`) does not overlap the next one.
        const X: i64 = 40;
        const Y0: i64 = 30;
        const DY: i64 = 260;

        let blocks: Vec<Value> = scripts
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut v = block_json(s);
                if let Some(o) = v.as_object_mut() {
                    o.insert("x".into(), Value::from(X));
                    o.insert("y".into(), Value::from(Y0 + DY * i as i64));
                }
                v
            })
            .collect();
        serde_json::json!({"blocks": {"languageVersion": 0, "blocks": blocks}}).to_string()
    }
}

#[cfg(test)]
mod baked_nodes {
    use super::templates;
    use ogar_loco::LaneShape;

    /// The BAKED NODES and the authoring JSON describe the same program.
    ///
    /// This is the gate that lets the demo serve bytes. If someone edits
    /// `pong.json` and forgets to re-bake, or the bake drifts from the cast,
    /// the two stop agreeing and this fails — so the stored artefact can never
    /// silently diverge from the form a human reviews.
    #[test]
    fn the_baked_nodes_reproduce_the_authoring_json_byte_for_byte() {
        // Every template, not just the first: a second template whose bake
        // was forgotten would otherwise pass here.
        assert_eq!(templates::ALL.len(), templates::ALL_NODES.len());
        for ((name, json), (nname, nodes)) in templates::ALL.iter().zip(templates::ALL_NODES) {
            assert_eq!(name, nname, "ALL and ALL_NODES are out of order");
            let from_json = super::from_workspace_json(json).unwrap();
            let from_nodes = templates::raise_nodes(nodes).expect("raises");

            assert_eq!(
                from_json.len(),
                from_nodes.len(),
                "{name}: script count differs between the JSON and the baked nodes"
            );

            for (i, (j, n)) in from_json.iter().zip(from_nodes.iter()).enumerate() {
                let a = templates::cast(LaneShape::Pairs, j).expect("json casts");
                let b = templates::cast(LaneShape::Pairs, n).expect("raised casts");
                assert_eq!(a.functions.len(), b.functions.len(), "{name} script {i}");
                for (f, (x, y)) in a.functions.iter().zip(b.functions.iter()).enumerate() {
                    assert_eq!(
                        x.as_body_bytes(),
                        y.as_body_bytes(),
                        "{name} script {i} function {f}: the bake and the JSON disagree — \
                         re-run `cargo run -p blockly-shim --example bake_template`"
                    );
                }
            }
        }
    }

    /// A workspace rendered FROM the nodes is loadable by the editor.
    ///
    /// The round trip that matters for the app: bytes → blocks → JSON → blocks
    /// → bytes. If the emit half dropped a socket or a `next` link, the final
    /// bytes diverge.
    #[test]
    fn nodes_render_to_json_the_membrane_can_read_back() {
        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        let json = super::emit::to_workspace_json(&scripts);
        let reread = super::from_workspace_json(&json).expect("the emitted JSON parses");

        assert_eq!(scripts.len(), reread.len());
        for (i, (a, b)) in scripts.iter().zip(reread.iter()).enumerate() {
            let x = templates::cast(LaneShape::Pairs, a).expect("casts");
            let y = templates::cast(LaneShape::Pairs, b).expect("casts");
            for (f, (p, q)) in x.functions.iter().zip(y.functions.iter()).enumerate() {
                assert_eq!(
                    p.as_body_bytes(),
                    q.as_body_bytes(),
                    "script {i} function {f} lost information through the emit/read cycle"
                );
            }
        }
    }
}

#[cfg(test)]
mod pong_runs {
    use super::templates;
    use blockly_run::Machine;
    use ogar_loco::LaneShape;

    /// Pong RUNS from its stored nodes — the ball moves, the paddle tracks
    /// the mouse, the score counts.
    ///
    /// The whole arc in one test: nodes → bodies → execution, with no JSON and
    /// no block tree in the path. Each script is asserted for the effect it is
    /// supposed to have, so a run that merely completed without panicking
    /// would still fail.
    #[test]
    fn pong_runs_from_its_stored_nodes() {
        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        assert_eq!(scripts.len(), 3);

        // Script 0 — the ball moves.
        let ball = templates::cast(LaneShape::Pairs, &scripts[0]).expect("casts");
        let mut m = Machine::new(&ball.functions, 400);
        m.run().expect("the ball script runs");
        assert!(
            m.stage.sprites[0].x != 0.0 || m.stage.sprites[0].y != 0.0,
            "the ball must have moved: ({}, {})",
            m.stage.sprites[0].x,
            m.stage.sprites[0].y
        );

        // Script 1 — the paddle follows the mouse.
        let paddle = templates::cast(LaneShape::Pairs, &scripts[1]).expect("casts");
        let mut p = Machine::new(&paddle.functions, 200);
        p.stage.mouse_y = 42.0;
        p.run().expect("the paddle script runs");
        assert_eq!(
            p.stage.sprites[0].y, 42.0,
            "the paddle must track the mouse"
        );

        // Script 2 — scoring, and it must NOT score when nothing is touched.
        let score = templates::cast(LaneShape::Pairs, &scripts[2]).expect("casts");
        let mut hit = Machine::new(&score.functions, 200);
        hit.stage.touching = true;
        hit.run().expect("the score script runs");
        assert!(hit.stage.var(0) > 0.0, "touching the goal must score");

        let mut miss = Machine::new(&score.functions, 200);
        miss.stage.touching = false;
        miss.run().expect("runs");
        assert_eq!(miss.stage.var(0), 0.0, "no goal, no score");
    }
}

#[cfg(test)]
mod layout {
    use super::templates;

    /// Raised scripts get DISTINCT coordinates, so they cannot stack.
    ///
    /// Regression: the deployed demo rendered Pong's three scripts on top of
    /// one another, because stored nodes carry no layout (correctly — `x`/`y`
    /// are presentation and the ABI excludes them) and the emit assigned
    /// none. Two-sided: every script must have a position AND no two may
    /// share one, which a constant offset would fail.
    #[test]
    fn emitted_scripts_are_laid_out_and_never_overlap() {
        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        assert_eq!(scripts.len(), 3);
        let json = super::emit::to_workspace_json(&scripts);
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        let blocks = v["blocks"]["blocks"].as_array().unwrap();

        let mut seen: Vec<(i64, i64)> = Vec::new();
        for (i, b) in blocks.iter().enumerate() {
            let x = b["x"]
                .as_i64()
                .unwrap_or_else(|| panic!("script {i} has no x"));
            let y = b["y"]
                .as_i64()
                .unwrap_or_else(|| panic!("script {i} has no y"));
            assert!(
                !seen.contains(&(x, y)),
                "script {i} shares position ({x},{y}) with an earlier script"
            );
            seen.push((x, y));
        }
        // …and they are separated enough to be readable, not merely distinct.
        let mut ys: Vec<i64> = seen.iter().map(|(_, y)| *y).collect();
        ys.sort_unstable();
        for w in ys.windows(2) {
            assert!(
                w[1] - w[0] >= 200,
                "scripts {} and {} are too close",
                w[0],
                w[1]
            );
        }
    }

    /// Layout is PRESENTATION: it must not reach the stored bytes.
    ///
    /// The arc's W1 falsifier says moving a block 40 pixels must not touch the
    /// ABI. Assigning coordinates at the membrane is only safe if that still
    /// holds, so this proves the emitted layout casts to the same program.
    #[test]
    fn the_assigned_layout_does_not_change_the_program() {
        use ogar_loco::LaneShape;

        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        let json = super::emit::to_workspace_json(&scripts);
        let reread = super::from_workspace_json(&json).expect("parses");

        for (i, (a, b)) in scripts.iter().zip(reread.iter()).enumerate() {
            let x = templates::cast(LaneShape::Pairs, a).expect("casts");
            let y = templates::cast(LaneShape::Pairs, b).expect("casts");
            for (f, (p, q)) in x.functions.iter().zip(y.functions.iter()).enumerate() {
                assert_eq!(
                    p.as_body_bytes(),
                    q.as_body_bytes(),
                    "script {i} fn {f}: layout leaked into the program"
                );
            }
        }
    }
}

#[cfg(test)]
mod pong_scene {
    use super::templates;
    use blockly_run::{Scene, Stage};
    use ogar_loco::LaneShape;

    /// Pong is a SCENE: ball and paddle both move, together, and the ball's
    /// path actually varies over time.
    ///
    /// Regression for what the deploy showed — "a frozen coordinate system
    /// with a very small frozen paddle". Three separate causes, all asserted
    /// here: the scripts must share one stage (so both sprites exist), they
    /// must interleave (so both move), and the trace must contain DISTINCT
    /// positions (so it is motion, not one repeated frame).
    #[test]
    fn pong_is_a_scene_where_both_sprites_move() {
        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        let progs: Vec<_> = scripts
            .iter()
            .map(|s| templates::cast(LaneShape::Pairs, s).expect("casts"))
            .collect();
        let bodies: Vec<&[ogar_loco::FunctionBody]> =
            progs.iter().map(|p| p.functions.as_slice()).collect();

        // A sweeping pointer: a paddle that tracks a CONSTANT mouse has
        // nothing to track and renders as two positions, which reads as
        // frozen. The input is simulated, and named as such.
        let mut scene = Scene::new(Stage::pong(), bodies).with_mouse_sweep(120.0);
        scene.run(120, 40).expect("the scene runs");

        let t = scene.trace();
        assert!(t.len() > 10, "the trace must have frames: {}", t.len());

        // The BALL moved.
        let ball_xs: Vec<f32> = t.iter().map(|s| s.sprites[0].x).collect();
        assert!(
            ball_xs.iter().any(|x| (x - ball_xs[0]).abs() > 1.0),
            "the ball never moved: {ball_xs:?}"
        );
        // …and it did not merely jump once and stop: several distinct
        // positions, which is what makes it an animation.
        let distinct = {
            let mut v: Vec<i32> = ball_xs.iter().map(|x| *x as i32).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(
            distinct > 5,
            "the ball has only {distinct} distinct positions"
        );

        // The ball is still moving at the END of the run, not just at the
        // start. Regression: clamping a bounce to EXACTLY the edge left the
        // `abs() >= half` test true forever, so the ball flipped direction
        // every frame and stuck to the wall — a run that starts lively and
        // freezes would pass the assertions above.
        let tail = &t[t.len() * 3 / 4..];
        let tail_distinct = {
            let mut v: Vec<i32> = tail.iter().map(|s| s.sprites[0].x as i32).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(
            tail_distinct > tail.len() / 4,
            "the ball stalls late in the run: {tail_distinct} distinct of {} frames",
            tail.len()
        );
        // …and it is CONTAINED: it may overshoot by at most one step, because
        // a trace frame can land between `movesteps` and `ifonedgebounce` —
        // after the step, before the correction. What must not happen is
        // drifting away, so the bound is one step and the run must END inside.
        //
        // Measured rather than assumed: the first version of this assertion
        // demanded `<= half_h` exactly and failed at y = -181.55, which is
        // sampling, not escape.
        const OVERSHOOT: f32 = 8.0;
        for s in t {
            assert!(
                s.sprites[0].y.abs() <= s.half_h + OVERSHOOT,
                "the ball escaped the stage: y = {}",
                s.sprites[0].y
            );
            assert!(
                s.sprites[0].x.abs() <= s.half_w + OVERSHOOT,
                "the ball escaped the stage: x = {}",
                s.sprites[0].x
            );
        }
        let end = &t[t.len() - 1].sprites[0];
        assert!(
            end.y.abs() <= t[0].half_h && end.x.abs() <= t[0].half_w,
            "the run must end inside the stage: ({}, {})",
            end.x,
            end.y
        );

        // The PADDLE moved too, in the SAME scene — this is the half that was
        // impossible before, because the paddle script ran on its own stage.
        assert!(
            scene.stage.sprites.len() >= 2,
            "the scene needs both actors"
        );
        let paddle_ys: Vec<f32> = t.iter().map(|s| s.sprites[1].y).collect();
        let paddle_distinct = {
            let mut v: Vec<i32> = paddle_ys.iter().map(|y| *y as i32).collect();
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        assert!(
            paddle_distinct > 5,
            "the paddle barely moves: {paddle_distinct} distinct positions"
        );
    }

    /// The touch targets are SPRITE NAMES resolved through the project basin:
    /// the stored byte is a dynamic-tail index, the raise gives the name back,
    /// and the static basin — which knows no sprites — refuses the same bytes.
    #[test]
    fn pong_touch_targets_are_project_menu_entries_not_numbers() {
        use blockly_abi::menus;
        let scripts = templates::raise_nodes(templates::PONG_NODES).expect("raises");
        let touch = &scripts[0].next.as_ref().unwrap().statements[0].1; // forever body head
        // Walk: movesteps -> bounce -> if(touching(menu))
        let cond = &touch.next.as_ref().unwrap().next.as_ref().unwrap().inputs[0].1;
        assert_eq!(cond.ty, "sensing_touchingobject");
        let menu = &cond.inputs[0].1;
        assert_eq!(menu.ty, "sensing_touchingobjectmenu");
        assert_eq!(
            menu.field("TOUCHINGOBJECTMENU"),
            Some(&blockly_abi::FieldValue::Code("Paddle".into()))
        );
        // The byte is past the static prefix (which is empty for this menu).
        let m = menus::menu_by_id(14).unwrap();
        let idx = menus::encode_in(templates::project_basin(), m, "Paddle").unwrap();
        assert!(usize::from(idx) > m.options.len());
        assert_eq!(idx, 4, "_mouse_, _edge_, Ball, Paddle");
        // Without the project the same nodes cannot be raised — the name is
        // the project's, not the palette's.
        let (_, nodes) = templates::ALL_NODES[0];
        let strip = |b: &[u8]| -> Vec<ogar_loco::FunctionBody> {
            use ogar_loco::node::NODE_BYTES;
            let n = usize::from(b[0]);
            let take = usize::from(b[1]) * NODE_BYTES;
            b[1 + n..1 + n + take]
                .chunks_exact(NODE_BYTES)
                .map(|c| {
                    blockly_abi::FunctionNode::from_le_bytes(
                        c.try_into().unwrap(),
                        ogar_loco::LaneShape::Pairs,
                    )
                    .body
                })
                .collect()
        };
        let ball = strip(nodes);
        assert!(matches!(
            blockly_abi::raise::raise_program(&ball),
            Err(blockly_abi::raise::RaiseError::UnknownOption(_, 4))
        ));
        assert!(blockly_abi::raise::raise_program_in(&ball, templates::project_basin()).is_ok());
    }

    /// Pong (keyboard edition): the paddle is driven by KEY_OPTION codebook
    /// indices — it climbs while `up arrow` is held, descends under `down
    /// arrow`, and does NOT move when no key is held. The menu shadow blocks
    /// in the stored nodes are what make this program expressible at all.
    #[test]
    fn pong_keys_paddle_follows_the_held_key_and_rests_without_one() {
        let scripts = templates::raise_nodes(templates::PONG_KEYS_NODES).expect("raises");
        let progs: Vec<_> = scripts
            .iter()
            .map(|s| templates::cast(LaneShape::Pairs, s).expect("casts"))
            .collect();
        let bodies = || -> Vec<&[ogar_loco::FunctionBody]> {
            progs.iter().map(|p| p.functions.as_slice()).collect()
        };
        // The paddle script really carries the menus: two keyoptions reporters.
        let key_byte = blockly_abi::scratch::device("sensing_keyoptions")
            .unwrap()
            .0;
        let menus_in_paddle = progs[1]
            .functions
            .iter()
            .flat_map(blockly_abi::raise_calls)
            .filter(|c| c.function.0 == key_byte)
            .count();
        assert_eq!(menus_in_paddle, 2, "up and down keyoptions");

        // Under the sweep the paddle goes BOTH ways.
        let mut scene = Scene::new(Stage::pong(), bodies()).with_key_sweep(20);
        scene.run(120, 40).expect("runs");
        let ys: Vec<f32> = scene.trace().iter().map(|s| s.sprites[1].y).collect();
        // Judged frame to frame, not by the extremes: the sweep is symmetric,
        // so a paddle that climbs for 20 rounds and descends for 20 returns
        // to where it started, and "min < 0" would have called that frozen.
        let rises = ys.windows(2).filter(|w| w[1] > w[0]).count();
        let falls = ys.windows(2).filter(|w| w[1] < w[0]).count();
        assert!(rises > 10, "paddle never climbed: {ys:?}");
        assert!(falls > 10, "paddle never descended: {ys:?}");
        let max = ys.iter().copied().fold(f32::MIN, f32::max);
        assert!(max > 30.0, "paddle barely climbed: max {max}");

        // Held `down arrow` only: monotone descent, never up.
        let down =
            blockly_abi::menus::encode(blockly_abi::menus::menu_by_id(1).unwrap(), "down arrow");
        let mut held = Scene::new(Stage::pong(), bodies());
        held.stage.key = down;
        held.run(30, 40).expect("runs");
        let hy: Vec<f32> = held.trace().iter().map(|s| s.sprites[1].y).collect();
        assert!(
            hy.windows(2).all(|w| w[1] <= w[0]),
            "went up under down arrow: {hy:?}"
        );
        assert!(hy.last().unwrap() < &-10.0);

        // Silence half: no key, the paddle rests at 0 while the ball still moves.
        let mut idle = Scene::new(Stage::pong(), bodies());
        idle.run(60, 40).expect("runs");
        assert!(
            idle.trace().iter().all(|s| s.sprites[1].y == 0.0),
            "paddle moved with no key"
        );
        let bx: Vec<i32> = idle.trace().iter().map(|s| s.sprites[0].x as i32).collect();
        assert!(bx.iter().any(|x| *x != bx[0]), "the ball froze too");
    }
}
