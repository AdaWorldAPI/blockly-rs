//! Scratch's dropdown menus — **values, not operations** — as basin codebooks.
//!
//! # Provenance (the fence, honoured)
//!
//! Every option code below was harvested BYTE-EXACT from Apache-2.0
//! `scratchfoundation/scratch-blocks` (branch `develop`, `src/blocks/*.ts`):
//! each `field_dropdown` declaration's `options` array, second column. The
//! `control_stop` set comes from the three string constants its `init`
//! declares (`ALL_SCRIPTS` / `THIS_SCRIPT` / `OTHER_SCRIPTS`); the
//! `event_touchingobjectmenu` pair from that block's own `options`. AGPL
//! `scratch-vm` / `scratch-gui` were NOT consulted.
//!
//! Re-run the harvest: read every `type: 'field_dropdown'` in
//! `src/blocks/*.ts` and take `options[i][1]`.
//!
//! # Why a menu is a codebook and not an opcode
//!
//! OGAR #295 measured the palette range at **112** opcode slots and ruled: *a
//! large palette is ONE function plus a shared value table, never N
//! functions.* A dropdown is exactly that shape. `event_whenkeypressed` is one
//! function; its 42 keys are not 42 hats but one operand byte indexing the
//! `KEY_OPTION` codebook. The declaration lives on the palette
//! ([`Vocabulary::value_codebook`](ogar_loco::Vocabulary::value_codebook)) and
//! the table it names is a sealed
//! [`BasinCodebook`] — one writer at mint
//! time, none afterwards.
//!
//! # Static prefix, per-project tail
//!
//! Two kinds of menu exist in the source, and the split is the codebook's
//! "same id, two basins, two readings" property made concrete:
//!
//! - **Static** menus (`KEY_OPTION`, `EFFECT`, `FRONT_BACK`, …) list every
//!   option in the block definition. Those options are interned here, in
//!   source order, and index `1..=n` is fixed for every project.
//! - **Dynamic** menus (`motion_goto_menu`, `looks_costume`, `sound_sounds_menu`,
//!   …) are registered EMPTY in scratch-blocks — the GUI fills them with the
//!   project's sprite / costume / sound names. Their static prefix here is
//!   therefore empty, and a project interns its own names into a
//!   [`BasinCodebookBuilder`] obtained from [`builder`] before sealing. Two
//!   projects legitimately disagree about what index 1 means; that is the
//!   basin.
//!
//! # Where the byte lives
//!
//! A dropdown's index is the call's immediate in field order — the same slot
//! a `NUMBER` literal uses — and `0` is the zero-fallback ("no option"), so
//! the first real option is `1`. A menu *shadow block* (`sensing_keyoptions`)
//! is a device reporter whose immediate is that index; it pushes the index
//! and the consuming block (`sensing_keypressed`) pops it as an operand, which
//! is how a menu that arrives as a nested block and a menu that arrives as an
//! inline field lower to the same bytes.
//!
//! # One exception, stated
//!
//! `control_stop` is the shared core's `STOP`, and the substrate declares no
//! codebook for core bytes (`value_codebook` answers `None` below the domain
//! floor by construction). Its `STOP_OPTION` byte is still encoded and
//! decoded through the table here — the palette is the authority for that one
//! field — but `resolve_operand` will not reach it. Recorded rather than
//! worked around.

use ogar_loco::basin::{BasinCodebook, BasinCodebookBuilder, BasinCodebooks};
use ogar_loco::pool::{CONSTANT_BYTES, PoolError};
use ogar_loco::vocabulary::ValueCodebook;

/// One dropdown menu: its basin-local codebook id, its legend name, and the
/// static option prefix harvested from the block definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Menu {
    /// The [`ValueCodebook::id`] — meaningful within this palette's basin.
    pub id: u8,
    /// Legend name, mirrors [`ValueCodebook::name`].
    pub name: &'static str,
    /// Options in source order; index `i` is stored as byte `i + 1`.
    pub options: &'static [&'static str],
}

/// Placeholder classid for a menu option whose code is wider than one facet
/// and is therefore stored as an FNV-1a digest. Deliberately invalid, same
/// posture as `ogar_loco::pool::placeholder`: a placeholder escaping into
/// stored data must be loud, not plausible. Real deployments supply a minted
/// id to [`basin_codebooks`].
pub const PLACEHOLDER_DIGEST_CLASSID: u32 = 0xDEAD_0003;

