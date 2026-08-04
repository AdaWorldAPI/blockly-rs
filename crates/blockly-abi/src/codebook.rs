//! The Blockly-type → [`FnIndex`] codebook.
//!
//! Harvested from the Apache-2.0 Blockly block definitions
//! (`packages/blockly/blocks/*.ts`) against the `ogar-blockly` palette:
//! **101 rows over 65 block types** — 96 exact (the palette's own doc comment
//! names the block), 2 inferred, 3 with **no mapping at all**.
//!
//! # Two kinds of dropdown, and the difference matters
//!
//! A dropdown field is not always part of the opcode:
//!
//! - **[`CodeRole::Selector`]** — the code chooses WHICH function.
//!   `logic_compare` + `LT` is [`FnIndex::LT`]; the same block with `GT` is a
//!   different function. The code is consumed by resolution and never becomes
//!   an immediate.
//! - **[`CodeRole::ValueParam`]** — the code is an ARGUMENT.
//!   `math_constant` is always [`FnIndex::CONSTANT`]; whether it yields π or e
//!   is a value. `text_charAt`'s `WHERE` likewise.
//!
//! Collapsing the two would either explode the palette (a slot per constant)
//! or silently drop the parameter. Both are recorded, and a `ValueParam` whose
//! byte encoding is not yet defined is **refused** rather than guessed.
//!
//! # The three gaps — deliberately unmapped
//!
//! These Blockly blocks have no palette entry. They are NOT minted here:
//! codebook ids are permanent, and a mint is an operator decision with a
//! ledger entry (OGAR `docs/BLOCK-EDITOR-PLAN.md` § "Standing rules"). Casting
//! one yields [`CastError::UnknownOpcode`](crate::CastError::UnknownOpcode).
//!
//! | block | what it does | why nothing covers it |
//! |---|---|---|
//! | `math_on_list[RANDOM]` | random element of a list | `ON_LIST`'s doc enumerates sum/min/max/average/median/mode/std_dev — not random |
//! | `lists_reverse` | reverse a list | `REVERSE` (`0x6F`) is scoped to `text_reverse`; the `0x70..0x7F` list family has no reverse |
//! | `lists_getIndex[GET_REMOVE]` | read AND delete in one op | compound; `LIST_GET` reads, `LIST_DELETE` deletes, and giving `LIST_GET` a silent side effect would misrepresent the block |
//!
//! # Palette entries no Blockly block reaches
//!
//! `FOREVER`, `WAIT`, `WAIT_UNTIL`, `STOP` are Scratch-only by design.
//! `CONTAINS` and `LIST_CONTAINS` have doc comments citing `text_contains` /
//! `lists_contains` — **neither block type exists in Blockly**; the real
//! sources are Scratch's `operator_contains` / `data_listcontainsitem`, so the
//! `-shaped` phrasing in those doc comments is hedging, not a Blockly citation.
//! `PROC_ARG` has no dedicated block: argument reads go through an ordinary
//! `variables_get` bound to the argument's variable model.

use ogar_blockly::FnIndex;

/// What a block's dropdown code does during resolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeRole {
    /// The block has no dropdown that participates in resolution.
    None,
    /// The code chooses WHICH function — consumed by resolution.
    Selector,
    /// The code is an ARGUMENT to a fixed function — must become a value.
    ValueParam,
}

/// One resolved block type: its function, and what its dropdown code meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpcodeMapping {
    /// The function this block invokes.
    pub function: FnIndex,
    /// Whether the dropdown code selected the function or is an argument.
    pub role: CodeRole,
}

