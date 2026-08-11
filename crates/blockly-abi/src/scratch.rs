//! The Scratch vocabulary — device families minted here, logic borrowed
//! from the substrate.
//!
//! # Provenance (the fence, honoured)
//!
//! Every opcode string below was harvested BYTE-EXACT from Apache-2.0
//! `scratchfoundation/scratch-blocks` (branch `develop`, `src/blocks/*.ts`,
//! the TypeScript rewrite of the old `blocks_vertical/*.js`), by cloning the
//! repo and reading the `Blockly.Blocks.<opcode> = {` registration keys —
//! never recalled from memory, never transcribed by hand. Arity and body
//! references are likewise READ from each block's own `args0` declaration
//! (`input_value` / `input_statement` counts), not assigned by judgement.
//!
//! AGPL `scratch-vm` and `scratch-gui` were NOT consulted, per the fence in
//! OGAR `docs/BLOCK-EDITOR-PLAN.md`.
//!
//! Re-run the harvest:
//! ```sh
//! git clone --depth 1 -b develop \
//!   https://github.com/scratchfoundation/scratch-blocks.git
//! # then read src/blocks/*.ts for `Blockly.Blocks.<opcode> = {`
//! ```
//!
//! # The measured split — Scratch is mostly ALREADY in the substrate
//!
//! Of 162 harvested block types:
//!
//! | half | count | where it lives |
//! |---|---|---|
//! | device families (motion/looks/sound/event/sensing, + clones, + stage monitors) | 94 | minted HERE, `0x90..=0xED` |
//! | logic (operators / control / data / procedures) | 43 + 14 mathop codes | the SHARED CORE, zero mints |
//! | dropdown menus | 15 | values, not operations |
//! | editor-only (prototype / declaration / argument editors) | 4 | not operations at all |
//!
//! That table is the sharing discipline paying off, measured rather than
//! claimed: `operator_add` is the same `ADD` a Blockly `math_arithmetic[ADD]`
//! lowers to, and `control_if_else` independently declares two statement
//! inputs — matching the core's `IF_ELSE.body_refs == 2`, which was written
//! without reference to Scratch. Two frontends, one computational core; only
//! the sprite-and-stage half is Scratch's own.
//!
//! The 94 device mints occupy `0x90..=0xED`, leaving 18 slots of the palette
//! range spare for Scratch extensions (pen, music, video sensing), which are
//! deliberately NOT minted here — they live in separate extension files and
//! would be harvested the same way when a consumer needs them.

use crate::codebook::{CodeRole, OpcodeMapping};
use ogar_loco::FnIndex;

