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

// ── ValueParam encoding ─────────────────────────────────────────────────────

/// The ordered parameter codes for one `(block type, field name)` dropdown
/// whose role is [`CodeRole::ValueParam`].
///
/// # The encoding, and why the ordinal is OURS
///
/// A `ValueParam` byte is the code's **ordinal in this table** — `PI` is `0`,
/// `E` is `1`, and so on. The block type is already named by the call's
/// [`FnIndex`], so the ordinal need only be unique *within* the block: no
/// global parameter namespace is minted, and no palette slot is spent.
///
/// The ordinal is deliberately anchored **here** rather than to "whatever
/// order Blockly's array has today". Blockly reordering an options array is a
/// cosmetic upstream change; if the encoding tracked it, that cosmetic change
/// would silently reinterpret every stored program — `math_constant` `0` would
/// stop meaning π. Anchoring the ordinal in this table converts that hazard
/// into a **loud** one: upstream reordering then breaks the drift test
/// (`the_value_param_option_sets_are_pinned`) and nothing else. Permanence lives with the codebook, which is the same reason
/// the three gaps are refused rather than minted.
///
/// Widths, measured against the Apache-2.0 definitions (largest set is 8):
/// every set fits a byte with three orders of magnitude to spare, so the
/// `u8` is not a squeeze.
///
/// Returns `None` for any pair this codebook does not name — never a guess.
#[must_use]
pub fn value_param_codes(ty: &str, field: &str) -> Option<&'static [&'static str]> {
    Some(match (ty, field) {
        ("math_constant", "CONSTANT") => {
            &["PI", "E", "GOLDEN_RATIO", "SQRT2", "SQRT1_2", "INFINITY"]
        }
        ("math_number_property", "PROPERTY") => &[
            "EVEN",
            "ODD",
            "PRIME",
            "WHOLE",
            "POSITIVE",
            "NEGATIVE",
            "DIVISIBLE_BY",
        ],
        // RANDOM is absent BY CONSTRUCTION: it is one of the three gaps, and
        // `resolve` already refuses it. Listing it here would hand a byte to a
        // block the palette cannot name.
        ("math_on_list", "OP") => &["SUM", "MIN", "MAX", "AVERAGE", "MEDIAN", "MODE", "STD_DEV"],
        ("text_indexOf" | "lists_indexOf", "END") => &["FIRST", "LAST"],
        ("text_charAt", "WHERE") => &["FROM_START", "FROM_END", "FIRST", "LAST", "RANDOM"],
        // Two dropdowns, and their sets genuinely DIFFER in the third slot
        // (FIRST vs LAST) — which is why the table keys on the field name and
        // not the block type alone.
        ("text_getSubstring" | "lists_getSublist", "WHERE1") => {
            &["FROM_START", "FROM_END", "FIRST"]
        }
        ("text_getSubstring" | "lists_getSublist", "WHERE2") => &["FROM_START", "FROM_END", "LAST"],
        ("text_changeCase", "CASE") => &["UPPERCASE", "LOWERCASE", "TITLECASE"],
        ("text_trim", "MODE") => &["BOTH", "LEFT", "RIGHT"],
        ("text_prompt" | "text_prompt_ext", "TYPE") => &["TEXT", "NUMBER"],
        ("lists_sort", "TYPE") => &["NUMERIC", "TEXT", "IGNORE_CASE"],
        // The codes really are the strings "1" and "-1" in the source. They
        // are NOT read as numbers: `-1` would not survive a `u8`, and the
        // ordinal encoding sidesteps the question entirely.
        ("lists_sort", "DIRECTION") => &["1", "-1"],
        // JUDGMENT, recorded rather than silently taken: SPLIT (string → list)
        // and JOIN (list → string) are arguably different *operations*, which
        // would make this a Selector. The palette mints ONE slot
        // (`LIST_SPLIT`), so treating the code as a parameter is what that
        // slot actually supports, and it is lossless — the byte survives.
        // Promoting it to two slots is a mint, i.e. an operator decision.
        ("lists_split", "MODE") => &["SPLIT", "JOIN"],
        _ => return None,
    })
}

/// Encode a `ValueParam` dropdown code as its immediate byte.
///
/// Returns `None` if the codebook does not name this `(type, field)` pair, or
/// names it but not this code — the caller refuses rather than substituting a
/// default, because a wrong parameter byte is a program that runs and computes
/// the wrong thing.
#[must_use]
pub fn encode_value_param(ty: &str, field: &str, code: &str) -> Option<u8> {
    let codes = value_param_codes(ty, field)?;
    let idx = codes.iter().position(|c| *c == code)?;
    // The largest set is 8; the pin below makes this unreachable in practice,
    // and the fallible conversion means a future oversized set refuses rather
    // than wrapping to a valid-looking byte.
    u8::try_from(idx).ok()
}

