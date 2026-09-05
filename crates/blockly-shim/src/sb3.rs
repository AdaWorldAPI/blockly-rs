//! Scratch 3's saved project format (`project.json`, the file inside an
//! `.sb3` zip) read into this crate's [`BlockRecord`]
//! trees, so an existing Scratch project casts through the same pipeline as
//! the Blockly editor's save.
//!
//! # Provenance
//!
//! The shape below (semver `3.0.0`) was measured directly from three real
//! `project.json` files exported by the Scratch editor — this file's format
//! is public data, and every fact here comes from those samples plus the
//! `sb3` format having been publicly documented for years, not from reading
//! `scratch-vm` / `scratch-gui` source, which is AGPL and was deliberately
//! not consulted for this module.
//!
//! # Input tags, and what each one means
//!
//! An `Input` value is a small tagged array. The first element says which
//! shape follows:
//!
//! | tag | shape | meaning |
//! |---|---|---|
//! | `1` | `[1, X]` | **shadow only** — `X` is a block id, a primitive array, or `null` |
//! | `2` | `[2, id]` | a real block, no shadow underneath (this is how a boolean `CONDITION` or a `SUBSTACK` arrives) |
//! | `3` | `[3, X, Y]` | a real block `X` covering an obscured shadow `Y` — `X` wins, `Y` is ignored |
//!
//! A **primitive array** (Scratch's compact literal form) can stand in for a
//! block id anywhere the table above says "a block id":
//!
//! | tag | meaning | value |
//! |---|---|---|
//! | 4 | number | `[4, "10"]` |
//! | 5 | positive number | `[5, "10"]` |
//! | 6 | positive integer | `[6, "10"]` |
//! | 7 | integer | `[7, "10"]` |
//! | 8 | angle | `[8, "10"]` |
//! | 9 | colour | `[9, "#rrggbb"]` |
//! | 10 | string | `[10, "text"]` |
//! | 11 | broadcast | `[11, name, id]` |
//! | 12 | variable | `[12, name, id]` |
//! | 13 | list | `[13, name, id]` |
//!
//! # What is deliberately NOT done here
//!
//! - **No zip unpacking.** This module reads `project.json` text; extracting
//!   it from the `.sb3` archive is the caller's concern.
//! - **A variable, list, or broadcast reference is a dropdown of NAMES, not
//!   an id reference.** `VARIABLE`, `LIST`, and `BROADCAST_OPTION` are read
//!   as [`FieldValue::Code`] carrying the human name — the FIRST element of
//!   the field array or primitive array — never the second-element id.
//!   Scratch enforces unique variable/list/broadcast names per scope, so the
//!   name is what a menu codebook interns and resolves; the id is dropped
//!   because nothing downstream of this reader needs it. This is why
//!   `data_variable` / `data_listcontents` leaves carry `Code`, not
//!   [`FieldValue::Ref`] — this parser does not produce `Ref` at all.
//! - **`procedures_call` is carried, not interpreted.** Its `mutation` lands
//!   verbatim in [`BlockRecord::extra_state`](blockly_abi::BlockRecord), and
//!   the cast refuses it with `CastError::MutatorUnsupported` for the same
//!   reason it refuses any other mutator payload — per-block-type mutator
//!   schemas are their own wave.

use blockly_abi::{BlockRecord, FieldValue};
use ogar_loco::basin::BasinCodebooks;
use serde_json::Value;

/// Why a `project.json` could not be read into an [`Sb3Project`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sb3Error {
    /// The bytes were not valid JSON.
    Json(String),
    /// The document has no `targets` array — not a Scratch project save.
    NotAProject,
    /// A block object had a shape this reader does not recognise.
    MalformedBlock {
        /// The block id, if one was present at all.
        id: String,
        /// What was wrong with it.
        what: String,
    },
    /// An input, `next`, or `parent` pointed at a block id absent from the
    /// target's own `blocks` map.
    DanglingReference {
        /// The block id the dangling reference was found on.
        from: String,
        /// The missing block id it pointed to.
        to: String,
    },
}

impl core::fmt::Display for Sb3Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Sb3Error::Json(e) => write!(f, "invalid JSON: {e}"),
            Sb3Error::NotAProject => f.write_str("not a Scratch project.json (no `targets`)"),
            Sb3Error::MalformedBlock { id, what } => {
                write!(f, "block `{id}`: {what}")
            }
            Sb3Error::DanglingReference { from, to } => {
                write!(f, "block `{from}` references missing block `{to}`")
            }
        }
    }
}

impl core::error::Error for Sb3Error {}