/// Every Scratch device operation: `(opcode, byte, stack_arity, body_refs)`.
///
/// Generated from the harvest described in the module docs. The byte is a
/// dense allocation from [`crate::palette::DEVICE_FAMILY_FLOOR`] in source
/// order (motion, looks, sound, event, sensing, then clone control, then the
/// stage monitors), so the family groupings stay contiguous and readable in a
/// hex dump.
pub const SCRATCH_DEVICE: &[(&str, u8, u8, u8)] = &[
    ("motion_movesteps", 0x90, 1, 0),
    ("motion_turnright", 0x91, 1, 0),
    ("motion_turnleft", 0x92, 1, 0),
    ("motion_pointindirection", 0x93, 1, 0),
    ("motion_pointtowards", 0x94, 1, 0),
    ("motion_gotoxy", 0x95, 2, 0),
    ("motion_goto", 0x96, 1, 0),
    ("motion_glidesecstoxy", 0x97, 3, 0),
    ("motion_glideto", 0x98, 2, 0),
    ("motion_changexby", 0x99, 1, 0),
    ("motion_setx", 0x9A, 1, 0),
    ("motion_changeyby", 0x9B, 1, 0),
    ("motion_sety", 0x9C, 1, 0),
    ("motion_ifonedgebounce", 0x9D, 0, 0),
    ("motion_setrotationstyle", 0x9E, 0, 0),
    ("motion_xposition", 0x9F, 0, 0),
    ("motion_yposition", 0xA0, 0, 0),
    ("motion_direction", 0xA1, 0, 0),
    ("motion_scroll_right", 0xA2, 1, 0),
    ("motion_scroll_up", 0xA3, 1, 0),
    ("motion_align_scene", 0xA4, 0, 0),
    ("motion_xscroll", 0xA5, 0, 0),
    ("motion_yscroll", 0xA6, 0, 0),
    ("looks_sayforsecs", 0xA7, 2, 0),
    ("looks_say", 0xA8, 1, 0),
    ("looks_thinkforsecs", 0xA9, 2, 0),
    ("looks_think", 0xAA, 1, 0),
    ("looks_show", 0xAB, 0, 0),
    ("looks_hide", 0xAC, 0, 0),
    ("looks_hideallsprites", 0xAD, 0, 0),
    ("looks_changeeffectby", 0xAE, 1, 0),
    ("looks_seteffectto", 0xAF, 1, 0),
    ("looks_cleargraphiceffects", 0xB0, 0, 0),
    ("looks_changesizeby", 0xB1, 1, 0),
    ("looks_setsizeto", 0xB2, 1, 0),
    ("looks_size", 0xB3, 0, 0),
    ("looks_changestretchby", 0xB4, 1, 0),
    ("looks_setstretchto", 0xB5, 1, 0),
    ("looks_switchcostumeto", 0xB6, 1, 0),
    ("looks_nextcostume", 0xB7, 0, 0),
    ("looks_switchbackdropto", 0xB8, 1, 0),
    ("looks_gotofrontback", 0xB9, 0, 0),
    ("looks_goforwardbackwardlayers", 0xBA, 1, 0),
    ("looks_backdropnumbername", 0xBB, 0, 0),
    ("looks_costumenumbername", 0xBC, 0, 0),
    ("looks_switchbackdroptoandwait", 0xBD, 1, 0),
    ("looks_nextbackdrop", 0xBE, 0, 0),
    ("sound_play", 0xBF, 1, 0),
    ("sound_playuntildone", 0xC0, 1, 0),
    ("sound_stopallsounds", 0xC1, 0, 0),
    ("sound_seteffectto", 0xC2, 1, 0),
    ("sound_changeeffectby", 0xC3, 1, 0),
    ("sound_cleareffects", 0xC4, 0, 0),
    ("sound_changevolumeby", 0xC5, 1, 0),
    ("sound_setvolumeto", 0xC6, 1, 0),
    ("sound_volume", 0xC7, 0, 0),
    ("event_whentouchingobject", 0xC8, 1, 0),
    ("event_whenflagclicked", 0xC9, 0, 0),
    ("event_whenthisspriteclicked", 0xCA, 0, 0),
    ("event_whenstageclicked", 0xCB, 0, 0),
    ("event_whenbroadcastreceived", 0xCC, 0, 0),
    ("event_whenbackdropswitchesto", 0xCD, 0, 0),
    ("event_whengreaterthan", 0xCE, 1, 0),
    ("event_broadcast", 0xCF, 1, 0),
    ("event_broadcastandwait", 0xD0, 1, 0),
    ("event_whenkeypressed", 0xD1, 0, 0),
    ("sensing_touchingobject", 0xD2, 1, 0),
    ("sensing_touchingcolor", 0xD3, 1, 0),
    ("sensing_coloristouchingcolor", 0xD4, 2, 0),
    ("sensing_distanceto", 0xD5, 1, 0),
    ("sensing_askandwait", 0xD6, 1, 0),
    ("sensing_answer", 0xD7, 0, 0),
    ("sensing_keypressed", 0xD8, 1, 0),
    ("sensing_mousedown", 0xD9, 0, 0),
    ("sensing_mousex", 0xDA, 0, 0),
    ("sensing_mousey", 0xDB, 0, 0),
    ("sensing_setdragmode", 0xDC, 0, 0),
    ("sensing_loudness", 0xDD, 0, 0),
    ("sensing_loud", 0xDE, 0, 0),
    ("sensing_timer", 0xDF, 0, 0),
    ("sensing_resettimer", 0xE0, 0, 0),
    ("sensing_of", 0xE1, 0, 0),
    ("sensing_current", 0xE2, 0, 0),
    ("sensing_dayssince2000", 0xE3, 0, 0),
    ("sensing_online", 0xE4, 0, 0),
    ("sensing_username", 0xE5, 0, 0),
    ("sensing_userid", 0xE6, 0, 0),
    ("control_start_as_clone", 0xE7, 0, 0),
    ("control_create_clone_of", 0xE8, 1, 0),
    ("control_delete_this_clone", 0xE9, 0, 0),
    ("data_showvariable", 0xEA, 0, 0),
    ("data_hidevariable", 0xEB, 0, 0),
    ("data_showlist", 0xEC, 0, 0),
    ("data_hidelist", 0xED, 0, 0),
];

/// Scratch operations that resolve to an EXISTING shared-core function.
///
/// The interesting half of the table: these need no palette byte at all,
/// because the substrate already carries the operation. Adding Scratch cost
/// zero new opcodes here.
pub const SCRATCH_CORE: &[(&str, FnIndex)] = &[
    ("operator_add", FnIndex::ADD),
    ("operator_subtract", FnIndex::SUB),
    ("operator_multiply", FnIndex::MUL),
    ("operator_divide", FnIndex::DIV),
    ("operator_random", FnIndex::RANDOM_INT),
    ("operator_lt", FnIndex::LT),
    ("operator_equals", FnIndex::EQ),
    ("operator_gt", FnIndex::GT),
    ("operator_and", FnIndex::AND),
    ("operator_or", FnIndex::OR),
    ("operator_not", FnIndex::NOT),
    ("operator_join", FnIndex::JOIN),
    ("operator_letter_of", FnIndex::CHAR_AT),
    ("operator_length", FnIndex::LENGTH),
    ("operator_contains", FnIndex::CONTAINS),
    ("operator_mod", FnIndex::MOD),
    ("operator_round", FnIndex::ROUND),
    ("control_forever", FnIndex::FOREVER),
    ("control_repeat", FnIndex::REPEAT),
    ("control_if", FnIndex::IF),
    ("control_if_else", FnIndex::IF_ELSE),
    ("control_stop", FnIndex::STOP),
    ("control_wait", FnIndex::WAIT),
    ("control_wait_until", FnIndex::WAIT_UNTIL),
    ("control_repeat_until", FnIndex::REPEAT_UNTIL),
    ("control_while", FnIndex::WHILE),
    ("control_for_each", FnIndex::FOR_EACH),
    ("data_variable", FnIndex::VAR_GET),
    ("data_setvariableto", FnIndex::VAR_SET),
    ("data_changevariableby", FnIndex::VAR_CHANGE),
    ("data_addtolist", FnIndex::LIST_ADD),
    ("data_deleteoflist", FnIndex::LIST_DELETE),
    ("data_deletealloflist", FnIndex::LIST_DELETE_ALL),
    ("data_insertatlist", FnIndex::LIST_INSERT),
    ("data_replaceitemoflist", FnIndex::LIST_SET),
    ("data_itemoflist", FnIndex::LIST_GET),
    ("data_itemnumoflist", FnIndex::LIST_INDEX_OF),
    ("data_lengthoflist", FnIndex::LIST_LENGTH),
    ("data_listcontainsitem", FnIndex::LIST_CONTAINS),
    ("procedures_definition", FnIndex::PROC_DEF),
    ("procedures_call", FnIndex::PROC_CALL),
    ("argument_reporter_boolean", FnIndex::PROC_ARG),
    ("argument_reporter_string_number", FnIndex::PROC_ARG),
];