/// Resolve a Blockly block type (with its dropdown code, if any) to a mapping.
///
/// Returns `None` for the three deliberate gaps and for any unknown type —
/// never a guess.
#[must_use]
pub fn resolve(ty: &str, code: Option<&str>) -> Option<OpcodeMapping> {
    use CodeRole::{None as NoCode, Selector, ValueParam};
    let m = |function, role| Some(OpcodeMapping { function, role });

    match (ty, code) {
        // ── logic ───────────────────────────────────────────────────────
        ("logic_boolean", Some("TRUE")) => m(FnIndex::TRUE, Selector),
        ("logic_boolean", Some("FALSE")) => m(FnIndex::FALSE, Selector),
        ("controls_if", _) => m(FnIndex::IF, NoCode),
        ("controls_ifelse", _) => m(FnIndex::IF_ELSE, NoCode),
        ("logic_compare", Some("EQ")) => m(FnIndex::EQ, Selector),
        ("logic_compare", Some("NEQ")) => m(FnIndex::NEQ, Selector),
        ("logic_compare", Some("LT")) => m(FnIndex::LT, Selector),
        ("logic_compare", Some("LTE")) => m(FnIndex::LTE, Selector),
        ("logic_compare", Some("GT")) => m(FnIndex::GT, Selector),
        ("logic_compare", Some("GTE")) => m(FnIndex::GTE, Selector),
        ("logic_operation", Some("AND")) => m(FnIndex::AND, Selector),
        ("logic_operation", Some("OR")) => m(FnIndex::OR, Selector),
        ("logic_negate", _) => m(FnIndex::NOT, NoCode),
        ("logic_null", _) => m(FnIndex::NULL, NoCode),
        ("logic_ternary", _) => m(FnIndex::TERNARY, NoCode),

        // ── loops ───────────────────────────────────────────────────────
        ("controls_repeat" | "controls_repeat_ext", _) => m(FnIndex::REPEAT, NoCode),
        ("controls_whileUntil", Some("WHILE")) => m(FnIndex::WHILE, Selector),
        ("controls_whileUntil", Some("UNTIL")) => m(FnIndex::REPEAT_UNTIL, Selector),
        ("controls_for", _) => m(FnIndex::FOR_RANGE, NoCode),
        ("controls_forEach", _) => m(FnIndex::FOR_EACH, NoCode),
        ("controls_flow_statements", Some("BREAK")) => m(FnIndex::BREAK, Selector),
        ("controls_flow_statements", Some("CONTINUE")) => m(FnIndex::CONTINUE, Selector),

        // ── math ────────────────────────────────────────────────────────
        ("math_number", _) => m(FnIndex::NUMBER, NoCode),
        ("math_arithmetic", Some("ADD")) => m(FnIndex::ADD, Selector),
        ("math_arithmetic", Some("MINUS")) => m(FnIndex::SUB, Selector),
        ("math_arithmetic", Some("MULTIPLY")) => m(FnIndex::MUL, Selector),
        ("math_arithmetic", Some("DIVIDE")) => m(FnIndex::DIV, Selector),
        ("math_arithmetic", Some("POWER")) => m(FnIndex::POW, Selector),
        ("math_single", Some("ROOT")) => m(FnIndex::SQRT, Selector),
        ("math_single", Some("ABS")) => m(FnIndex::ABS, Selector),
        ("math_single", Some("NEG")) => m(FnIndex::NEG, Selector),
        ("math_single", Some("LN")) => m(FnIndex::LN, Selector),
        ("math_single", Some("LOG10")) => m(FnIndex::LOG10, Selector),
        ("math_single", Some("EXP")) => m(FnIndex::EXP_E, Selector),
        ("math_single", Some("POW10")) => m(FnIndex::EXP_10, Selector),
        ("math_trig", Some("SIN")) => m(FnIndex::SIN, Selector),
        ("math_trig", Some("COS")) => m(FnIndex::COS, Selector),
        ("math_trig", Some("TAN")) => m(FnIndex::TAN, Selector),
        ("math_trig", Some("ASIN")) => m(FnIndex::ASIN, Selector),
        ("math_trig", Some("ACOS")) => m(FnIndex::ACOS, Selector),
        ("math_trig", Some("ATAN")) => m(FnIndex::ATAN, Selector),
        ("math_constant", _) => m(FnIndex::CONSTANT, ValueParam),
        ("math_number_property", _) => m(FnIndex::NUMBER_PROPERTY, ValueParam),
        ("math_change", _) => m(FnIndex::VAR_CHANGE, NoCode),
        ("math_round", Some("ROUND")) => m(FnIndex::ROUND, Selector),
        ("math_round", Some("ROUNDUP")) => m(FnIndex::CEIL, Selector),
        ("math_round", Some("ROUNDDOWN")) => m(FnIndex::FLOOR, Selector),
        // GAP: math_on_list[RANDOM] — ON_LIST does not enumerate `random`.
        ("math_on_list", Some("RANDOM")) => Option::None,
        ("math_on_list", _) => m(FnIndex::ON_LIST, ValueParam),
        ("math_modulo", _) => m(FnIndex::MOD, NoCode),
        ("math_constrain", _) => m(FnIndex::CONSTRAIN, NoCode),
        ("math_random_int", _) => m(FnIndex::RANDOM_INT, NoCode),
        ("math_random_float", _) => m(FnIndex::RANDOM_FLOAT, NoCode),
        ("math_atan2", _) => m(FnIndex::ATAN2, NoCode),

        // ── text ────────────────────────────────────────────────────────
        ("text", _) => m(FnIndex::TEXT, NoCode),
        ("text_join", _) => m(FnIndex::JOIN, NoCode),
        ("text_append", _) => m(FnIndex::APPEND, NoCode),
        ("text_length", _) => m(FnIndex::LENGTH, NoCode),
        ("text_isEmpty", _) => m(FnIndex::IS_EMPTY, NoCode),
        ("text_indexOf", _) => m(FnIndex::INDEX_OF, ValueParam),
        ("text_charAt", _) => m(FnIndex::CHAR_AT, ValueParam),
        ("text_getSubstring", _) => m(FnIndex::SUBSTRING, ValueParam),
        ("text_changeCase", _) => m(FnIndex::CHANGE_CASE, ValueParam),
        ("text_trim", _) => m(FnIndex::TRIM, ValueParam),
        ("text_print", _) => m(FnIndex::PRINT, NoCode),
        ("text_prompt" | "text_prompt_ext", _) => m(FnIndex::PROMPT, ValueParam),
        ("text_count", _) => m(FnIndex::COUNT, NoCode),
        ("text_replace", _) => m(FnIndex::REPLACE, NoCode),
        ("text_reverse", _) => m(FnIndex::REVERSE, NoCode),

        // ── lists ───────────────────────────────────────────────────────
        ("lists_create_empty", _) => m(FnIndex::LIST_EMPTY, NoCode),
        ("lists_create_with", _) => m(FnIndex::LIST_WITH, NoCode),
        ("lists_repeat", _) => m(FnIndex::LIST_REPEAT, NoCode),
        ("lists_length", _) => m(FnIndex::LIST_LENGTH, NoCode),
        ("lists_isEmpty", _) => m(FnIndex::LIST_IS_EMPTY, NoCode),
        ("lists_indexOf", _) => m(FnIndex::LIST_INDEX_OF, ValueParam),
        ("lists_getIndex", Some("GET")) => m(FnIndex::LIST_GET, Selector),
        // GAP: GET_REMOVE is compound (read AND delete); neither half covers it.
        ("lists_getIndex", Some("GET_REMOVE")) => Option::None,
        ("lists_getIndex", Some("REMOVE")) => m(FnIndex::LIST_DELETE, Selector),
        ("lists_setIndex", Some("SET")) => m(FnIndex::LIST_SET, Selector),
        ("lists_setIndex", Some("INSERT")) => m(FnIndex::LIST_INSERT, Selector),
        ("lists_getSublist", _) => m(FnIndex::LIST_SUBLIST, ValueParam),
        ("lists_sort", _) => m(FnIndex::LIST_SORT, ValueParam),
        ("lists_split", _) => m(FnIndex::LIST_SPLIT, ValueParam),
        // GAP: lists_reverse — REVERSE (0x6F) is text-scoped; no list reverse.
        ("lists_reverse", _) => Option::None,

        // ── variables ───────────────────────────────────────────────────
        ("variables_get" | "variables_get_dynamic", _) => m(FnIndex::VAR_GET, NoCode),
        ("variables_set" | "variables_set_dynamic", _) => m(FnIndex::VAR_SET, NoCode),

        // ── procedures ──────────────────────────────────────────────────
        ("procedures_defnoreturn" | "procedures_defreturn", _) => m(FnIndex::PROC_DEF, NoCode),
        ("procedures_callnoreturn" | "procedures_callreturn", _) => m(FnIndex::PROC_CALL, NoCode),
        ("procedures_ifreturn", _) => m(FnIndex::RETURN, NoCode),

        _ => Option::None,
    }
}