/// One Scratch target (a sprite, or the stage) after reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Sb3Target {
    /// The target's name (`"Stage"`, `"Sprite1"`, …).
    pub name: String,
    /// Whether this target is the stage rather than a sprite.
    pub is_stage: bool,
    /// Top-level scripts, in the order the `blocks` map yields them. A
    /// shadow block or a loose-reporter array is never a script.
    pub scripts: Vec<BlockRecord>,
    /// Variable names declared on this target.
    pub variables: Vec<String>,
    /// List names declared on this target.
    pub lists: Vec<String>,
    /// Broadcast names declared on this target.
    pub broadcasts: Vec<String>,
    /// Costume names, in project order.
    pub costumes: Vec<String>,
    /// Sound names, in project order.
    pub sounds: Vec<String>,
    /// Number of OBJECT blocks in this target's `blocks` map — shadows
    /// included, loose-reporter arrays excluded. Used for coverage
    /// arithmetic by a caller comparing what was read against what was
    /// present.
    pub block_count: usize,
}

/// A Scratch project after reading `project.json`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sb3Project {
    /// Every target, stage and sprites, in the order the file lists them.
    pub targets: Vec<Sb3Target>,
}

/// Read a Scratch `project.json` document into its targets and scripts.
///
/// # Errors
///
/// [`Sb3Error::Json`] for malformed input, [`Sb3Error::NotAProject`] if the
/// document has no `targets`, and the per-block / per-reference refusals.
pub fn from_project_json(json: &str) -> Result<Sb3Project, Sb3Error> {
    let doc: Value = serde_json::from_str(json).map_err(|e| Sb3Error::Json(e.to_string()))?;
    let targets = doc
        .get("targets")
        .and_then(Value::as_array)
        .ok_or(Sb3Error::NotAProject)?;

    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        out.push(read_target(t)?);
    }
    Ok(Sb3Project { targets: out })
}

/// Read one target object.
fn read_target(t: &Value) -> Result<Sb3Target, Sb3Error> {
    let name = t
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let is_stage = t.get("isStage").and_then(Value::as_bool).unwrap_or(false);

    let variables = named_pairs(t.get("variables"));
    let lists = named_pairs(t.get("lists"));
    let broadcasts = single_strings(t.get("broadcasts"));
    let costumes = names_of(t.get("costumes"));
    let sounds = names_of(t.get("sounds"));

    let Some(blocks_obj) = t.get("blocks").and_then(Value::as_object) else {
        return Ok(Sb3Target {
            name,
            is_stage,
            scripts: Vec::new(),
            variables,
            lists,
            broadcasts,
            costumes,
            sounds,
            block_count: 0,
        });
    };

    let block_count = blocks_obj.values().filter(|v| v.is_object()).count();

    let mut scripts = Vec::new();
    for (id, v) in blocks_obj {
        // Arrays are top-level reporter/variable shorthand sitting loose on
        // the canvas (`[12, name, id, x, y]`), never a script.
        let Some(block) = v.as_object() else {
            continue;
        };
        let top_level = block
            .get("topLevel")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let shadow = block
            .get("shadow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if top_level && !shadow {
            scripts.push(read_sb3_block(id, blocks_obj)?);
        }
    }

    Ok(Sb3Target {
        name,
        is_stage,
        scripts,
        variables,
        lists,
        broadcasts,
        costumes,
        sounds,
        block_count,
    })
}

/// `{id: [name, value]}` -> names, e.g. `variables` and `lists`.
fn named_pairs(v: Option<&Value>) -> Vec<String> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Vec::new();
    };
    obj.values()
        .filter_map(|pair| pair.as_array()?.first()?.as_str().map(str::to_string))
        .collect()
}

/// `{id: name}` -> names, e.g. `broadcasts`.
fn single_strings(v: Option<&Value>) -> Vec<String> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Vec::new();
    };
    obj.values()
        .filter_map(|n| n.as_str().map(str::to_string))
        .collect()
}

/// `[{name: ..}, ..]` -> names, in order, e.g. `costumes` and `sounds`.
fn names_of(v: Option<&Value>) -> Vec<String> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|o| o.get("name")?.as_str().map(str::to_string))
        .collect()
}