/// `operator_mathop`'s dropdown, byte-exact from the source's `options` list.
///
/// A selector, exactly like Blockly's `math_single`: the code chooses WHICH
/// function, so it is consumed by resolution rather than becoming a value.
/// All fourteen already exist in the shared core.
pub const SCRATCH_MATHOP: &[(&str, FnIndex)] = &[
    ("abs", FnIndex::ABS),
    ("floor", FnIndex::FLOOR),
    ("ceiling", FnIndex::CEIL),
    ("sqrt", FnIndex::SQRT),
    ("sin", FnIndex::SIN),
    ("cos", FnIndex::COS),
    ("tan", FnIndex::TAN),
    ("asin", FnIndex::ASIN),
    ("acos", FnIndex::ACOS),
    ("atan", FnIndex::ATAN),
    ("ln", FnIndex::LN),
    ("log", FnIndex::LOG10),
    ("e ^", FnIndex::EXP_E),
    ("10 ^", FnIndex::EXP_10),
];

/// Look up a device opcode's row.
#[must_use]
pub fn device(ty: &str) -> Option<(u8, u8, u8)> {
    SCRATCH_DEVICE
        .iter()
        .find(|(name, ..)| *name == ty)
        .map(|&(_, b, a, r)| (b, a, r))
}

/// The device row for a palette byte, if it is minted.
#[must_use]
pub fn device_by_byte(byte: u8) -> Option<(&'static str, u8, u8)> {
    SCRATCH_DEVICE
        .iter()
        .find(|(_, b, ..)| *b == byte)
        .map(|&(n, _, a, r)| (n, a, r))
}

/// Resolve a Scratch block type to its function.
///
/// Returns `None` for menus, editor-only blocks, and the Scratch-internal
/// counter blocks — never a guess. A `None` here is the same loud refusal the
/// Blockly codebook gives: the cast refuses rather than storing wrong bytes.
#[must_use]
pub fn resolve_scratch(ty: &str, code: Option<&str>) -> Option<OpcodeMapping> {
    if ty == "operator_mathop" {
        let c = code?;
        let f = SCRATCH_MATHOP.iter().find(|(k, _)| *k == c)?.1;
        return Some(OpcodeMapping {
            function: f,
            role: CodeRole::Selector,
        });
    }
    if let Some((byte, ..)) = device(ty) {
        return Some(OpcodeMapping {
            function: FnIndex(byte),
            role: CodeRole::None,
        });
    }
    SCRATCH_CORE
        .iter()
        .find(|(k, _)| *k == ty)
        .map(|&(_, function)| OpcodeMapping {
            function,
            role: CodeRole::None,
        })
}

