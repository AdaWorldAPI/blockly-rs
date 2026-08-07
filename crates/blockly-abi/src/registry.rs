//! The plug — this consumer's ENTIRE registration surface.
//!
//! One boot-time registry, one `plug_into` call, no consumer-side "this node
//! must be Blockly" branch anywhere.
//!
//! # Why this replaced a capability-registry plug
//!
//! An earlier draft registered against `ogar_vocab::capability_registry`,
//! which joins a plugged classid against `class_ids::ALL` — and to make that
//! join resolve it minted `block_function`/`block_inventory` into the shared
//! codebook. That was wrong twice over (OGAR #255):
//!
//! 1. `class_ids::ALL` is mirrored into `lance_graph_contract::ogar_codebook`
//!    under a **compile-time** count fuse, so minting a frontend's palette
//!    there silently makes it a lance-graph change. A block editor's codebook
//!    is not lance-graph's concern.
//! 2. `0x1701`/`0x1702` were never blocks-owned. They are described purely in
//!    `ogar-loco`'s vocabulary — [`FunctionBody`](ogar_loco::LocoConcept),
//!    `LaneShape`, the value slab — the node shapes EVERY sibling vocabulary
//!    rides (thinking-orchestration templates included). `0x17` belongs to the
//!    substrate; this crate is a consumer seated at `0x1717`.
//!
//! The real mechanism is [`VocabularyRegistry`]: the palette plugs itself in
//! under its own content concept, and the shared codebook is never touched.
//! `0x17XX` keeps **zero** codebook rows — that is precisely what makes the
//! palette plug-and-play rather than canon.
//!
//! # What the classid selects
//!
//! A stored node's classid resolves to the vocabulary that reads its call
//! bytes — not to the node's shape:
//!
//! | classid | owner | meaning |
//! |---|---|---|
//! | `0x1701` / `0x1702` | `ogar-loco` | the node SHAPES — function body, inventory |
//! | `0x1717` | this palette | WHICH vocabulary resolves the call bytes |
//!
//! So a Blockly body and an elixir-shaped template body are the same
//! `0x1701` shape; only the palette differs. That is the whole point of the
//! split, and the reason a consumer never branches on "is this Blockly".

use ogar_loco::registry::{RegistryError, VocabularyRegistry};

/// Build the boot-time registry with this palette plugged in.
///
/// A consumer that hosts more than one vocabulary calls each crate's
/// `plug_into` on the SAME registry; every stored function node then resolves
/// through [`VocabularyRegistry::resolve_classid`] with no per-frontend
/// branch. This helper is the single-vocabulary case.
///
/// # Errors
///
/// [`RegistryError::ConceptTaken`] if something already claimed the Blocks
/// content concept — refused loudly rather than silently overwritten.
pub fn registry() -> Result<VocabularyRegistry, RegistryError> {
    let mut registry = VocabularyRegistry::new();
    ogar_blockly::plug_into(&mut registry)?;
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_palette_plugs_in_and_resolves_by_classid() {
        let registry = registry().expect("the palette plugs into an empty registry");
        assert_eq!(registry.len(), 1);

        // The render classid a stored node actually carries — canon-high
        // concept over an app prefix — resolves to this palette's table.
        let stored = ogar_blockly::BlockConcept::Palette.render_classid(0x1000);
        let table = registry
            .resolve_classid(stored)
            .expect("a stored Blockly node resolves to the plugged palette");

        // …and the table it hands back is the real one: the shared core
        // answers through it, which is what a caller reads call bytes with.
        assert_eq!(table.stack_arity(ogar_loco::FnIndex::ADD), Some(2));
        assert_eq!(table.body_refs(ogar_loco::FnIndex::IF_ELSE), 2);
    }

    #[test]
    fn an_unplugged_classid_resolves_to_nothing() {
        // The silence half: resolution is not "yes to everything". A node
        // under a concept nobody plugged has no vocabulary, and the registry
        // says so instead of guessing the only table it happens to hold.
        let registry = registry().unwrap();
        let unplugged = ((0x1718u32) << 16) | 0x1000;
        assert!(registry.resolve_classid(unplugged).is_none());
    }

    #[test]
    fn plugging_the_same_palette_twice_is_refused_not_overwritten() {
        // Two vocabularies claiming one concept is a real collision — the
        // registry must bang rather than let the second silently win.
        let mut registry = registry().unwrap();
        assert!(matches!(
            ogar_blockly::plug_into(&mut registry),
            Err(RegistryError::ConceptTaken { .. })
        ));
    }

    #[test]
    fn the_palette_never_claims_the_substrates_node_shapes() {
        // The ownership line this module exists to hold: 0x1701/0x1702 are
        // ogar-loco's. If this palette ever plugged one of them, a block
        // editor would own the shape every vocabulary shares.
        let registry = registry().unwrap();
        let plugged: Vec<u16> = registry.concepts().collect();
        assert_eq!(plugged, vec![0x1717]);
        assert!(!plugged.contains(&ogar_loco::LocoConcept::FunctionBody.concept_id()));
        assert!(!plugged.contains(&ogar_loco::LocoConcept::Inventory.concept_id()));
    }
}