/// Read one block by id out of the target's `blocks` map, following its
/// `next` chain.
fn read_sb3_block(
    id: &str,
    blocks: &serde_json::Map<String, Value>,
) -> Result<BlockRecord, Sb3Error> {
    let Some(v) = blocks.get(id) else {
        return Err(Sb3Error::DanglingReference {
            from: id.to_string(),
            to: id.to_string(),
        });
    };
    let Some(obj) = v.as_object() else {
        return Err(Sb3Error::MalformedBlock {
            id: id.to_string(),
            what: "not a block object".to_string(),
        });
    };
    let opcode =
        obj.get("opcode")
            .and_then(Value::as_str)
            .ok_or_else(|| Sb3Error::MalformedBlock {
                id: id.to_string(),
                what: "missing `opcode`".to_string(),
            })?;

    let mut rec = BlockRecord::leaf(opcode, id);

    if let Some(fields) = obj.get("fields").and_then(Value::as_object) {
        for (name, fv) in fields {
            let Some(arr) = fv.as_array() else {
                return Err(Sb3Error::MalformedBlock {
                    id: id.to_string(),
                    what: format!("field `{name}` is not an array"),
                });
            };
            let Some(value) = arr.first() else {
                return Err(Sb3Error::MalformedBlock {
                    id: id.to_string(),
                    what: format!("field `{name}` is empty"),
                });
            };
            let field_id = arr.get(1).and_then(Value::as_str);
            rec = rec.with_field(name.clone(), read_sb3_field(name, value, field_id));
        }
    }

    if let Some(mutation) = obj.get("mutation") {
        rec = rec.with_extra_state(mutation.to_string());
    }

    if let Some(inputs) = obj.get("inputs").and_then(Value::as_object) {
        let stmt_names = statement_names(opcode);
        for (name, input) in inputs {
            let Some(child) = read_sb3_input(id, input, blocks)? else {
                continue;
            };
            if stmt_names.iter().any(|s| s == name) || name.starts_with("SUBSTACK") {
                rec = rec.with_statement(name.clone(), child);
            } else {
                rec = rec.with_input(name.clone(), child);
            }
        }
        if let Some(order) = crate::statement_inputs(opcode) {
            rec.statements
                .sort_by_key(|(n, _)| order.iter().position(|o| o == n).unwrap_or(usize::MAX));
        }
    }

    if let Some(next_id) = obj.get("next").and_then(Value::as_str) {
        rec = rec.with_next(read_sb3_block(next_id, blocks)?);
    }

    Ok(rec)
}

/// The statement-input names of an sb3 opcode, for the SUBSTACK naming
/// convention that has no entry in [`crate::statement_inputs`]'s harvested
/// table. `crate::statement_inputs` already answers for every opcode this
/// crate has harvested (it reads `SCRATCH_BLOCK_DEFS`); this is only a
/// fallback so an un-harvested `SUBSTACK`/`SUBSTACK2`-named input still
/// routes to `statements` rather than `inputs`.
fn statement_names(opcode: &str) -> &'static [&'static str] {
    crate::statement_inputs(opcode).unwrap_or(&[])
}

/// The effective child of one `inputs` entry: `[1, X]`, `[2, id]`, or
/// `[3, X, Y]` (`X` wins; `Y`, the obscured shadow, is ignored). Returns
/// `Ok(None)` for an explicitly empty slot (`[1, null]`).
fn read_sb3_input(
    parent_id: &str,
    input: &Value,
    blocks: &serde_json::Map<String, Value>,
) -> Result<Option<BlockRecord>, Sb3Error> {
    let Some(arr) = input.as_array() else {
        return Err(Sb3Error::MalformedBlock {
            id: parent_id.to_string(),
            what: "an input is not an array".to_string(),
        });
    };
    let Some(tag) = arr.first().and_then(Value::as_i64) else {
        return Err(Sb3Error::MalformedBlock {
            id: parent_id.to_string(),
            what: "an input has no tag".to_string(),
        });
    };
    let x = arr.get(1);
    match tag {
        1 | 2 => resolve_operand(parent_id, x, blocks),
        3 => resolve_operand(parent_id, x, blocks),
        other => Err(Sb3Error::MalformedBlock {
            id: parent_id.to_string(),
            what: format!("unrecognised input tag {other}"),
        }),
    }
}

/// Resolve one operand slot: `null` (empty), a primitive array (a literal),
/// or a block id (string) to look up in `blocks`.
fn resolve_operand(
    parent_id: &str,
    x: Option<&Value>,
    blocks: &serde_json::Map<String, Value>,
) -> Result<Option<BlockRecord>, Sb3Error> {
    let Some(x) = x else {
        return Ok(None);
    };
    if x.is_null() {
        return Ok(None);
    }
    if let Some(arr) = x.as_array() {
        return Ok(Some(primitive_leaf(parent_id, arr)?));
    }
    let Some(block_id) = x.as_str() else {
        return Err(Sb3Error::MalformedBlock {
            id: parent_id.to_string(),
            what: "an operand is neither a block id, an array, nor null".to_string(),
        });
    };
    if !blocks.contains_key(block_id) {
        return Err(Sb3Error::DanglingReference {
            from: parent_id.to_string(),
            to: block_id.to_string(),
        });
    }
    Ok(Some(read_sb3_block(block_id, blocks)?))
}