/// The Scratch palette as presented: `(label, family key, block types)`.
///
/// Ordering and grouping follow Scratch's own category order. The family key
/// is the source file the opcodes came from, and is what a page uses to pick
/// a tile colour — the colours themselves are NOT here, because they are no
/// longer defined in `scratch-blocks` (the `colours_<family>` extensions are
/// referenced by the block definitions but registered by the GUI). Inventing
/// hex values and calling them harvested would be exactly the fabrication the
/// provenance fence exists to prevent, so tile colour is left to the page as
/// a labelled presentation choice.
///
/// Menus, editor-only blocks, and the Scratch-internal counter blocks are
/// excluded: they are not operations, so offering them would put a block on
/// the palette that the cast refuses on drag.
pub const SCRATCH_CATEGORIES: &[(&str, &str, &[&str])] = &[
    (
        "Motion",
        "motion",
        &[
            "motion_movesteps",
            "motion_turnright",
            "motion_turnleft",
            "motion_pointindirection",
            "motion_pointtowards",
            "motion_gotoxy",
            "motion_goto",
            "motion_glidesecstoxy",
            "motion_glideto",
            "motion_changexby",
            "motion_setx",
            "motion_changeyby",
            "motion_sety",
            "motion_ifonedgebounce",
            "motion_setrotationstyle",
            "motion_xposition",
            "motion_yposition",
            "motion_direction",
            "motion_scroll_right",
            "motion_scroll_up",
            "motion_align_scene",
            "motion_xscroll",
            "motion_yscroll",
        ],
    ),
    (
        "Looks",
        "looks",
        &[
            "looks_sayforsecs",
            "looks_say",
            "looks_thinkforsecs",
            "looks_think",
            "looks_show",
            "looks_hide",
            "looks_hideallsprites",
            "looks_changeeffectby",
            "looks_seteffectto",
            "looks_cleargraphiceffects",
            "looks_changesizeby",
            "looks_setsizeto",
            "looks_size",
            "looks_changestretchby",
            "looks_setstretchto",
            "looks_switchcostumeto",
            "looks_nextcostume",
            "looks_switchbackdropto",
            "looks_gotofrontback",
            "looks_goforwardbackwardlayers",
            "looks_backdropnumbername",
            "looks_costumenumbername",
            "looks_switchbackdroptoandwait",
            "looks_nextbackdrop",
        ],
    ),
    (
        "Sound",
        "sound",
        &[
            "sound_play",
            "sound_playuntildone",
            "sound_stopallsounds",
            "sound_seteffectto",
            "sound_changeeffectby",
            "sound_cleareffects",
            "sound_changevolumeby",
            "sound_setvolumeto",
            "sound_volume",
        ],
    ),
    (
        "Events",
        "event",
        &[
            "event_whentouchingobject",
            "event_whenflagclicked",
            "event_whenthisspriteclicked",
            "event_whenstageclicked",
            "event_whenbroadcastreceived",
            "event_whenbackdropswitchesto",
            "event_whengreaterthan",
            "event_broadcast",
            "event_broadcastandwait",
            "event_whenkeypressed",
        ],
    ),
    (
        "Control",
        "control",
        &[
            "control_forever",
            "control_repeat",
            "control_if",
            "control_if_else",
            "control_stop",
            "control_wait",
            "control_wait_until",
            "control_repeat_until",
            "control_while",
            "control_for_each",
            "control_start_as_clone",
            "control_create_clone_of",
            "control_delete_this_clone",
        ],
    ),
    (
        "Sensing",
        "sensing",
        &[
            "sensing_touchingobject",
            "sensing_touchingcolor",
            "sensing_coloristouchingcolor",
            "sensing_distanceto",
            "sensing_askandwait",
            "sensing_answer",
            "sensing_keypressed",
            "sensing_mousedown",
            "sensing_mousex",
            "sensing_mousey",
            "sensing_setdragmode",
            "sensing_loudness",
            "sensing_loud",
            "sensing_timer",
            "sensing_resettimer",
            "sensing_of",
            "sensing_current",
            "sensing_dayssince2000",
            "sensing_online",
            "sensing_username",
            "sensing_userid",
        ],
    ),
    (
        "Operators",
        "operators",
        &[
            "operator_add",
            "operator_subtract",
            "operator_multiply",
            "operator_divide",
            "operator_random",
            "operator_lt",
            "operator_equals",
            "operator_gt",
            "operator_and",
            "operator_or",
            "operator_not",
            "operator_join",
            "operator_letter_of",
            "operator_length",
            "operator_contains",
            "operator_mod",
            "operator_round",
            "operator_mathop",
        ],
    ),
    (
        "Variables",
        "data",
        &[
            "data_variable",
            "data_setvariableto",
            "data_changevariableby",
            "data_showvariable",
            "data_hidevariable",
            "data_addtolist",
            "data_deleteoflist",
            "data_deletealloflist",
            "data_insertatlist",
            "data_replaceitemoflist",
            "data_itemoflist",
            "data_itemnumoflist",
            "data_lengthoflist",
            "data_listcontainsitem",
            "data_showlist",
            "data_hidelist",
        ],
    ),
    (
        "My Blocks",
        "procedures",
        &[
            "procedures_definition",
            "procedures_call",
            "argument_reporter_boolean",
            "argument_reporter_string_number",
        ],
    ),
];

/// How a Scratch block connects — read from its `extensions` list in the
/// Apache-2.0 source (`shape_hat` / `output_*` / `shape_statement`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// An event hat: starts a stack, nothing may connect above it.
    Hat,
    /// A stackable statement.
    Statement,
    /// A round value reporter.
    Reporter,
    /// A hexagonal boolean reporter.
    Boolean,
}

/// One Scratch tile as a browser needs to DEFINE it: `(type, family, shape,
/// value input names, statement input names)`.
///
/// Named because the tuple is wide enough that a bare type reads as noise at
/// the use site — the fields are what matter, not the shape of the row.
///
/// Every Scratch tile the palette offers, with what a browser needs to DEFINE
/// it, is listed in [`SCRATCH_BLOCK_DEFS`].
///
/// The input NAMES are Scratch's own, harvested from the same source as the
/// opcodes (`args0`'s `name:` fields) rather than invented — `CONDITION`,
/// `SUBSTACK`, `SUBSTACK2`, `STEPS`. That matters beyond tidiness: a workspace
/// saved from this demo is then shaped like a real Scratch project rather than
/// carrying placeholder socket names, so a stored template reads as Scratch to
/// anything else that knows the format.
///
/// The demo's page has no Scratch block definitions available — vanilla
/// Blockly ships Blockly's blocks, not Scratch's — so it generates them from
/// this table at boot. That is deliberate: the tiles a user drags are built
/// from the SAME harvested rows that mint the opcodes, so a tile cannot exist
/// without the operation behind it, and its input count cannot disagree with
/// the arity the palette reports.
pub type BlockDef = (
    &'static str,
    &'static str,
    Shape,
    &'static [&'static str],
    &'static [&'static str],
);

