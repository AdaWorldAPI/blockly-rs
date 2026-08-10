//! This repo's palette — the vocabulary that reads a stored body's call bytes.
//!
//! Declared HERE because this repo *is* the Blockly consumer. The substrate
//! (`ogar-loco`) supplies the call encoding, the lane carvings, the node
//! layout and the shared computational core; what a *frontend* adds is a
//! palette — which bytes above the shared floor mean what — and that is the
//! consumer's own business, not the substrate's.
//!
//! # Why this is not an OGAR crate any more
//!
//! It was: OGAR hosted a Blockly crate supplying exactly a concept id, an
//! empty [`Vocabulary`] impl and a plug helper, and this repo depended back on
//! OGAR to obtain its own palette. Worse, `ogar-vocab`'s Blocks table named
//! this repo's executor crate in a `const` — the substrate knowing its
//! consumer by name, in another repository.
//!
//! [`VocabularyRegistry`] exists so that inversion is unnecessary. A consumer
//! declares its own slot in the reserved consumer range and plugs it at boot;
//! the substrate never learns that Blockly exists. The registry's
//! [`ConceptTaken`](RegistryError::ConceptTaken) is what keeps two frontends
//! from claiming one slot — enforcement at the plug, not a name in a table.
//!
//! Consequence for the build: this repo depends on `ogar-loco` alone.
//!
//! # The slot
//!
//! `0x17` is the substrate's domain. `0x1701`/`0x1702` are `ogar-loco`'s node
//! shapes (function body, inventory) and `0x1703`–`0x1716` is its reserved
//! headroom; consumers are seated from `0x1717` up, so the substrate keeps
//! contiguous room beneath them. This palette takes the first consumer slot.
//!
//! A classid selects **which vocabulary reads the call bytes**, never the
//! node's shape — a Blockly body and an elixir-shaped template body are the
//! same `0x1701` shape with different palettes.

use ogar_loco::registry::{RegistryError, VocabularyRegistry};
use ogar_loco::{FnIndex, Vocabulary};

/// This palette's content concept — the first consumer slot in the
/// substrate's `0x17` domain (see the module docs for the seating rule).
///
/// Declared here and nowhere else. Uniqueness is enforced at plug time by
/// [`VocabularyRegistry`], not by a central table that would have to know
/// every frontend's name.
pub const PALETTE_CONCEPT: u16 = 0x1717;

/// First byte reserved for **device-specific** families — the sprite/stage
/// vocabulary (motion, looks, sound, events, sensing) a Scratch-style
/// frontend has and a general block editor does not.
///
/// This is this palette's reading of the substrate's domain floor: below it
/// is the shared computational core, whose tables live once in `ogar-loco`;
/// at or above it is this palette's own range. Currently **reserved, not
/// allocated** — 108 device opcodes were measured in the Apache-2.0
/// `scratch-blocks` definitions and mint when a consumer needs them.
pub const DEVICE_FAMILY_FLOOR: u8 = ogar_loco::DOMAIN_FLOOR;

/// The full V3 render classid under an app prefix — canon-high
/// `(concept << 16) | app_prefix`. The concept half is what
/// [`VocabularyRegistry::resolve_classid`] routes on; the app prefix chooses
/// a render skin and never participates in vocabulary routing.
#[must_use]
pub const fn render_classid(app_prefix: u16) -> u32 {
    ((PALETTE_CONCEPT as u32) << 16) | (app_prefix as u32)
}

/// The Blockly/Scratch palette as an `ogar-loco` [`Vocabulary`].
///
/// Every operation this palette has *allocated* sits below the floor — in the
/// shared computational core, whose tables live in the substrate — so the
/// domain hooks answer nothing yet. That is honest rather than lazy: the
/// device families above [`DEVICE_FAMILY_FLOOR`] are reserved, not allocated,
/// and when they mint their arity tables land here (and only here — the
/// substrate never learns device vocabulary).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlocklyPalette;

impl Vocabulary for BlocklyPalette {
    fn domain_stack_arity(&self, _f: FnIndex) -> Option<u8> {
        // No device family is minted yet; an above-floor byte is refused,
        // never guessed.
        None
    }

    fn domain_body_refs(&self, _f: FnIndex) -> u8 {
        0
    }
}

/// Plug this palette into a registry under [`PALETTE_CONCEPT`].
///
/// # Errors
///
/// [`RegistryError::ConceptTaken`] if something already claimed the slot —
/// refused loudly rather than silently overwritten.
pub fn plug_into(registry: &mut VocabularyRegistry) -> Result<(), RegistryError> {
    let checked = ogar_loco::vocabulary::conformance::validate(BlocklyPalette)
        .expect("BlocklyPalette conforms; pinned by this module's tests");
    registry.plug(PALETTE_CONCEPT, &checked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_loco::vocabulary::{conformance, shared_core};

    #[test]
    fn the_palette_conforms_to_the_sharing_discipline() {
        // The mechanical gate every vocabulary must pass: shared-core bytes
        // answer from the core, the domain range refuses what is not minted,
        // and no reported shape can truncate a call's own body references.
        assert_eq!(conformance::check(&BlocklyPalette), Ok(()));

        // Spot-check the routing this palette relies on: control flow and
        // expressions answer from the shared core THROUGH the vocabulary.
        let v = BlocklyPalette;
        assert_eq!(v.stack_arity(FnIndex::REPEAT), Some(1));
        assert_eq!(v.body_refs(FnIndex::IF_ELSE), 2);
        assert_eq!(v.stack_arity(FnIndex::ADD), Some(2));
        // …and an unminted device byte is refused, not guessed.
        assert_eq!(v.stack_arity(FnIndex(DEVICE_FAMILY_FLOOR)), None);
    }

    #[test]
    fn the_slot_sits_above_the_substrates_reserved_range() {
        // The seating rule, asserted rather than trusted: consumers start at
        // 0x1717 so the substrate keeps 0x1701-0x1716 contiguous beneath.
        assert_eq!(PALETTE_CONCEPT, 0x1717);
        assert!(PALETTE_CONCEPT > ogar_loco::LocoConcept::Inventory.concept_id());
        assert!(PALETTE_CONCEPT > ogar_loco::LocoConcept::FunctionBody.concept_id());
        // The domain byte is the substrate's, shared by both.
        assert_eq!(PALETTE_CONCEPT >> 8, 0x17);
    }

    #[test]
    fn the_render_classid_keeps_the_concept_in_the_canon_half() {
        let id = render_classid(0x1000);
        assert_eq!(id, 0x1717_1000);
        assert_eq!((id >> 16) as u16, PALETTE_CONCEPT);
        assert_eq!((id & 0xFFFF) as u16, 0x1000);
    }

    #[test]
    fn the_shared_core_and_this_palette_partition_the_byte_space() {
        // Can-fire AND can-stay-silent on the same predicate: a classifier
        // answering the same way for everything carries no information.
        assert!(FnIndex::LT.is_shared_core());
        assert!(!FnIndex::LT.is_domain_specific());

        let device = FnIndex(DEVICE_FAMILY_FLOOR);
        assert!(device.is_domain_specific());
        assert!(!device.is_shared_core());

        // NOP is not an operation at all — neither bucket claims it.
        assert!(!FnIndex::NOP.is_shared_core());
        assert!(!FnIndex::NOP.is_domain_specific());

        // And the core's own tables cover the control range this palette
        // lowers through — anchored so a substrate regression is caught here.
        assert_eq!(shared_core::stack_arity(FnIndex::REPEAT), Some(1));
        assert_eq!(shared_core::body_refs(FnIndex::REPEAT), 1);
    }
}