/// Turn one primitive array (`[tag, ..]`) into the leaf `BlockRecord` it
/// denotes — a literal, a variable/list reference, or a broadcast name.
fn primitive_leaf(parent_id: &str, arr: &[Value]) -> Result<BlockRecord, Sb3Error> {
    let malformed = |what: &str| Sb3Error::MalformedBlock {
        id: parent_id.to_string(),
        what: what.to_string(),
    };
    let tag = arr
        .first()
        .and_then(Value::as_i64)
        .ok_or_else(|| malformed("a primitive has no tag"))?;
    let id = format!("{parent_id}:prim");

    match tag {
        4..=8 => {
            let s = scalar_text(arr.get(1))
                .ok_or_else(|| malformed("a numeric primitive has no value"))?;
            Ok(number_leaf(&id, &s))
        }
        9 => {
            let s = arr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("a colour primitive is not a string"))?;
            Ok(BlockRecord::leaf("colour_picker", id)
                .with_field("COLOUR", FieldValue::Wide(s.to_string())))
        }
        10 => {
            // Measured on real projects: a text primitive's value is
            // usually a string but sb3 also writes `[10, 0]` — a JSON
            // number — so the same scalar reading as the numeric tags.
            let s = scalar_text(arr.get(1))
                .ok_or_else(|| malformed("a text primitive has no scalar value"))?;
            Ok(number_leaf(&id, &s))
        }
        11 => {
            let name = arr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("a broadcast primitive has no name"))?;
            Ok(BlockRecord::leaf("event_broadcast_menu", id)
                .with_field("BROADCAST_OPTION", FieldValue::Code(name.to_string())))
        }
        12 => {
            let name = arr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("a variable primitive has no name"))?;
            Ok(BlockRecord::leaf("data_variable", id)
                .with_field("VARIABLE", FieldValue::Code(name.to_string())))
        }
        13 => {
            let name = arr
                .get(1)
                .and_then(Value::as_str)
                .ok_or_else(|| malformed("a list primitive has no name"))?;
            Ok(BlockRecord::leaf("data_listcontents", id)
                .with_field("LIST", FieldValue::Code(name.to_string())))
        }
        other => Err(malformed(&format!("unrecognised primitive tag {other}"))),
    }
}

/// A numeric-string literal, or a `text` leaf if it does not parse as `f64`.
fn number_leaf(id: &str, s: &str) -> BlockRecord {
    if s.parse::<f64>().is_ok() {
        BlockRecord::leaf("math_number", id).with_field("NUM", crate::byte_or_wide(s.to_string()))
    } else {
        BlockRecord::leaf("text", id).with_field("TEXT", crate::byte_or_wide(s.to_string()))
    }
}

/// A field-array's own value slot may be a JSON number (`[10]`) or a string
/// (`["10"]`) — sb3 writes numeric primitives either way.
fn scalar_text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(n) = v.as_f64() {
        return Some(n.to_string());
    }
    v.as_str().map(str::to_string)
}

/// Read one `fields` entry's value into a [`FieldValue`].
///
/// A variable/list/broadcast field is a **dropdown of names**: Scratch
/// enforces unique names per scope, so the name — the FIRST element of the
/// field array — is what a menu codebook interns, and the `id` parameter
/// (the array's second element, when present) is deliberately unused here.
fn read_sb3_field(name: &str, value: &Value, _id: Option<&str>) -> FieldValue {
    let is_name_menu = matches!(name, "VARIABLE" | "LIST" | "BROADCAST_OPTION");
    if is_name_menu {
        let text = value
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| value.as_f64().map(|n| n.to_string()).unwrap_or_default());
        return FieldValue::Code(text);
    }
    if let Some(n) = value.as_f64() {
        return crate::byte_or_wide(n.to_string());
    }
    if let Some(s) = value.as_str() {
        return crate::byte_or_wide(s.to_string());
    }
    if let Some(b) = value.as_bool() {
        return FieldValue::Code(if b { "TRUE" } else { "FALSE" }.to_string());
    }
    crate::byte_or_wide(value.to_string())
}