/// See [`SCRATCH_BLOCK_DEFS`].
pub const SCRATCH_BLOCK_DEFS: &[BlockDef] = &[
    (
        "motion_movesteps",
        "motion",
        Shape::Statement,
        &["STEPS"],
        &[],
    ),
    (
        "motion_turnright",
        "motion",
        Shape::Statement,
        &["DEGREES"],
        &[],
    ),
    (
        "motion_turnleft",
        "motion",
        Shape::Statement,
        &["DEGREES"],
        &[],
    ),
    (
        "motion_pointindirection",
        "motion",
        Shape::Statement,
        &["DIRECTION"],
        &[],
    ),
    (
        "motion_pointtowards",
        "motion",
        Shape::Statement,
        &["TOWARDS"],
        &[],
    ),
    (
        "motion_gotoxy",
        "motion",
        Shape::Statement,
        &["X", "Y"],
        &[],
    ),
    ("motion_goto", "motion", Shape::Statement, &["TO"], &[]),
    (
        "motion_glidesecstoxy",
        "motion",
        Shape::Statement,
        &["SECS", "X", "Y"],
        &[],
    ),
    (
        "motion_glideto",
        "motion",
        Shape::Statement,
        &["SECS", "TO"],
        &[],
    ),
    ("motion_changexby", "motion", Shape::Statement, &["DX"], &[]),
    ("motion_setx", "motion", Shape::Statement, &["X"], &[]),
    ("motion_changeyby", "motion", Shape::Statement, &["DY"], &[]),
    ("motion_sety", "motion", Shape::Statement, &["Y"], &[]),
    (
        "motion_ifonedgebounce",
        "motion",
        Shape::Statement,
        &[],
        &[],
    ),
    (
        "motion_setrotationstyle",
        "motion",
        Shape::Statement,
        &[],
        &[],
    ),
    ("motion_xposition", "motion", Shape::Reporter, &[], &[]),
    ("motion_yposition", "motion", Shape::Reporter, &[], &[]),
    ("motion_direction", "motion", Shape::Reporter, &[], &[]),
    (
        "motion_scroll_right",
        "motion",
        Shape::Statement,
        &["DISTANCE"],
        &[],
    ),
    (
        "motion_scroll_up",
        "motion",
        Shape::Statement,
        &["DISTANCE"],
        &[],
    ),
    ("motion_align_scene", "motion", Shape::Statement, &[], &[]),
    ("motion_xscroll", "motion", Shape::Reporter, &[], &[]),
    ("motion_yscroll", "motion", Shape::Reporter, &[], &[]),
    (
        "looks_sayforsecs",
        "looks",
        Shape::Statement,
        &["MESSAGE", "SECS"],
        &[],
    ),
    ("looks_say", "looks", Shape::Statement, &["MESSAGE"], &[]),
    (
        "looks_thinkforsecs",
        "looks",
        Shape::Statement,
        &["MESSAGE", "SECS"],
        &[],
    ),
    ("looks_think", "looks", Shape::Statement, &["MESSAGE"], &[]),
    ("looks_show", "looks", Shape::Statement, &[], &[]),
    ("looks_hide", "looks", Shape::Statement, &[], &[]),
    ("looks_hideallsprites", "looks", Shape::Statement, &[], &[]),
    (
        "looks_changeeffectby",
        "looks",
        Shape::Statement,
        &["CHANGE"],
        &[],
    ),
    (
        "looks_seteffectto",
        "looks",
        Shape::Statement,
        &["VALUE"],
        &[],
    ),
    (
        "looks_cleargraphiceffects",
        "looks",
        Shape::Statement,
        &[],
        &[],
    ),
    (
        "looks_changesizeby",
        "looks",
        Shape::Statement,
        &["CHANGE"],
        &[],
    ),
    ("looks_setsizeto", "looks", Shape::Statement, &["SIZE"], &[]),
    ("looks_size", "looks", Shape::Reporter, &[], &[]),
    (
        "looks_changestretchby",
        "looks",
        Shape::Statement,
        &["CHANGE"],
        &[],
    ),
    (
        "looks_setstretchto",
        "looks",
        Shape::Statement,
        &["STRETCH"],
        &[],
    ),
    (
        "looks_switchcostumeto",
        "looks",
        Shape::Statement,
        &["COSTUME"],
        &[],
    ),
    ("looks_nextcostume", "looks", Shape::Statement, &[], &[]),
    (
        "looks_switchbackdropto",
        "looks",
        Shape::Statement,
        &["BACKDROP"],
        &[],
    ),
    ("looks_gotofrontback", "looks", Shape::Statement, &[], &[]),
    (
        "looks_backdropnumbername",
        "looks",
        Shape::Reporter,
        &[],
        &[],
    ),
    (
        "looks_costumenumbername",
        "looks",
        Shape::Reporter,
        &[],
        &[],
    ),
    ("looks_nextbackdrop", "looks", Shape::Statement, &[], &[]),
    (
        "sound_play",
        "sound",
        Shape::Statement,
        &["SOUND_MENU"],
        &[],
    ),
    (
        "sound_playuntildone",
        "sound",
        Shape::Statement,
        &["SOUND_MENU"],
        &[],
    ),
    ("sound_stopallsounds", "sound", Shape::Statement, &[], &[]),
    (
        "sound_seteffectto",
        "sound",
        Shape::Statement,
        &["VALUE"],
        &[],
    ),
    (
        "sound_changeeffectby",
        "sound",
        Shape::Statement,
        &["VALUE"],
        &[],
    ),
    ("sound_cleareffects", "sound", Shape::Statement, &[], &[]),
    (
        "sound_changevolumeby",
        "sound",
        Shape::Statement,
        &["VOLUME"],
        &[],
    ),
    (
        "sound_setvolumeto",
        "sound",
        Shape::Statement,
        &["VOLUME"],
        &[],
    ),
    ("sound_volume", "sound", Shape::Reporter, &[], &[]),
    (
        "event_whentouchingobject",
        "event",
        Shape::Hat,
        &["TOUCHINGOBJECTMENU"],
        &[],
    ),
    ("event_whenflagclicked", "event", Shape::Hat, &[], &[]),
    ("event_whenthisspriteclicked", "event", Shape::Hat, &[], &[]),
    ("event_whenstageclicked", "event", Shape::Hat, &[], &[]),
    ("event_whenbroadcastreceived", "event", Shape::Hat, &[], &[]),
    (
        "event_whengreaterthan",
        "event",
        Shape::Hat,
        &["VALUE"],
        &[],
    ),
    (
        "event_broadcast",
        "event",
        Shape::Statement,
        &["BROADCAST_INPUT"],
        &[],
    ),
    (
        "event_broadcastandwait",
        "event",
        Shape::Statement,
        &["BROADCAST_INPUT"],
        &[],
    ),
    ("event_whenkeypressed", "event", Shape::Hat, &[], &[]),
    (
        "control_forever",
        "control",
        Shape::Statement,
        &[],
        &["SUBSTACK"],
    ),
    (
        "control_repeat",
        "control",
        Shape::Statement,
        &["TIMES"],
        &["SUBSTACK"],
    ),
    (
        "control_if",
        "control",
        Shape::Statement,
        &["CONDITION"],
        &["SUBSTACK"],
    ),
    (
        "control_if_else",
        "control",
        Shape::Statement,
        &["CONDITION"],
        &["SUBSTACK", "SUBSTACK2"],
    ),
    ("control_stop", "control", Shape::Statement, &[], &[]),
    (
        "control_wait",
        "control",
        Shape::Statement,
        &["DURATION"],
        &[],
    ),
    (
        "control_wait_until",
        "control",
        Shape::Statement,
        &["CONDITION"],
        &[],
    ),
    (
        "control_repeat_until",
        "control",
        Shape::Statement,
        &["CONDITION"],
        &["SUBSTACK"],
    ),
    (
        "control_while",
        "control",
        Shape::Statement,
        &["CONDITION"],
        &["SUBSTACK"],
    ),
    (
        "control_for_each",
        "control",
        Shape::Statement,
        &["VALUE"],
        &["SUBSTACK"],
    ),
    ("control_start_as_clone", "control", Shape::Hat, &[], &[]),
    (
        "control_create_clone_of",
        "control",
        Shape::Statement,
        &["CLONE_OPTION"],
        &[],
    ),
    (
        "sensing_touchingobject",
        "sensing",
        Shape::Boolean,
        &["TOUCHINGOBJECTMENU"],
        &[],
    ),
    (
        "sensing_touchingcolor",
        "sensing",
        Shape::Boolean,
        &["COLOR"],
        &[],
    ),
    (
        "sensing_distanceto",
        "sensing",
        Shape::Reporter,
        &["DISTANCETOMENU"],
        &[],
    ),
    (
        "sensing_askandwait",
        "sensing",
        Shape::Statement,
        &["QUESTION"],
        &[],
    ),
    ("sensing_answer", "sensing", Shape::Reporter, &[], &[]),
    (
        "sensing_keypressed",
        "sensing",
        Shape::Boolean,
        &["KEY_OPTION"],
        &[],
    ),
    ("sensing_mousedown", "sensing", Shape::Boolean, &[], &[]),
    ("sensing_mousex", "sensing", Shape::Reporter, &[], &[]),
    ("sensing_mousey", "sensing", Shape::Reporter, &[], &[]),
    ("sensing_setdragmode", "sensing", Shape::Statement, &[], &[]),
    ("sensing_loudness", "sensing", Shape::Reporter, &[], &[]),
    ("sensing_loud", "sensing", Shape::Boolean, &[], &[]),
    ("sensing_timer", "sensing", Shape::Reporter, &[], &[]),
    ("sensing_resettimer", "sensing", Shape::Statement, &[], &[]),
    ("sensing_of", "sensing", Shape::Statement, &[], &[]),
    ("sensing_current", "sensing", Shape::Reporter, &[], &[]),
    (
        "sensing_dayssince2000",
        "sensing",
        Shape::Reporter,
        &[],
        &[],
    ),
    ("sensing_online", "sensing", Shape::Boolean, &[], &[]),
    ("sensing_username", "sensing", Shape::Reporter, &[], &[]),
    ("sensing_userid", "sensing", Shape::Reporter, &[], &[]),
    (
        "operator_add",
        "operators",
        Shape::Reporter,
        &["NUM1", "NUM2"],
        &[],
    ),
    (
        "operator_subtract",
        "operators",
        Shape::Reporter,
        &["NUM1", "NUM2"],
        &[],
    ),
    (
        "operator_multiply",
        "operators",
        Shape::Reporter,
        &["NUM1", "NUM2"],
        &[],
    ),
    (
        "operator_divide",
        "operators",
        Shape::Reporter,
        &["NUM1", "NUM2"],
        &[],
    ),
    (
        "operator_random",
        "operators",
        Shape::Reporter,
        &["FROM", "TO"],
        &[],
    ),
    (
        "operator_lt",
        "operators",
        Shape::Boolean,
        &["OPERAND1", "OPERAND2"],
        &[],
    ),
    (
        "operator_equals",
        "operators",
        Shape::Boolean,
        &["OPERAND1", "OPERAND2"],
        &[],
    ),
    (
        "operator_gt",
        "operators",
        Shape::Boolean,
        &["OPERAND1", "OPERAND2"],
        &[],
    ),
    (
        "operator_and",
        "operators",
        Shape::Boolean,
        &["OPERAND1", "OPERAND2"],
        &[],
    ),
    (
        "operator_or",
        "operators",
        Shape::Boolean,
        &["OPERAND1", "OPERAND2"],
        &[],
    ),
    (
        "operator_not",
        "operators",
        Shape::Boolean,
        &["OPERAND"],
        &[],
    ),
    (
        "operator_join",
        "operators",
        Shape::Reporter,
        &["STRING1", "STRING2"],
        &[],
    ),
    (
        "operator_letter_of",
        "operators",
        Shape::Reporter,
        &["LETTER", "STRING"],
        &[],
    ),
    (
        "operator_length",
        "operators",
        Shape::Reporter,
        &["STRING"],
        &[],
    ),
    (
        "operator_contains",
        "operators",
        Shape::Boolean,
        &["STRING1", "STRING2"],
        &[],
    ),
    (
        "operator_mod",
        "operators",
        Shape::Reporter,
        &["NUM1", "NUM2"],
        &[],
    ),
    (
        "operator_round",
        "operators",
        Shape::Reporter,
        &["NUM"],
        &[],
    ),
    (
        "operator_mathop",
        "operators",
        Shape::Reporter,
        &["NUM"],
        &[],
    ),
    ("data_variable", "data", Shape::Reporter, &[], &[]),
    (
        "data_setvariableto",
        "data",
        Shape::Statement,
        &["VALUE"],
        &[],
    ),
    (
        "data_changevariableby",
        "data",
        Shape::Statement,
        &["VALUE"],
        &[],
    ),
    ("data_showvariable", "data", Shape::Statement, &[], &[]),
    ("data_hidevariable", "data", Shape::Statement, &[], &[]),
    ("data_addtolist", "data", Shape::Statement, &["ITEM"], &[]),
    (
        "data_deleteoflist",
        "data",
        Shape::Statement,
        &["INDEX"],
        &[],
    ),
    ("data_deletealloflist", "data", Shape::Statement, &[], &[]),
    (
        "data_insertatlist",
        "data",
        Shape::Statement,
        &["ITEM", "INDEX"],
        &[],
    ),
    (
        "data_replaceitemoflist",
        "data",
        Shape::Statement,
        &["INDEX", "ITEM"],
        &[],
    ),
    ("data_itemoflist", "data", Shape::Statement, &["INDEX"], &[]),
    (
        "data_itemnumoflist",
        "data",
        Shape::Statement,
        &["ITEM"],
        &[],
    ),
    ("data_lengthoflist", "data", Shape::Reporter, &[], &[]),
    (
        "data_listcontainsitem",
        "data",
        Shape::Boolean,
        &["ITEM"],
        &[],
    ),
    ("data_showlist", "data", Shape::Statement, &[], &[]),
    ("data_hidelist", "data", Shape::Statement, &[], &[]),
    ("procedures_call", "procedures", Shape::Statement, &[], &[]),
];

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_loco::vocabulary::shared_core;

    /// The device allocation is dense, in range, and collision-free.
    #[test]
    fn the_device_mint_is_dense_in_range_and_unique() {
        assert_eq!(SCRATCH_DEVICE.len(), 94);
        let floor = crate::palette::DEVICE_FAMILY_FLOOR;
        let mut bytes: Vec<u8> = SCRATCH_DEVICE.iter().map(|&(_, b, ..)| b).collect();
        let n = bytes.len();
        bytes.sort_unstable();
        bytes.dedup();
        assert_eq!(bytes.len(), n, "two device opcodes share a byte");
        assert_eq!(
            *bytes.first().unwrap(),
            floor,
            "allocation must start at the floor"
        );
        assert_eq!(*bytes.last().unwrap(), 0xED);
        // Dense: no holes inside the allocated span.
        for (i, b) in bytes.iter().enumerate() {
            assert_eq!(*b, floor + i as u8, "hole in the device allocation");
        }
        // Every device byte is genuinely IN the palette's own range, never
        // squatting on a shared-core opcode.
        for &(name, b, ..) in SCRATCH_DEVICE {
            assert!(
                b >= floor,
                "{name} at {b:#04x} collides with the shared core"
            );
            assert!(
                FnIndex(b).is_domain_specific(),
                "{name} is not in the domain range"
            );
        }
        // …and 15 slots stay free for the extensions deliberately not minted.
        assert_eq!(0xFFu16 - u16::from(*bytes.last().unwrap()), 18);
    }

    /// Scratch's LOGIC half needs no mint — the substrate already has it.
    ///
    /// This is the headline claim of the whole module, so it is measured in
    /// both directions: every borrowed opcode must resolve to a byte BELOW
    /// the device floor (i.e. genuinely the shared core, not a device mint
    /// wearing a core name), and the shared core must actually answer for it.
    #[test]
    fn every_borrowed_scratch_opcode_resolves_to_the_real_shared_core() {
        assert_eq!(SCRATCH_CORE.len(), 43);
        let floor = crate::palette::DEVICE_FAMILY_FLOOR;
        for &(name, f) in SCRATCH_CORE {
            assert!(
                f.0 < floor,
                "{name} claims to borrow the core but sits at {:#04x}",
                f.0
            );
            assert!(f.is_shared_core(), "{name} is not a shared-core byte");
        }

        // ── A MEASURED SUBSTRATE GAP, pinned rather than papered over ──
        //
        // This pin did its job. It was 21: that many borrowed opcodes were
        // NAMED by the core but carried no row in `shared_core::stack_arity`
        // — a gap in `ogar-loco` that predated Scratch (37 Blockly types hit
        // the same bytes). Per Core-First the fix went upstream rather than
        // into a local table here, and when it landed this assertion failed
        // exactly as designed, demanding a deliberate re-pin.
        //
        // Now 1: `PROC_CALL`, which stays refused upstream because it is
        // genuinely variadic (a call passes as many arguments as its
        // procedure declares), not because anyone forgot it.
        let arity_gap = SCRATCH_CORE
            .iter()
            .filter(|&&(_, f)| {
                shared_core::stack_arity(f).is_none() && shared_core::body_refs(f) == 0
            })
            .count();
        assert_eq!(
            arity_gap, 1,
            "the shared core's arity coverage moved; re-measure with the \
             core_gap example and re-pin deliberately"
        );
        // Every mathop selector likewise lands in the core.
        assert_eq!(SCRATCH_MATHOP.len(), 14);
        for &(code, f) in SCRATCH_MATHOP {
            assert!(f.is_shared_core(), "mathop '{code}' left the shared core");
        }
    }

    /// The two frontends meet on the SAME function, not on a lookalike.
    ///
    /// If Blockly's `math_arithmetic[ADD]` and Scratch's `operator_add` ever
    /// resolved to different bytes, the "one computational core, two
    /// frontends" claim would be decoration.
    #[test]
    fn blockly_and_scratch_lower_the_same_operation_to_the_same_byte() {
        let pairs = [
            ("math_arithmetic", Some("ADD"), "operator_add"),
            ("math_arithmetic", Some("MULTIPLY"), "operator_multiply"),
            ("logic_operation", Some("AND"), "operator_and"),
            ("logic_compare", Some("LT"), "operator_lt"),
            ("controls_if", None, "control_if"),
            ("controls_ifelse", None, "control_if_else"),
            ("controls_repeat", None, "control_repeat"),
            ("text_join", None, "operator_join"),
            ("lists_length", None, "data_lengthoflist"),
            ("variables_set", None, "data_setvariableto"),
        ];
        for (bty, bcode, sty) in pairs {
            let b = crate::codebook::resolve(bty, bcode)
                .unwrap_or_else(|| panic!("blockly {bty} lost its mapping"));
            let s = resolve_scratch(sty, None)
                .unwrap_or_else(|| panic!("scratch {sty} lost its mapping"));
            assert_eq!(
                b.function, s.function,
                "{bty} and {sty} must be the same operation"
            );
        }
        // Scratch's own declaration agrees with the core it borrows: the
        // source says control_if_else has two statement inputs, and the core
        // — written without reference to Scratch — says IF_ELSE branches twice.
        assert_eq!(shared_core::body_refs(FnIndex::IF_ELSE), 2);
    }

    /// Non-operations are refused, loudly, rather than given an opcode.
    #[test]
    fn menus_editors_and_internals_do_not_resolve() {
        for ty in [
            "motion_goto_menu",            // dropdown shadow — a value
            "looks_costume",               // dropdown shadow — a value
            "procedures_prototype",        // editor-only
            "argument_editor_boolean",     // editor-only
            "control_get_counter",         // Scratch-internal legacy
            "nonsense_block_that_is_fake", // unknown
        ] {
            assert!(
                resolve_scratch(ty, None).is_none(),
                "{ty} must be refused, not given an opcode"
            );
        }
        // …and the can-fire half, so the refusal above carries information.
        assert!(resolve_scratch("motion_movesteps", None).is_some());
        assert!(resolve_scratch("operator_mathop", Some("sqrt")).is_some());
        // A mathop with an unknown dropdown code is refused, not guessed.
        assert!(resolve_scratch("operator_mathop", Some("tesseract")).is_none());
    }

    /// Every tile the palette offers actually casts.
    ///
    /// The Blockly half had a 46-of-64 under-exposure bug; this is the same
    /// gate for the Scratch half, in both directions.
    #[test]
    fn every_offered_tile_resolves_and_every_device_op_is_offered() {
        let offered: Vec<&str> = SCRATCH_CATEGORIES
            .iter()
            .flat_map(|(_, _, types)| types.iter().copied())
            .collect();
        assert_eq!(offered.len(), 138);
        for ty in &offered {
            let code = if *ty == "operator_mathop" {
                Some("abs")
            } else {
                None
            };
            assert!(
                resolve_scratch(ty, code).is_some(),
                "{ty} is on the palette but the cast refuses it"
            );
        }
        // No device operation is hidden from the palette.
        for &(name, ..) in SCRATCH_DEVICE {
            assert!(
                offered.contains(&name),
                "{name} is minted but invisible on the palette"
            );
        }
    }
}