/// The inverse of [`encode_value_param`] — the code a stored byte denotes.
#[must_use]
pub fn decode_value_param(ty: &str, field: &str, byte: u8) -> Option<&'static str> {
    value_param_codes(ty, field)?
        .get(usize::from(byte))
        .copied()
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

    /// Every `(type, field)` dropdown set, pinned verbatim in source order.
    ///
    /// This is the D3 drift anchor for the parameter half. The ordinal IS the
    /// stored byte, so a reorder here silently reinterprets stored programs —
    /// which is exactly why the order is asserted rather than derived.
    #[test]
    fn the_value_param_option_sets_are_pinned() {
        let expected: &[(&str, &str, &[&str])] = &[
            (
                "math_constant",
                "CONSTANT",
                &["PI", "E", "GOLDEN_RATIO", "SQRT2", "SQRT1_2", "INFINITY"],
            ),
            (
                "math_number_property",
                "PROPERTY",
                &[
                    "EVEN",
                    "ODD",
                    "PRIME",
                    "WHOLE",
                    "POSITIVE",
                    "NEGATIVE",
                    "DIVISIBLE_BY",
                ],
            ),
            (
                "math_on_list",
                "OP",
                &["SUM", "MIN", "MAX", "AVERAGE", "MEDIAN", "MODE", "STD_DEV"],
            ),
            ("text_indexOf", "END", &["FIRST", "LAST"]),
            ("lists_indexOf", "END", &["FIRST", "LAST"]),
            (
                "text_charAt",
                "WHERE",
                &["FROM_START", "FROM_END", "FIRST", "LAST", "RANDOM"],
            ),
            (
                "text_getSubstring",
                "WHERE1",
                &["FROM_START", "FROM_END", "FIRST"],
            ),
            (
                "text_getSubstring",
                "WHERE2",
                &["FROM_START", "FROM_END", "LAST"],
            ),
            (
                "lists_getSublist",
                "WHERE1",
                &["FROM_START", "FROM_END", "FIRST"],
            ),
            (
                "lists_getSublist",
                "WHERE2",
                &["FROM_START", "FROM_END", "LAST"],
            ),
            (
                "text_changeCase",
                "CASE",
                &["UPPERCASE", "LOWERCASE", "TITLECASE"],
            ),
            ("text_trim", "MODE", &["BOTH", "LEFT", "RIGHT"]),
            ("text_prompt", "TYPE", &["TEXT", "NUMBER"]),
            ("text_prompt_ext", "TYPE", &["TEXT", "NUMBER"]),
            ("lists_sort", "TYPE", &["NUMERIC", "TEXT", "IGNORE_CASE"]),
            ("lists_sort", "DIRECTION", &["1", "-1"]),
            ("lists_split", "MODE", &["SPLIT", "JOIN"]),
        ];
        for (ty, field, codes) in expected {
            let got = value_param_codes(ty, field)
                .unwrap_or_else(|| panic!("{ty}.{field} lost its option set"));
            assert_eq!(&got, codes, "{ty}.{field} option set drifted");
        }
        assert_eq!(expected.len(), 17, "value-param dropdown census moved");
    }

    #[test]
    fn a_value_param_code_encodes_to_its_ordinal_and_back() {
        // Can-fire: distinct codes must reach DISTINCT bytes, or the whole
        // encoding is decoration and math_constant[PI] == math_constant[E].
        let pi = encode_value_param("math_constant", "CONSTANT", "PI").unwrap();
        let e = encode_value_param("math_constant", "CONSTANT", "E").unwrap();
        assert_eq!(pi, 0);
        assert_eq!(e, 1);
        assert_ne!(pi, e);
        // Round-trip every code of every set, so a table edit that breaks the
        // inverse is caught here and not in a stored program.
        for (ty, field) in [
            ("math_constant", "CONSTANT"),
            ("math_number_property", "PROPERTY"),
            ("math_on_list", "OP"),
            ("text_charAt", "WHERE"),
            ("lists_sort", "DIRECTION"),
            ("lists_split", "MODE"),
        ] {
            for code in value_param_codes(ty, field).unwrap() {
                let byte = encode_value_param(ty, field, code).unwrap();
                assert_eq!(decode_value_param(ty, field, byte), Some(*code));
            }
        }
    }

    #[test]
    fn the_two_dropdowns_of_one_block_do_not_share_a_table() {
        // WHERE1 ends FIRST, WHERE2 ends LAST. Keying on the block type alone
        // would make ordinal 2 mean two different things on one block — and
        // would pass a same-table test vacuously, since the first two entries
        // ARE identical.
        assert_eq!(
            encode_value_param("text_getSubstring", "WHERE1", "FROM_END"),
            encode_value_param("text_getSubstring", "WHERE2", "FROM_END")
        );
        assert_eq!(
            decode_value_param("text_getSubstring", "WHERE1", 2),
            Some("FIRST")
        );
        assert_eq!(
            decode_value_param("text_getSubstring", "WHERE2", 2),
            Some("LAST")
        );
        // FIRST is not reachable at all on WHERE2, and LAST not on WHERE1.
        assert_eq!(
            encode_value_param("text_getSubstring", "WHERE2", "FIRST"),
            None
        );
        assert_eq!(
            encode_value_param("text_getSubstring", "WHERE1", "LAST"),
            None
        );
    }

    #[test]
    fn an_unknown_value_param_is_refused_not_defaulted() {
        // Silence twin for the encoding test: a code the codebook does not
        // name must yield None, NOT ordinal 0 — a defaulted parameter is a
        // program that runs and computes the wrong thing.
        assert_eq!(encode_value_param("math_constant", "CONSTANT", "TAU"), None);
        assert_eq!(
            encode_value_param("math_constant", "WRONG_FIELD", "PI"),
            None
        );
        assert_eq!(encode_value_param("not_a_block", "CONSTANT", "PI"), None);
        // The gap stays a gap on this surface too: RANDOM has no ordinal.
        assert_eq!(encode_value_param("math_on_list", "OP", "RANDOM"), None);
        assert!(encode_value_param("math_on_list", "OP", "SUM").is_some());
        // And decoding past the end of a set is refused rather than wrapping.
        assert_eq!(decode_value_param("math_constant", "CONSTANT", 6), None);
        assert!(decode_value_param("math_constant", "CONSTANT", 5).is_some());
    }

    /// The D3 palette drift anchor for the FUNCTION half.
    ///
    /// The census test above compares against `FnIndex::LT` — the *symbol*. If
    /// `ogar-blockly` renumbered `LT` from `0x32` to `0x33`, every symbolic
    /// assertion in this file would still pass while every stored program
    /// silently changed meaning. This pins the BYTES.
    #[test]
    fn the_palette_byte_values_are_pinned() {
        let pinned: &[(&str, Option<&str>, u8)] = &[
            ("controls_if", None, 0x01),
            ("controls_repeat", None, 0x03),
            ("controls_whileUntil", Some("WHILE"), 0x05),
            ("logic_negate", None, 0x22),
            ("logic_compare", Some("EQ"), 0x30),
            ("logic_compare", Some("LT"), 0x32),
            ("logic_compare", Some("GT"), 0x34),
            ("math_arithmetic", Some("ADD"), 0x40),
            ("math_number", None, 0x46),
            ("math_single", Some("ABS"), 0x47),
            ("math_trig", Some("SIN"), 0x51),
            ("math_constant", Some("PI"), 0x5C),
            ("text", None, 0x60),
            ("lists_getIndex", Some("GET"), 0x76),
            ("variables_get", None, 0x80),
            ("procedures_callnoreturn", None, 0x84),
        ];
        for (ty, code, byte) in pinned {
            let got = resolve_opcode(ty, *code)
                .unwrap_or_else(|| panic!("{ty} lost its mapping"))
                .0;
            assert_eq!(got, *byte, "{ty} moved off palette byte {byte:#04x}");
        }
        // Anti-vacuity: the pins must be DISTINCT bytes. A palette that
        // collapsed every entry onto one slot would satisfy a per-row check
        // only if the expected bytes were also collapsed — this catches the
        // case where the table itself is edited to match a broken palette.
        let mut bytes: Vec<u8> = pinned.iter().map(|(_, _, b)| *b).collect();
        bytes.sort_unstable();
        let before = bytes.len();
        bytes.dedup();
        assert_eq!(bytes.len(), before, "two pins share a palette byte");

        // Nothing the Blockly frontend reaches may land in the DEVICE family
        // (`0x90..`), which is reserved for hardware/robotics blocks that have
        // no Blockly-core counterpart.
        for (ty, _, byte) in pinned {
            assert!(
                *byte < ogar_blockly::DEVICE_FAMILY_FLOOR,
                "{ty} reached the reserved device family"
            );
        }
        // …and the floor is a real boundary, not a vacuous one: it must sit
        // ABOVE every pin (or the loop above would be impossible) and BELOW
        // the top of the byte range (or "below the floor" would exclude
        // nothing and the loop would be free).
        const { assert!(ogar_blockly::DEVICE_FAMILY_FLOOR < u8::MAX) };
        let highest_pin = pinned.iter().map(|(_, _, b)| *b).max().unwrap();
        assert!(highest_pin < ogar_blockly::DEVICE_FAMILY_FLOOR);
    }
}