/// The project basin as seen from one target: every static menu prefix
/// (via [`blockly_abi::menus::builder`]) plus this project's own names —
/// sprites, costumes, sounds, broadcasts — interned into the dynamic menus,
/// exactly as [`crate::templates::project_basin`] does for the built-in
/// demo, but built from a real project's own target list.
///
/// A menu that overflows its 256-entry pool (`PoolError::Full`) simply stops
/// interning further names for that menu — the names already interned stay
/// resolvable, and the rest are unavailable in this basin.
#[must_use]
pub fn target_basin(project: &Sb3Project, target: &Sb3Target) -> BasinCodebooks {
    use blockly_abi::menus;

    const UTF8: u32 = ogar_loco::pool::placeholder::CONST_UTF8_INLINE;
    const DIGEST: u32 = menus::PLACEHOLDER_DIGEST_CLASSID;

    let sprites: Vec<&Sb3Target> = project.targets.iter().filter(|t| !t.is_stage).collect();
    let stage = project.targets.iter().find(|t| t.is_stage);

    let mut basin = BasinCodebooks::new();
    for m in menus::SCRATCH_MENUS {
        let Ok(mut b) = menus::builder(m, UTF8, DIGEST) else {
            continue;
        };
        let names: Vec<String> = match m.name {
            "TOUCHING_OBJECT" => sprite_names(&sprites, target, &["_mouse_", "_edge_"]),
            "DISTANCE_TO" | "POINT_TOWARDS" | "GOTO" | "GLIDE_TO" => {
                sprite_names(&sprites, target, &["_mouse_", "_random_"])
            }
            "OF_OBJECT" => {
                let mut v = vec!["_stage_".to_string()];
                v.extend(sprites.iter().map(|s| s.name.clone()));
                v
            }
            "COSTUME" => target.costumes.clone(),
            "BACKDROP" => {
                let mut v = stage.map(|s| s.costumes.clone()).unwrap_or_default();
                v.push("next backdrop".to_string());
                v.push("previous backdrop".to_string());
                v.push("random backdrop".to_string());
                v
            }
            "SOUND" => target.sounds.clone(),
            // These menus do not exist in `SCRATCH_MENUS` yet — matched here
            // so this arm activates the moment they land, with no further
            // change needed. Stage (globals) first, then this target's own.
            "VARIABLE" => {
                let mut v = stage.map(|s| s.variables.clone()).unwrap_or_default();
                v.extend(target.variables.iter().cloned());
                v
            }
            "LIST" => {
                let mut v = stage.map(|s| s.lists.clone()).unwrap_or_default();
                v.extend(target.lists.iter().cloned());
                v
            }
            "BROADCAST" => {
                let mut v: Vec<String> = Vec::new();
                if let Some(s) = stage {
                    v.extend(s.broadcasts.iter().cloned());
                }
                for t in &project.targets {
                    if !t.is_stage {
                        for name in &t.broadcasts {
                            if !v.contains(name) {
                                v.push(name.clone());
                            }
                        }
                    }
                }
                v
            }
            "CLONE_OF" => sprite_names(&sprites, target, &["_myself_"]),
            _ => Vec::new(),
        };
        for name in &names {
            let wide = name.len() > ogar_loco::pool::CONSTANT_BYTES;
            let interned = if wide {
                b.intern(DIGEST, &menus::digest(name).to_le_bytes())
            } else {
                b.intern(UTF8, name.as_bytes())
            };
            if interned.is_err() {
                // `PoolError::Full`: stop interning further names for this
                // menu, keeping what is already sealed.
                break;
            }
        }
        let _ = basin.plug(b.seal());
    }
    basin
}