const KEYS: &[&str] = &[
    "space",
    "up arrow",
    "down arrow",
    "right arrow",
    "left arrow",
    "any",
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
];

/// Every menu, static and dynamic. Ids are dense from 1; `0` is never a
/// codebook, just as `0` is never an option.
pub const SCRATCH_MENUS: &[Menu] = &[
    Menu {
        id: 1,
        name: "KEY_OPTION",
        options: KEYS,
    },
    Menu {
        id: 2,
        name: "LOOKS_EFFECT",
        options: &[
            "COLOR",
            "FISHEYE",
            "WHIRL",
            "PIXELATE",
            "MOSAIC",
            "BRIGHTNESS",
            "GHOST",
        ],
    },
    Menu {
        id: 3,
        name: "SOUND_EFFECT",
        options: &["PITCH", "PAN"],
    },
    Menu {
        id: 4,
        name: "FRONT_BACK",
        options: &["front", "back"],
    },
    Menu {
        id: 5,
        name: "FORWARD_BACKWARD",
        options: &["forward", "backward"],
    },
    Menu {
        id: 6,
        name: "NUMBER_NAME",
        options: &["number", "name"],
    },
    Menu {
        id: 7,
        name: "ROTATION_STYLE",
        options: &["left-right", "all around"],
    },
    Menu {
        id: 8,
        name: "ALIGNMENT",
        options: &[
            "bottom-left",
            "bottom-right",
            "middle",
            "top-left",
            "top-right",
        ],
    },
    Menu {
        id: 9,
        name: "DRAG_MODE",
        options: &["draggable", "not draggable"],
    },
    Menu {
        id: 10,
        name: "CURRENT",
        options: &[
            "YEAR",
            "MONTH",
            "DATE",
            "DAYOFWEEK",
            "HOUR",
            "MINUTE",
            "SECOND",
        ],
    },
    Menu {
        id: 11,
        name: "WHENGREATERTHAN",
        options: &["LOUDNESS", "TIMER"],
    },
    Menu {
        id: 12,
        name: "STOP_OPTION",
        options: &["all", "this script", "other scripts in sprite"],
    },
    Menu {
        id: 13,
        name: "EVENT_TOUCHING",
        options: &["_mouse_", "_edge_"],
    },
    // Dynamic menus: registered empty in scratch-blocks, filled per project.
    Menu {
        id: 14,
        name: "TOUCHING_OBJECT",
        options: &[],
    },
    Menu {
        id: 15,
        name: "DISTANCE_TO",
        options: &[],
    },
    Menu {
        id: 16,
        name: "POINT_TOWARDS",
        options: &[],
    },
    Menu {
        id: 17,
        name: "GOTO",
        options: &[],
    },
    Menu {
        id: 18,
        name: "GLIDE_TO",
        options: &[],
    },
    Menu {
        id: 19,
        name: "OF_OBJECT",
        options: &[],
    },
    Menu {
        id: 20,
        name: "COSTUME",
        options: &[],
    },
    Menu {
        id: 21,
        name: "BACKDROP",
        options: &[],
    },
    Menu {
        id: 22,
        name: "SOUND",
        options: &[],
    },
    Menu {
        id: 23,
        name: "BROADCAST",
        options: &[],
    },
    Menu {
        id: 24,
        name: "CLONE_OF",
        options: &[],
    },
];

/// Blocks carrying an INLINE dropdown: `(block type, field name, menu id)`.
///
/// The field name is the harvested `field_dropdown.name`; `control_stop`'s is
/// the `appendField(…, 'STOP_OPTION')` name.
pub const MENU_FIELDS: &[(&str, &str, u8)] = &[
    ("event_whenkeypressed", "KEY_OPTION", 1),
    ("looks_changeeffectby", "EFFECT", 2),
    ("looks_seteffectto", "EFFECT", 2),
    ("sound_changeeffectby", "EFFECT", 3),
    ("sound_seteffectto", "EFFECT", 3),
    ("looks_gotofrontback", "FRONT_BACK", 4),
    ("looks_goforwardbackwardlayers", "FORWARD_BACKWARD", 5),
    ("looks_costumenumbername", "NUMBER_NAME", 6),
    ("looks_backdropnumbername", "NUMBER_NAME", 6),
    ("motion_setrotationstyle", "STYLE", 7),
    ("motion_align_scene", "ALIGNMENT", 8),
    ("sensing_setdragmode", "DRAG_MODE", 9),
    ("sensing_current", "CURRENTMENU", 10),
    ("event_whengreaterthan", "WHENGREATERTHANMENU", 11),
    ("control_stop", "STOP_OPTION", 12),
];