/// Resolve to just the function index — the common case.
#[must_use]
pub fn resolve_opcode(ty: &str, code: Option<&str>) -> Option<FnIndex> {
    resolve(ty, code).map(|m| m.function)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dropdown_selector_picks_different_functions() {
        // The whole point of Selector: the SAME block type with different
        // codes must land on DIFFERENT palette slots.
        let lt = resolve("logic_compare", Some("LT")).unwrap();
        let gt = resolve("logic_compare", Some("GT")).unwrap();
        assert_eq!(lt.function, FnIndex::LT);
        assert_eq!(gt.function, FnIndex::GT);
        assert_ne!(lt.function, gt.function);
        assert_eq!(lt.role, CodeRole::Selector);
    }

    #[test]
    fn a_value_param_keeps_one_function_across_codes() {
        // The mirror image, and the anti-vacuity guard for the test above: a
        // ValueParam must NOT fan out. If these ever differ, the two roles
        // have been conflated.
        let pi = resolve("math_constant", Some("PI")).unwrap();
        let e = resolve("math_constant", Some("E")).unwrap();
        assert_eq!(pi.function, e.function);
        assert_eq!(pi.function, FnIndex::CONSTANT);
        assert_eq!(pi.role, CodeRole::ValueParam);
    }

    #[test]
    fn the_two_frontends_converge_on_one_slot() {
        // The arc's whole justification: Blockly's dropdown-carrying
        // logic_compare[LT] lands on the SAME byte Scratch's operator_lt does.
        assert_eq!(
            resolve_opcode("logic_compare", Some("LT")),
            Some(FnIndex::LT)
        );
        // And math_single + math_trig (two Blockly blocks) fan into the slots
        // Scratch's single operator_mathop covers.
        assert_eq!(
            resolve_opcode("math_single", Some("ABS")),
            Some(FnIndex::ABS)
        );
        assert_eq!(resolve_opcode("math_trig", Some("SIN")), Some(FnIndex::SIN));
    }

    #[test]
    fn the_three_gaps_are_refused_not_guessed() {
        // Deliberate: a mint is an operator decision. Each gap must resolve to
        // None while its NEIGHBOURS still resolve — otherwise this test would
        // pass for the trivial reason that the block type is simply unknown.
        assert_eq!(resolve("math_on_list", Some("RANDOM")), None);
        assert!(resolve("math_on_list", Some("SUM")).is_some());

        assert_eq!(resolve("lists_reverse", None), None);
        assert!(resolve("text_reverse", None).is_some());

        assert_eq!(resolve("lists_getIndex", Some("GET_REMOVE")), None);
        assert!(resolve("lists_getIndex", Some("GET")).is_some());
        assert!(resolve("lists_getIndex", Some("REMOVE")).is_some());
    }

    #[test]
    fn an_unknown_type_resolves_to_nothing() {
        assert_eq!(resolve("definitely_not_a_block", None), None);
        assert_eq!(resolve("math_arithmetic", Some("NOT_AN_OP")), None);
    }

    #[test]
    fn the_harvest_census_is_pinned() {
        // 65 block types harvested from the Apache-2.0 definitions. If Blockly
        // upstream adds or renames one, this count is the drift signal — the
        // palette has no producer, so nothing else would notice.
        // (OGAR BLOCK-EDITOR-PLAN.md D3: hand-curation + drift test.)
        let selector_cases = [
            ("logic_boolean", "TRUE"),
            ("logic_boolean", "FALSE"),
            ("logic_compare", "EQ"),
            ("logic_compare", "NEQ"),
            ("logic_compare", "LT"),
            ("logic_compare", "LTE"),
            ("logic_compare", "GT"),
            ("logic_compare", "GTE"),
            ("logic_operation", "AND"),
            ("logic_operation", "OR"),
            ("controls_whileUntil", "WHILE"),
            ("controls_whileUntil", "UNTIL"),
            ("controls_flow_statements", "BREAK"),
            ("controls_flow_statements", "CONTINUE"),
            ("math_arithmetic", "ADD"),
            ("math_arithmetic", "MINUS"),
            ("math_arithmetic", "MULTIPLY"),
            ("math_arithmetic", "DIVIDE"),
            ("math_arithmetic", "POWER"),
            ("math_single", "ROOT"),
            ("math_single", "ABS"),
            ("math_single", "NEG"),
            ("math_single", "LN"),
            ("math_single", "LOG10"),
            ("math_single", "EXP"),
            ("math_single", "POW10"),
            ("math_trig", "SIN"),
            ("math_trig", "COS"),
            ("math_trig", "TAN"),
            ("math_trig", "ASIN"),
            ("math_trig", "ACOS"),
            ("math_trig", "ATAN"),
            ("math_round", "ROUND"),
            ("math_round", "ROUNDUP"),
            ("math_round", "ROUNDDOWN"),
            ("lists_getIndex", "GET"),
            ("lists_getIndex", "REMOVE"),
            ("lists_setIndex", "SET"),
            ("lists_setIndex", "INSERT"),
        ];
        for (ty, code) in selector_cases {
            let mapped =
                resolve(ty, Some(code)).unwrap_or_else(|| panic!("{ty}[{code}] lost its mapping"));
            assert_eq!(mapped.role, CodeRole::Selector, "{ty}[{code}]");
        }
        assert_eq!(selector_cases.len(), 39, "selector census moved");

        let plain = [
            "controls_if",
            "controls_ifelse",
            "logic_negate",
            "logic_null",
            "logic_ternary",
            "controls_repeat",
            "controls_repeat_ext",
            "controls_for",
            "controls_forEach",
            "math_number",
            "math_change",
            "math_modulo",
            "math_constrain",
            "math_random_int",
            "math_random_float",
            "math_atan2",
            "text",
            "text_join",
            "text_append",
            "text_length",
            "text_isEmpty",
            "text_print",
            "text_count",
            "text_replace",
            "text_reverse",
            "lists_create_empty",
            "lists_create_with",
            "lists_repeat",
            "lists_length",
            "lists_isEmpty",
            "variables_get",
            "variables_set",
            "variables_get_dynamic",
            "variables_set_dynamic",
            "procedures_defnoreturn",
            "procedures_defreturn",
            "procedures_callnoreturn",
            "procedures_callreturn",
            "procedures_ifreturn",
        ];
        for ty in plain {
            assert!(resolve(ty, None).is_some(), "{ty} lost its mapping");
        }
        assert_eq!(plain.len(), 39, "plain-block census moved");

        let value_params = [
            "math_constant",
            "math_number_property",
            "math_on_list",
            "text_indexOf",
            "text_charAt",
            "text_getSubstring",
            "text_changeCase",
            "text_trim",
            "text_prompt",
            "text_prompt_ext",
            "lists_indexOf",
            "lists_getSublist",
            "lists_sort",
            "lists_split",
        ];
        for ty in value_params {
            let mapped =
                resolve(ty, Some("ANY")).unwrap_or_else(|| panic!("{ty} lost its mapping"));
            assert_eq!(mapped.role, CodeRole::ValueParam, "{ty}");
        }
        assert_eq!(value_params.len(), 14, "value-param census moved");
    }
}
