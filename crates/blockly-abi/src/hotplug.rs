//! The hot-plug registration — this consumer's ENTIRE registration surface.
//!
//! One [`HotPlug`] const + one activation test. No plug crate, no per-capability
//! registration type, no pins to monitor: *"wenn's knallt, dann einmal."*
//!
//! # The three roles
//!
//! | Role | Home |
//! |---|---|
//! | **SOCKET** (agnostic, zero-dep) | [`lance_graph_contract::hotplug`] — [`HotPlug`], `Activation`, `ActivationDrift`, `CapabilityAuthority` |
//! | **AUTHORITY** (declares + resolves) | `ogar_vocab::blocks_actions` + `ogar_vocab::capability_registry` |
//! | **CONSUMER** | this module + the executor arms it names |
//!
//! Everything links into ONE binary. Nothing serializes; the check below is a
//! test in this crate's own suite, run against the real OGAR tables.
//!
//! # The classid is the join key
//!
//! This crate says *"`block_function` is hot"*; the authority answers with the
//! vocab rows AND every capability whose subject is that classid. Both
//! directions are checked, so the plug fails if the authority declares a
//! capability this crate has no arm for (`Uncovered`) **or** if this crate
//! claims one the authority never declared (`Undeclared`). A one-way check
//! would let either half drift silently.
//!
//! # Why `block_inventory` is NOT plugged
//!
//! It is minted and addressable, but binds no capability: a registry read
//! never touches a body, so no arm here implements one. Plugging it would
//! (correctly) fail with `NoCapabilitiesFor(0x1702)` — see the authority's own
//! `the_inventory_concept_is_minted_but_binds_nothing`.

use lance_graph_contract::hotplug::HotPlug;

/// The capabilities this crate's executor actually implements — each name maps
/// to a real public function, and the authority's table declares exactly this
/// set. Kept in `BLOCKS_ACTION_NAMES` order so a reader can diff the two lists
/// by eye.
///
/// | capability | arm |
/// |---|---|
/// | `lower_script` | [`crate::lower_script`] |
/// | `raise_calls` | [`crate::raise_calls`] |
/// | `render_text` | [`crate::projection::render_text`] |
/// | `parse_text` | [`crate::projection::parse_text`] |
/// | `klickweg_address` | [`crate::klickweg::address_of`] |
pub const COVERED_CAPABILITIES: &[&str] = &[
    "lower_script",
    "raise_calls",
    "render_text",
    "parse_text",
    "klickweg_address",
];

/// This consumer's hot-plug declaration — the whole registration surface.
///
/// `classids` is READ from the authority rather than restated: a local literal
/// would be a second source of truth for exactly the value the join uses, and
/// the drift it introduced would be invisible until a stored node failed to
/// resolve.
pub const HOT_PLUG: HotPlug = HotPlug {
    consumer: "blockly-abi",
    classids: ogar_vocab::blocks_actions::BLOCKS_SUBJECT_CLASSIDS,
    covered: COVERED_CAPABILITIES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use ogar_vocab::capability_registry::{HotplugDrift, resolve_hotplug};

    #[test]
    fn the_plug_activates_against_the_authoritative_ogar_tables() {
        // THE test. If OGAR adds a capability on `block_function`, or renames
        // one, or drops this crate from the expected executors, this bangs
        // once — here, in this crate's own suite, at test time.
        let (concepts, capabilities) =
            resolve_hotplug(HOT_PLUG.consumer, HOT_PLUG.classids, HOT_PLUG.covered)
                .expect("hot-plug drifted from the authoritative OGAR tables");

        assert_eq!(
            capabilities.len(),
            COVERED_CAPABILITIES.len(),
            "authority and executor disagree on capability count"
        );
        let names: Vec<&str> = concepts.iter().map(|&(n, _)| n).collect();
        assert_eq!(names, vec!["block_function"]);
    }

    #[test]
    fn the_plugged_classid_is_the_one_this_crate_actually_stores_nodes_under() {
        // The join key must be the SAME id `BlockConcept::Content` renders
        // into a stored node's classid — otherwise the activation above proves
        // something true about an id this crate never writes.
        assert_eq!(
            HOT_PLUG.classids,
            &[ogar_blockly::BlockConcept::Content.concept_id()]
        );
        // …and the render classid keeps that concept in its hi u16 under any
        // app prefix (canon-high), which is what the port resolves on.
        let rendered = ogar_blockly::BlockConcept::Content.render_classid(0xBEEF);
        assert_eq!((rendered >> 16) as u16, HOT_PLUG.classids[0]);
    }

    #[test]
    fn the_plug_bangs_on_drift_in_either_direction() {
        // Can-fire halves, so the activation above is not "the port says yes
        // to everything". A silent-only test would pass against a port that
        // accepted anything.
        assert!(matches!(
            resolve_hotplug("some-other-crate", HOT_PLUG.classids, HOT_PLUG.covered),
            Err(HotplugDrift::UnexpectedConsumer(_))
        ));
        // Declared-but-uncovered: this crate loses an arm.
        assert!(matches!(
            resolve_hotplug(HOT_PLUG.consumer, HOT_PLUG.classids, &["lower_script"]),
            Err(HotplugDrift::Uncovered(_))
        ));
        // Covered-but-undeclared: this crate claims surface OGAR never declared.
        let mut over = COVERED_CAPABILITIES.to_vec();
        over.push("compile_to_wasm");
        assert!(matches!(
            resolve_hotplug(HOT_PLUG.consumer, HOT_PLUG.classids, &over),
            Err(HotplugDrift::Undeclared(_))
        ));
    }

    #[test]
    fn the_socket_accepts_this_plug_through_the_agnostic_trait() {
        // The plug is a `lance_graph_contract::hotplug::HotPlug`, so it is
        // consumable by ANY `CapabilityAuthority` — not welded to OGAR's
        // concrete `resolve_hotplug`. Proven by resolving through a
        // `dyn CapabilityAuthority` rather than the free function.
        use lance_graph_contract::hotplug::{Activation, ActivationDrift, CapabilityAuthority};

        struct OgarAuthority;
        impl CapabilityAuthority for OgarAuthority {
            fn activate(&self, plug: &HotPlug) -> Result<Activation, ActivationDrift> {
                match resolve_hotplug(plug.consumer, plug.classids, plug.covered) {
                    Ok((concepts, capabilities)) => Ok(Activation {
                        concepts: concepts
                            .into_iter()
                            .map(|(n, id)| (n.to_string(), id))
                            .collect(),
                        capabilities,
                    }),
                    Err(HotplugDrift::UnknownClassid(id)) => {
                        Err(ActivationDrift::UnknownClassid(id))
                    }
                    Err(HotplugDrift::NoCapabilitiesFor(id)) => {
                        Err(ActivationDrift::NoCapabilitiesFor(id))
                    }
                    Err(HotplugDrift::UnexpectedConsumer(c)) => {
                        Err(ActivationDrift::UnexpectedConsumer(c))
                    }
                    Err(HotplugDrift::Uncovered(c)) => Err(ActivationDrift::Uncovered(c)),
                    Err(HotplugDrift::Undeclared(c)) => Err(ActivationDrift::Undeclared(c)),
                }
            }
        }

        let authority: &dyn CapabilityAuthority = &OgarAuthority;
        let activation = authority.activate(&HOT_PLUG).expect("socket activation");
        assert_eq!(activation.concepts.len(), 1);
        assert_eq!(activation.capabilities.len(), COVERED_CAPABILITIES.len());
    }
}