/// The fixed pointer/random/mouse names, then every sprite except `target`
/// itself, in project order.
fn sprite_names(sprites: &[&Sb3Target], target: &Sb3Target, fixed: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = fixed.iter().map(|s| (*s).to_string()).collect();
    v.extend(
        sprites
            .iter()
            .filter(|s| s.name != target.name)
            .map(|s| s.name.clone()),
    );
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockly_abi::lower_program_in;
    use ogar_loco::LaneShape;

    /// A hand-built `project.json` with one stage and one sprite. The
    /// sprite has TWO top-level scripts:
    ///
    /// - `event_whenflagclicked -> control_forever { motion_movesteps(10)
    ///   -> control_if(sensing_touchingobject(shadow menu "_mouse_"))
    ///   { motion_turnright(speed | shadow 15) } }` — exercises menu
    ///   shadows, primitive numbers, and a variable primitive obscuring a
    ///   shadow (`[3, var, shadow]`).
    /// - `data_setvariableto(VARIABLE speed, VALUE shadow "3")` —
    ///   variable-free operand handling for the field itself, plus a text
    ///   primitive that parses as a number.
    ///
    /// Plus: one `procedures_call` with a mutation (top-level, its own
    /// script), one loose-reporter ARRAY entry (must not become a script),
    /// and one top-level `shadow: true` block (must not become a script
    /// either).
    const PROJECT_JSON: &str = r#"{
      "targets": [
        {
          "isStage": true, "name": "Stage",
          "variables": {}, "lists": {}, "broadcasts": {"bc1": "go"},
          "blocks": {}, "costumes": [{"name": "backdrop1"}], "sounds": []
        },
        {
          "isStage": false, "name": "Sprite1",
          "variables": {"varid": ["speed", 0]},
          "lists": {"listid": ["scores", []]},
          "broadcasts": {},
          "costumes": [{"name": "costume1"}, {"name": "costume2"}],
          "sounds": [{"name": "pop"}],
          "blocks": {
            "hat": {
              "opcode": "event_whenflagclicked", "next": "forever", "parent": null,
              "topLevel": true, "inputs": {}, "fields": {}
            },
            "forever": {
              "opcode": "control_forever", "next": null, "parent": "hat",
              "topLevel": false, "fields": {},
              "inputs": {"SUBSTACK": [2, "move"]}
            },
            "move": {
              "opcode": "motion_movesteps", "next": "ifblock", "parent": "forever",
              "topLevel": false, "fields": {},
              "inputs": {"STEPS": [1, [4, "10"]]}
            },
            "ifblock": {
              "opcode": "control_if", "next": null, "parent": "move",
              "topLevel": false, "fields": {},
              "inputs": {
                "CONDITION": [2, "touching"],
                "SUBSTACK": [2, "turn"]
              }
            },
            "touching": {
              "opcode": "sensing_touchingobject", "next": null, "parent": "ifblock",
              "topLevel": false, "fields": {},
              "inputs": {"TOUCHINGOBJECTMENU": [1, "menu"]}
            },
            "menu": {
              "opcode": "sensing_touchingobjectmenu", "next": null, "parent": "touching",
              "topLevel": false, "shadow": true,
              "fields": {"TOUCHINGOBJECTMENU": ["_mouse_", null]}, "inputs": {}
            },
            "turn": {
              "opcode": "motion_turnright", "next": null, "parent": "ifblock",
              "topLevel": false, "fields": {},
              "inputs": {"DEGREES": [3, [12, "speed", "varid"], [4, "15"]]}
            },
            "setvar": {
              "opcode": "data_setvariableto", "next": null, "parent": null,
              "topLevel": true, "x": 5, "y": 5,
              "fields": {"VARIABLE": ["speed", "varid"]},
              "inputs": {"VALUE": [1, [10, "3"]]}
            },
            "callproc": {
              "opcode": "procedures_call", "next": null, "parent": null,
              "topLevel": true, "x": 5, "y": 400,
              "fields": {}, "inputs": {},
              "mutation": {
                "tagName": "mutation", "children": [], "proccode": "go %s",
                "argumentids": "[\"a\"]", "warp": "false"
              }
            },
            "loose_reporter": [12, "speed", "varid", 500, 500],
            "shadow_top": {
              "opcode": "math_number", "next": null, "parent": null,
              "topLevel": true, "shadow": true, "x": 900, "y": 900,
              "fields": {"NUM": 42}, "inputs": {}
            }
          }
        }
      ],
      "monitors": [], "extensions": [], "meta": {"semver": "3.0.0"}
    }"#;

    fn project() -> Sb3Project {
        from_project_json(PROJECT_JSON).expect("the fixture parses")
    }

    fn sprite(p: &Sb3Project) -> &Sb3Target {
        p.targets.iter().find(|t| !t.is_stage).unwrap()
    }

    #[test]
    fn only_real_top_level_object_blocks_become_scripts() {
        let p = project();
        let sp = sprite(&p);
        // hat->forever->... , setvar, callproc — NOT loose_reporter (an
        // array), NOT shadow_top (a shadow block).
        assert_eq!(sp.scripts.len(), 3, "{:#?}", sp.scripts);
        let tys: Vec<&str> = sp.scripts.iter().map(|s| s.ty.as_str()).collect();
        assert!(tys.contains(&"event_whenflagclicked"));
        assert!(tys.contains(&"data_setvariableto"));
        assert!(tys.contains(&"procedures_call"));
        assert!(
            !tys.contains(&"math_number"),
            "the shadow must not be a script"
        );
    }

    #[test]
    fn the_tree_shape_matches_the_chain_and_the_statement_split() {
        let p = project();
        let sp = sprite(&p);
        let hat = sp
            .scripts
            .iter()
            .find(|s| s.ty == "event_whenflagclicked")
            .unwrap();
        let forever = hat.next.as_ref().unwrap();
        assert_eq!(forever.ty, "control_forever");
        assert_eq!(forever.inputs.len(), 0, "SUBSTACK is not an operand");
        assert_eq!(forever.statements.len(), 1);
        assert_eq!(forever.statements[0].0, "SUBSTACK");

        let mv = &forever.statements[0].1;
        assert_eq!(mv.ty, "motion_movesteps");
        assert_eq!(mv.inputs[0].0, "STEPS");
        assert_eq!(mv.inputs[0].1.ty, "math_number");
        assert_eq!(
            mv.inputs[0].1.field("NUM"),
            Some(&FieldValue::Byte(10)),
            "[1,[4,\"10\"]] must become a math_number literal"
        );

        let iff = mv.next.as_ref().unwrap();
        assert_eq!(iff.ty, "control_if");
        assert_eq!(iff.inputs.len(), 1, "CONDITION is an operand");
        assert_eq!(iff.inputs[0].0, "CONDITION");
        assert_eq!(iff.inputs[0].1.ty, "sensing_touchingobject");
        assert_eq!(iff.statements.len(), 1, "SUBSTACK is a body");
        assert_eq!(iff.statements[0].0, "SUBSTACK");
        assert_eq!(iff.statements[0].1.ty, "motion_turnright");
    }

    #[test]
    fn a_shadow_menu_resolves_to_the_menu_leaf_with_a_code() {
        let p = project();
        let sp = sprite(&p);
        let hat = sp
            .scripts
            .iter()
            .find(|s| s.ty == "event_whenflagclicked")
            .unwrap();
        let touching = &hat.next.as_ref().unwrap().statements[0]
            .1
            .next
            .as_ref()
            .unwrap()
            .inputs[0]
            .1;
        assert_eq!(touching.ty, "sensing_touchingobject");
        let menu = &touching.inputs[0].1;
        assert_eq!(menu.ty, "sensing_touchingobjectmenu");
        // `_mouse_` is not all-uppercase, so the general `byte_or_wide`
        // classifier reads it as `Wide`, not `Code` — and that is fine for
        // casting: `blockly_abi`'s field resolution treats `Code` and `Wide`
        // identically for any field the menu table names
        // (`FieldValue::Code(c) | FieldValue::Wide(c) => c.clone()`), so a
        // menu-shadow field resolves through the codebook regardless of
        // which of the two this reader happened to classify it as.
        assert_eq!(
            menu.field("TOUCHINGOBJECTMENU"),
            Some(&FieldValue::Wide("_mouse_".to_string()))
        );
    }

    #[test]
    fn a_real_block_obscuring_a_shadow_picks_the_real_block() {
        let p = project();
        let sp = sprite(&p);
        let hat = sp
            .scripts
            .iter()
            .find(|s| s.ty == "event_whenflagclicked")
            .unwrap();
        let turn = &hat.next.as_ref().unwrap().statements[0]
            .1
            .next
            .as_ref()
            .unwrap()
            .statements[0]
            .1;
        assert_eq!(turn.ty, "motion_turnright");
        let degrees = &turn.inputs[0].1;
        // [3, [12,"speed","varid"], [4,"15"]] — the variable wins, not 15.
        // The NAME, not the id: variable/list/broadcast fields are a
        // dropdown of names.
        assert_eq!(degrees.ty, "data_variable");
        assert_eq!(
            degrees.field("VARIABLE"),
            Some(&FieldValue::Code("speed".to_string()))
        );
    }

    #[test]
    fn a_variable_field_becomes_a_name_code_not_a_wide_literal() {
        let p = project();
        let sp = sprite(&p);
        let setvar = sp
            .scripts
            .iter()
            .find(|s| s.ty == "data_setvariableto")
            .unwrap();
        assert_eq!(
            setvar.field("VARIABLE"),
            Some(&FieldValue::Code("speed".to_string()))
        );
        // VALUE is [1,[10,"3"]] — a text primitive that parses as a number,
        // so it becomes a math_number, not a `text` leaf.
        assert_eq!(setvar.inputs[0].0, "VALUE");
        assert_eq!(setvar.inputs[0].1.ty, "math_number");
        assert_eq!(setvar.inputs[0].1.field("NUM"), Some(&FieldValue::Byte(3)));
    }

    #[test]
    fn a_mutation_is_carried_as_extra_state() {
        let p = project();
        let sp = sprite(&p);
        let call = sp
            .scripts
            .iter()
            .find(|s| s.ty == "procedures_call")
            .unwrap();
        assert!(call.extra_state.is_some());
    }

    #[test]
    fn counts_and_lists_are_read() {
        let p = project();
        let sp = sprite(&p);
        // hat, forever, move, ifblock, touching, menu, turn, setvar,
        // callproc, shadow_top — 10 OBJECT blocks (shadows counted too, per
        // spec); loose_reporter is an array, excluded.
        assert_eq!(sp.block_count, 10);
        assert_eq!(sp.variables, vec!["speed".to_string()]);
        assert_eq!(sp.lists, vec!["scores".to_string()]);
        assert_eq!(
            sp.costumes,
            vec!["costume1".to_string(), "costume2".to_string()]
        );
        assert_eq!(sp.sounds, vec!["pop".to_string()]);
        let stage = p.targets.iter().find(|t| t.is_stage).unwrap();
        assert_eq!(stage.broadcasts, vec!["go".to_string()]);
    }

    #[test]
    fn scripts_cast_and_a_variable_read_carries_its_codebook_index() {
        let p = project();
        let sp = sprite(&p);
        let basin = target_basin(&p, sp);

        let hat = sp
            .scripts
            .iter()
            .find(|s| s.ty == "event_whenflagclicked")
            .unwrap();
        // Contains a `data_variable` (via the obscured-shadow pick). It casts
        // because `VARIABLE` is menu 25 and `target_basin` interned the
        // project's variable names into it — the variable's IDENTITY is in
        // the call's bytes as the codebook index, asserted below, not merely
        // "a variable read happened".
        let prog = lower_program_in(LaneShape::Pairs, hat, &basin).expect("names are interned");
        let var_menu = blockly_abi::menus::SCRATCH_MENUS
            .iter()
            .find(|m| m.name == "VARIABLE")
            .unwrap();
        let speed = blockly_abi::menus::encode_in(&basin, var_menu, "speed").expect("interned");
        let var_get = ogar_loco::FnIndex::VAR_GET;
        let carried: Vec<u8> = prog
            .functions
            .iter()
            .flat_map(blockly_abi::raise_calls)
            .filter(|c| c.function == var_get)
            .map(|c| c.values[0])
            .collect();
        assert_eq!(carried, vec![speed], "the read names WHICH variable");
        assert_ne!(speed, 0, "not the zero-fallback slot");
        // Silence half: without the project's names the same script is
        // refused rather than written to slot 0.
        assert!(blockly_abi::lower_program(LaneShape::Pairs, hat).is_err());

        // setvar is ALSO a variable reference (its own VARIABLE field), so
        // instead assert on a variable-free script assembled from the same
        // fixture's parts: hat -> forever -> move -> if(touching _mouse_)
        // -> show.
        let variable_free = BlockRecord::leaf("event_whenflagclicked", "vf-hat").with_next(
            BlockRecord::leaf("control_forever", "vf-forever").with_statement(
                "SUBSTACK",
                BlockRecord::leaf("motion_movesteps", "vf-move")
                    .with_input(
                        "STEPS",
                        BlockRecord::leaf("math_number", "vf-move:n")
                            .with_field("NUM", FieldValue::Byte(10)),
                    )
                    .with_next(
                        BlockRecord::leaf("control_if", "vf-if")
                            .with_input(
                                "CONDITION",
                                BlockRecord::leaf("sensing_touchingobject", "vf-touch").with_input(
                                    "TOUCHINGOBJECTMENU",
                                    BlockRecord::leaf("sensing_touchingobjectmenu", "vf-menu")
                                        .with_field(
                                            "TOUCHINGOBJECTMENU",
                                            FieldValue::Code("_mouse_".to_string()),
                                        ),
                                ),
                            )
                            .with_statement("SUBSTACK", BlockRecord::leaf("looks_show", "vf-show")),
                    ),
            ),
        );
        assert!(lower_program_in(LaneShape::Pairs, &variable_free, &basin).is_ok());
    }

    #[test]
    fn target_basin_resolves_sprites_excluding_self() {
        use blockly_abi::menus;
        let p = project();
        let sprites: Vec<&Sb3Target> = p.targets.iter().filter(|t| !t.is_stage).collect();
        assert_eq!(sprites.len(), 1, "the fixture only has one sprite so far");

        // Extend the fixture in-memory with a second sprite to exercise
        // self-exclusion and cross-target resolution, rather than growing
        // the giant JSON further.
        let mut p2 = p.clone();
        let mut paddle = sprite(&p2).clone();
        paddle.name = "Paddle".to_string();
        paddle.scripts.clear();
        let mut ball = sprite(&p2).clone();
        ball.name = "Ball".to_string();
        ball.scripts.clear();
        p2.targets.retain(|t| t.is_stage);
        p2.targets.push(ball.clone());
        p2.targets.push(paddle.clone());

        let ball_basin = target_basin(&p2, &ball);
        let m = menus::menu_by_id(14).unwrap(); // TOUCHING_OBJECT
        // _mouse_=1, _edge_=2, Paddle=3 (Ball excluded, it is self).
        assert_eq!(menus::encode_in(&ball_basin, m, "Paddle"), Some(3));
        assert_eq!(
            menus::encode_in(&ball_basin, m, "Ball"),
            None,
            "a sprite must not see itself in its own basin"
        );

        let paddle_basin = target_basin(&p2, &paddle);
        assert_eq!(menus::encode_in(&paddle_basin, m, "Ball"), Some(3));

        // Anti-vacuity: a name nobody interned resolves to nothing.
        assert_eq!(menus::encode_in(&ball_basin, m, "Nobody"), None);

        // A wide (> 12 byte) sprite name is interned as a digest entry.
        let mut wide = ball.clone();
        wide.name = "AVeryLongSpriteNameThatDoesNotFit".to_string();
        assert!(wide.name.len() > 12);
        let mut p3 = p2.clone();
        p3.targets.retain(|t| t.is_stage || t.name == "Paddle");
        p3.targets.push(wide.clone());
        let wide_basin = target_basin(&p3, &paddle);
        let idx = menus::encode_in(&wide_basin, m, &wide.name).expect("wide name resolves");
        let book = wide_basin.get(m.id).unwrap();
        let entry = book.resolve(idx).unwrap();
        assert_eq!(entry.classid, menus::PLACEHOLDER_DIGEST_CLASSID);
    }

    #[test]
    fn malformed_and_missing_input_is_refused_not_half_read() {
        assert!(matches!(from_project_json("{"), Err(Sb3Error::Json(_))));
        assert_eq!(
            from_project_json(r#"{"monitors":[]}"#),
            Err(Sb3Error::NotAProject)
        );
        let dangling = r#"{"targets":[{"isStage":false,"name":"S","blocks":{
            "a":{"opcode":"motion_movesteps","topLevel":true,"next":null,
                 "fields":{},"inputs":{"STEPS":[2,"missing"]}}
        }}]}"#;
        assert!(matches!(
            from_project_json(dangling),
            Err(Sb3Error::DanglingReference { .. })
        ));
        // Two-sided: a well-formed document still reads.
        assert!(from_project_json(PROJECT_JSON).is_ok());
    }
}