/// Menu SHADOW blocks — a dropdown that arrives as a nested reporter:
/// `(block type, field name, menu id)`.
///
/// Where scratch-blocks registers the block with a field, the field name is
/// harvested (`KEY_OPTION`, `TOUCHINGOBJECTMENU`, `BROADCAST_OPTION`). Where it
/// registers the block EMPTY (`= {}`, GUI-filled), the name used is the
/// input the consuming block declares for it — the only name the Apache-2.0
/// source has for that value.
pub const MENU_BLOCKS: &[(&str, &str, u8)] = &[
    ("sensing_keyoptions", "KEY_OPTION", 1),
    ("event_touchingobjectmenu", "TOUCHINGOBJECTMENU", 13),
    ("sensing_touchingobjectmenu", "TOUCHINGOBJECTMENU", 14),
    ("sensing_distancetomenu", "DISTANCETOMENU", 15),
    ("motion_pointtowards_menu", "TOWARDS", 16),
    ("motion_goto_menu", "TO", 17),
    ("motion_glideto_menu", "TO", 18),
    ("sensing_of_object_menu", "OBJECT", 19),
    ("looks_costume", "COSTUME", 20),
    ("looks_backdrops", "BACKDROP", 21),
    ("sound_sounds_menu", "SOUND_MENU", 22),
    ("event_broadcast_menu", "BROADCAST_OPTION", 23),
    ("control_create_clone_of_menu", "CLONE_OPTION", 24),
];

/// Which menu shadow block a consuming block's input expects:
/// `(consumer type, input name, menu block type)`. Read from each consumer's
/// `input_value` names; a toolbox uses it to seat the shadow.
pub const MENU_INPUTS: &[(&str, &str, &str)] = &[
    ("sensing_keypressed", "KEY_OPTION", "sensing_keyoptions"),
    (
        "event_whentouchingobject",
        "TOUCHINGOBJECTMENU",
        "event_touchingobjectmenu",
    ),
    (
        "sensing_touchingobject",
        "TOUCHINGOBJECTMENU",
        "sensing_touchingobjectmenu",
    ),
    (
        "sensing_distanceto",
        "DISTANCETOMENU",
        "sensing_distancetomenu",
    ),
    ("motion_pointtowards", "TOWARDS", "motion_pointtowards_menu"),
    ("motion_goto", "TO", "motion_goto_menu"),
    ("motion_glideto", "TO", "motion_glideto_menu"),
    ("sensing_of", "OBJECT", "sensing_of_object_menu"),
    ("looks_switchcostumeto", "COSTUME", "looks_costume"),
    ("looks_switchbackdropto", "BACKDROP", "looks_backdrops"),
    ("sound_play", "SOUND_MENU", "sound_sounds_menu"),
    ("sound_playuntildone", "SOUND_MENU", "sound_sounds_menu"),
    ("event_broadcast", "BROADCAST_INPUT", "event_broadcast_menu"),
    (
        "event_broadcastandwait",
        "BROADCAST_INPUT",
        "event_broadcast_menu",
    ),
    (
        "control_create_clone_of",
        "CLONE_OPTION",
        "control_create_clone_of_menu",
    ),
];

/// The menu with this codebook id.
#[must_use]
pub fn menu_by_id(id: u8) -> Option<&'static Menu> {
    SCRATCH_MENUS.iter().find(|m| m.id == id)
}

/// The menu a block's field reads from — inline dropdowns AND shadow blocks.
#[must_use]
pub fn menu_for_field(ty: &str, field: &str) -> Option<&'static Menu> {
    MENU_FIELDS
        .iter()
        .chain(MENU_BLOCKS.iter())
        .find(|&&(t, f, _)| t == ty && f == field)
        .and_then(|&(_, _, id)| menu_by_id(id))
}

/// The one dropdown a block carries, if any: `(field name, menu)`.
#[must_use]
pub fn menu_for_block(ty: &str) -> Option<(&'static str, &'static Menu)> {
    MENU_FIELDS
        .iter()
        .chain(MENU_BLOCKS.iter())
        .find(|&&(t, ..)| t == ty)
        .and_then(|&(_, f, id)| menu_by_id(id).map(|m| (f, m)))
}

/// Whether `ty` is a menu shadow block — a value, never offered as a tile.
#[must_use]
pub fn is_menu_block(ty: &str) -> bool {
    MENU_BLOCKS.iter().any(|&(t, ..)| t == ty)
}

/// The static index of an option: source position + 1. `None` for a code
/// outside the static prefix — a dynamic entry a project must intern itself,
/// or a typo; either way not guessed.
#[must_use]
pub fn encode(menu: &Menu, code: &str) -> Option<u8> {
    let pos = menu.options.iter().position(|o| *o == code)?;
    u8::try_from(pos + 1).ok()
}

/// The inverse of [`encode`] over the static prefix. `0` is the zero-fallback
/// and decodes to nothing.
#[must_use]
pub fn decode(menu: &Menu, byte: u8) -> Option<&'static str> {
    if byte == 0 {
        return None;
    }
    menu.options.get(usize::from(byte) - 1).copied()
}

/// The declaration a palette answers for a function whose immediate is this
/// menu's index.
#[must_use]
pub const fn value_codebook(menu: &Menu) -> ValueCodebook {
    ValueCodebook {
        id: menu.id,
        name: menu.name,
    }
}

/// FNV-1a over the code, for the options wider than one facet
/// (`"other scripts in sprite"`, `"not draggable"`). The index — not the
/// payload — is the option's identity; the payload is what a legend prints.
#[must_use]
pub fn digest(code: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in code.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// A builder pre-loaded with the menu's static prefix, so index `1..=n`
/// matches [`encode`] and a project can intern its dynamic tail after.
///
/// `utf8_classid` names the reading of an option that fits a facet;
/// `digest_classid` the reading of a wider one (its FNV-1a digest, LE).
///
/// # Errors
///
/// [`PoolError::Full`] cannot occur for the static prefixes (the widest is
/// 42); propagated for completeness.
pub fn builder(
    menu: &Menu,
    utf8_classid: u32,
    digest_classid: u32,
) -> Result<BasinCodebookBuilder, PoolError> {
    let mut b = BasinCodebookBuilder::new(value_codebook(menu));
    for (i, code) in menu.options.iter().enumerate() {
        let idx = if code.len() <= CONSTANT_BYTES {
            b.intern(utf8_classid, code.as_bytes())?
        } else {
            b.intern(digest_classid, &digest(code).to_le_bytes())?
        };
        // The table's index and this module's static index are ONE number.
        // Content-addressed interning could merge two equal codes; the
        // harvested sets have none, and this keeps that a checked fact.
        debug_assert_eq!(usize::from(idx), i + 1, "{}: duplicate option", menu.name);
    }
    Ok(b)
}

/// Every menu sealed with its static prefix — the basin a project starts
/// from before interning its own sprite / costume / sound names.
///
/// # Panics
///
/// Never for the harvested tables; the assertions are the falsifiers a
/// changed table would trip.
#[must_use]
pub fn basin_codebooks(utf8_classid: u32, digest_classid: u32) -> BasinCodebooks {
    let mut basin = BasinCodebooks::new();
    for m in SCRATCH_MENUS {
        let book: BasinCodebook = builder(m, utf8_classid, digest_classid)
            .expect("static prefixes are far below capacity")
            .seal();
        basin
            .plug(book)
            .expect("menu ids are unique; pinned by this module's tests");
    }
    basin
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_loco::pool::placeholder::CONST_UTF8_INLINE;

    fn basin() -> BasinCodebooks {
        basin_codebooks(CONST_UTF8_INLINE, PLACEHOLDER_DIGEST_CLASSID)
    }

    #[test]
    fn ids_are_dense_from_one_and_every_field_names_a_real_menu() {
        for (i, m) in SCRATCH_MENUS.iter().enumerate() {
            assert_eq!(usize::from(m.id), i + 1, "{} is out of order", m.name);
        }
        for &(t, f, id) in MENU_FIELDS.iter().chain(MENU_BLOCKS.iter()) {
            assert!(
                menu_by_id(id).is_some(),
                "{t}.{f} names menu {id}, which does not exist"
            );
        }
        // A block carries at most ONE dropdown, which is what lets the raise
        // read `values[refs]` without a per-block field order.
        let mut seen: Vec<&str> = Vec::new();
        for &(t, ..) in MENU_FIELDS.iter().chain(MENU_BLOCKS.iter()) {
            assert!(!seen.contains(&t), "{t} carries two dropdowns");
            seen.push(t);
        }
        // Every shadow block is seated by some consumer, and vice versa.
        for &(_, _, mb) in MENU_INPUTS {
            assert!(is_menu_block(mb), "{mb} is seated but not a menu block");
        }
        for &(mb, ..) in MENU_BLOCKS {
            assert!(
                MENU_INPUTS.iter().any(|&(_, _, b)| b == mb),
                "{mb} is a menu block no consumer seats"
            );
        }
    }

    #[test]
    fn encode_and_decode_are_inverse_and_zero_is_never_an_option() {
        let keys = menu_by_id(1).unwrap();
        assert_eq!(keys.options.len(), 42);
        assert_eq!(encode(keys, "space"), Some(1));
        assert_eq!(encode(keys, "up arrow"), Some(2));
        assert_eq!(encode(keys, "9"), Some(42));
        assert_eq!(decode(keys, 2), Some("up arrow"));
        assert_eq!(decode(keys, 0), None, "0 is the zero-fallback");
        assert_eq!(decode(keys, 43), None);
        // Anti-vacuity: two options must not share a byte.
        assert_ne!(encode(keys, "a"), encode(keys, "b"));
        // A code outside the static prefix is refused, not guessed.
        assert_eq!(encode(keys, "Space"), None);
        assert_eq!(
            encode(menu_by_id(17).unwrap(), "Sprite1"),
            None,
            "dynamic tail is the project's"
        );
        for m in SCRATCH_MENUS {
            for (i, o) in m.options.iter().enumerate() {
                let b = encode(m, o).unwrap();
                assert_eq!(usize::from(b), i + 1);
                assert_eq!(decode(m, b), Some(*o));
            }
        }
    }

    #[test]
    fn the_sealed_basin_agrees_with_the_static_index_byte_for_byte() {
        let basin = basin();
        assert_eq!(basin.len(), SCRATCH_MENUS.len());
        for m in SCRATCH_MENUS {
            let book = basin.get(m.id).expect("plugged");
            assert_eq!(book.len(), m.options.len());
            for o in m.options {
                let idx = encode(m, o).unwrap();
                let entry = basin.resolve(value_codebook(m), idx).expect("resolves");
                if o.len() <= CONSTANT_BYTES {
                    assert_eq!(entry.classid, CONST_UTF8_INLINE);
                    assert_eq!(&entry.bytes[..o.len()], o.as_bytes(), "{}: {o}", m.name);
                } else {
                    assert_eq!(entry.classid, PLACEHOLDER_DIGEST_CLASSID);
                    assert_eq!(&entry.bytes[..8], &digest(o).to_le_bytes());
                }
            }
        }
        // The wide-option arm is actually exercised: at least one harvested
        // code exceeds a facet, or the digest path is dead code.
        let wide = SCRATCH_MENUS
            .iter()
            .flat_map(|m| m.options.iter())
            .filter(|o| o.len() > CONSTANT_BYTES)
            .count();
        assert!(wide >= 1, "no wide option; the digest arm is untested");
        assert!(wide < 4, "wide options multiplied; re-check the harvest");
    }

    #[test]
    fn a_project_extends_a_dynamic_menu_without_disturbing_the_static_prefix() {
        let goto = menu_by_id(17).unwrap();
        let mut b = builder(goto, CONST_UTF8_INLINE, PLACEHOLDER_DIGEST_CLASSID).unwrap();
        assert!(b.is_empty(), "GOTO is registered empty in scratch-blocks");
        let ball = b.intern(CONST_UTF8_INLINE, b"Ball").unwrap();
        let paddle = b.intern(CONST_UTF8_INLINE, b"Paddle").unwrap();
        assert_eq!((ball, paddle), (1, 2));
        let book = b.seal();
        assert_eq!(&book.resolve(2).unwrap().bytes[..6], b"Paddle");

        // And on a STATIC menu the tail comes AFTER the prefix, so the
        // harvested indices never move.
        let keys = menu_by_id(1).unwrap();
        let mut k = builder(keys, CONST_UTF8_INLINE, PLACEHOLDER_DIGEST_CLASSID).unwrap();
        let extra = k.intern(CONST_UTF8_INLINE, b"enter").unwrap();
        assert_eq!(extra, 43);
        assert_eq!(encode(keys, "space"), Some(1));
    }
}
