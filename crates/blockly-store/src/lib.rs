//! The storage layer: a cast program's function nodes ARE lance-graph V3 rows.
//!
//! # The convergence this crate does not invent
//!
//! `ogar-loco` stores a function as 512 bytes — key at `0..16`, a reserved
//! slot at `16..32` that is written as zeroes and never reclaimed, and the
//! value slab at `32..512`. `lance-graph-contract`'s [`NodeRow`] is the V3
//! canon row — `key(16) | edges(16) | value(480)`, `#[repr(C, align(64))]`,
//! locked at 512 by `const _` size asserts. **These are the same bytes.**
//!
//! So this crate is a binding, not a port. It does not translate a blockly
//! program into a storage format; it names the format the program was
//! already in, and supplies the two things `ogar-loco` deliberately leaves
//! out — its node module says so in as many words: *"this crate does not
//! mint GUIDs: the canonical layout is the substrate's"*.
//!
//! 1. **A minted key.** [`mint_key`] routes through
//!    [`NodeGuid::mint_for`], the substrate's own minter. Three sites in
//!    this workspace used to spell a key by hand
//!    (`k[0..4].copy_from_slice(...)`, `k[10..16] = ...`), which is bit
//!    math on a documented byte layout — the thing the LE contract's
//!    §1 rule 4 forbids, and which wrote the V1 `family:identity` u24 tail
//!    that is closed to new mints.
//! 2. **An envelope over the array.** [`ProgramRows::packet`] hands the
//!    rows to [`NodeRowPacket`], the zero-copy [`SoaEnvelope`] whose whole
//!    purpose is that "Lance's columnar I/O reads it directly". Nothing is
//!    serialized on the way to storage; `to_le_bytes` IS the wire format
//!    and the stored format at once.
//!
//! # The V3 tail comes from the seat's owner, never from the canon registry
//!
//! An earlier cut of this crate composed its own classid over an invented
//! placeholder prefix. That classid was in no registry, so
//! `classid_read_mode` fell through to its conservative default and every
//! key minted a **V1** `family:identity` u24 tail — the shape the canon
//! calls forbidden for new units. The mechanism was right and the address
//! was wrong, which is the failure mode that looks like success: keys
//! appeared, were distinct, round-tripped, and were legacy.
//!
//! The cut after that fixed the address the wrong way — by registering
//! `CLASSID_BLOCKS_V3` in the substrate's `BUILTIN_READ_MODES`. Operator
//! ruling **D-BLOCKS-HOTPLUG-1** withdrew it (lance-graph #1207):
//! *"Blockly is a hot-plugged consumer, not a canon builtin. `0x1717` is
//! authority-owned. `BUILTIN_READ_MODES` must remain unchanged by
//! Blockly."* Registering a per-frontend seat there would make adding a
//! frontend an edit to the substrate plus a recompile — central lockstep,
//! rebuilt one layer up.
//!
//! `blockly-abi` had already reached that ruling from this side, months
//! earlier and in its own words: *"`0x17XX` keeps **zero** codebook rows —
//! that is precisely what makes the palette plug-and-play rather than
//! canon"* (`registry.rs`). So the two halves are:
//!
//! - **The address is `blockly-abi`'s.** [`CLASSID`] is
//!   [`render_classid`](blockly_abi::palette::render_classid)`(0x1000)` —
//!   the palette that declares `0x1717` composes its own address. Reaching
//!   upstream for a canon constant was the second spelling, not this one.
//! - **The reading is inherited with the persistence.** A cast program's
//!   nodes ARE lance-graph V3 rows, so the moment this crate persists them
//!   it is on lance-graph and the V3 substrate comes with it. [`READ_MODE`]
//!   states this seat's reading once; [`mint_key`] passes its
//!   `tail_variant` to the substrate's own minter.
//!
//! What must NOT happen is asking `classid_read_mode` — the **canon**
//! registry, which by the ruling does not know `0x1717` and answers
//! `DEFAULT` (V1). That call compiles and silently mints legacy keys: the
//! tail is not recorded in the key ([`NodeGuid`] has no tail-variant
//! accessor; `decode` versus `decode_v2` is chosen by classid alone), so
//! V3 bytes would read back as V1 with no error anywhere.
//!
//! # The lance feature
//!
//! [`LanceStore`] (feature `lance`) is the durable half. It is OFF by
//! default: the mapping above is what a reader of this crate needs, and it
//! costs nothing, while `lance` + `lancedb` + `datafusion` is a large tree
//! no lean consumer should link to learn what a row is.
//!
//! **On the version set.** Every requirement floats on its major —
//! `lance = "11"`, `arrow = "58"`, datafusion 54 transitively. An exact
//! `=X.Y.Z` is owed only where an upstream crate demands exact-equals, and
//! the crate that does is `lancedb` (its manifest requires
//! `lance = "=11.0.0"`), which this workspace does not link. Pinning
//! anyway would make our requirement strictly NARROWER than the `^58.0.0` /
//! `^54.0.0` the family itself asks for, and being narrower than the graph
//! is what makes a graph unsatisfiable. Which major was read from lancedb
//! 0.38.0's published manifest rather than assumed.

use blockly_abi::FunctionNode;
use lance_graph_contract::canonical_node::{
    EdgeBlock, EdgeCodecFlavor, NodeGuid, NodeRow, NodeRowPacket, ReadMode, TailVariant,
    ValueSchema,
};
use ogar_loco::node::NODE_BYTES;
use ogar_loco::{FunctionBody, LaneShape, Program};

/// The V3 generation marker used as this seat's app prefix.
///
/// The canon's convention for "V3, no app prefix minted yet". Minting a
/// real prefix for this frontend is still the operator decision this
/// workspace calls M1; when it lands the class gets a sibling classid and
/// this constant moves — the rows do not, because the tail is a reading of
/// the same 16 key bytes either way.
pub const V3_GENERATION_MARKER: u16 = 0x1000;

/// The classid every stored blockly function is minted under.
///
/// Composed by the crate that owns the seat:
/// [`blockly_abi::palette::render_classid`] over [`V3_GENERATION_MARKER`],
/// giving canon-high `0x1717` (the palette concept `blockly-abi` declares
/// "here and nowhere else") over the custom-low marker.
///
/// Not a canon constant, deliberately. `0x1717` is a per-frontend consumer
/// seat in the substrate's `0x17` domain; the substrate must never learn
/// that Blockly exists (see the module docs and `blockly-abi`'s
/// `registry.rs`). Composing it here is the FIRST spelling, not a second
/// one — the canon has no entry to drift from.
pub const CLASSID: u32 = blockly_abi::palette::render_classid(V3_GENERATION_MARKER);

/// How a row addressed under [`CLASSID`] is read.
///
/// This seat's own reading, stated once, because the canon registry must
/// not carry it. It is the same descriptor the hot-plug authority hands
/// back for `0x1717` in [`Activation::read_modes`], which exists precisely
/// so a consumer's reading rides the activation instead of a registry row
/// (lance-graph #1207, D-BLOCKS-HOTPLUG-1).
///
/// - `V3` — the content-blind 4+12 facet. The V1 `family:identity` u24
///   tail is closed to new mints, and a flat u24 carries no rail.
/// - `Bootstrap` — key + edges only. This crate stores a function body in
///   `ogar-loco`'s own value slab; it materialises no cognitive tenants.
/// - `CoarseOnly` — the canon zero-fallback edge carving. Nothing here
///   reads the edge block yet; it is reserved and zeroed.
///
/// [`Activation::read_modes`]: lance_graph_contract::hotplug::Activation::read_modes
pub const READ_MODE: ReadMode = ReadMode {
    tail_variant: TailVariant::V3,
    value_schema: ValueSchema::Bootstrap,
    edge_codec: EdgeCodecFlavor::CoarseOnly,
};

/// Mint the key for function `index` of a program under `classid`.
///
/// Bootstrap-addressed: the three cascade tiers, the leaf and the family
/// are all zero, so `identity` alone discriminates — the canon's documented
/// starting address while routing and basin binding are dormant. Those
/// fields keep their offsets, so minting a real family later wakes them
/// with no layout change (RESERVE, DON'T RECLAIM).
///
/// `index` is the same number a body-reference byte stores, so a key is
/// derivable from a program without a side table.
#[must_use]
pub fn mint_key(classid: u32, index: u16) -> NodeGuid {
    NodeGuid::mint_for(
        READ_MODE.tail_variant,
        classid,
        0,
        0,
        0,
        0,
        0,
        u32::from(index),
    )
}

/// Why a program could not be laid out as rows.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    /// More functions than an index byte can address. A body-reference is
    /// one byte, so a program past this is unaddressable by its own calls
    /// long before it is unstorable — reported rather than truncated.
    TooManyFunctions(usize),
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::TooManyFunctions(n) => {
                write!(f, "program has {n} functions; a body reference is one byte")
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// A program's functions as V3 rows, ready for storage.
///
/// Owns `Vec<NodeRow>` rather than a byte buffer so the array is already
/// the row-strided packet [`NodeRowPacket`] borrows — nothing between the
/// cast and the write reshapes it.
///
/// No `Debug`: `NodeRow` has none upstream, and 512 bytes per row is not a
/// diagnostic anyway. [`ProgramRows::len`] and the minted keys are what a
/// caller actually wants to look at.
pub struct ProgramRows {
    rows: Vec<NodeRow>,
}

impl ProgramRows {
    /// Lay out every function of `program` as a row, minting keys.
    ///
    /// # Errors
    ///
    /// [`StoreError::TooManyFunctions`] past `u8::MAX + 1` functions.
    pub fn from_program(program: &Program, classid: u32) -> Result<Self, StoreError> {
        let n = program.functions.len();
        if n > usize::from(u8::MAX) + 1 {
            return Err(StoreError::TooManyFunctions(n));
        }
        let rows = program
            .functions
            .iter()
            .enumerate()
            .map(|(i, body)| {
                // `i` fits: bounded by the guard above.
                let index = u16::try_from(i).unwrap_or(u16::MAX);
                row_of(mint_key(classid, index), body)
            })
            .collect();
        Ok(Self { rows })
    }

    /// The rows.
    #[must_use]
    pub fn rows(&self) -> &[NodeRow] {
        &self.rows
    }

    /// How many functions the program stored as.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether nothing is stored. Never true for a cast program — a script
    /// always has an entry body — but the lint asks and the answer is cheap.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The zero-copy envelope over the rows, stamped with `cycle`.
    ///
    /// The returned packet borrows; it is deliberately neither `Copy` nor
    /// `Clone` upstream, so a second holder cannot silently acquire a view
    /// of the whole slab.
    #[must_use]
    pub fn packet(&self, cycle: u32) -> NodeRowPacket<'_> {
        NodeRowPacket::new(&self.rows, cycle)
    }

    /// The rows as one contiguous LE byte run — `len() * 512` bytes, in row
    /// order, no header. What a writer hands to storage verbatim.
    #[must_use]
    pub fn as_le_bytes(&self) -> &[u8] {
        // SAFETY: `NodeRow` is `#[repr(C, align(64))]` with a `const _`
        // size assert at exactly 512 and three plain byte-array fields
        // (`NodeGuid([u8; 16])`, `EdgeBlock` of `[u8; 12]` + `[u8; 4]`,
        // `[u8; 480]`), so it has no padding and no invalid bit pattern:
        // every byte of every row is initialized and readable as `u8`. The
        // slice keeps the borrow, so the view cannot outlive the rows. This
        // is the same reinterpretation `NodeRowPacket::as_le_bytes` makes
        // of the same type, and the pinned identity test below compares
        // both against `FunctionNode::to_le_bytes`.
        unsafe {
            core::slice::from_raw_parts(
                self.rows.as_ptr().cast::<u8>(),
                core::mem::size_of_val(self.rows.as_slice()),
            )
        }
    }
}

/// One function as one row: mint the key, let `ogar-loco` write the bytes,
/// then read them back under the canon's field names.
///
/// Going through [`FunctionNode::to_le_bytes`] rather than composing the
/// row field-by-field is the point. The value slab is INTERLEAVED — each
/// 16-byte lane is `classid(4) + payload(12)` and a call's bytes land at
/// `(i / calls_per_lane) * 16 + 4 + ...` — so a hand-built slab would look
/// plausible, read back correctly through the same wrong function, and be
/// wrong on the wire. The substrate owns that arithmetic; this is a
/// re-reading of its output, never a second derivation of it.
fn row_of(key: NodeGuid, body: &FunctionBody) -> NodeRow {
    let bytes = FunctionNode::new(*key.as_bytes(), *body).to_le_bytes();
    let mut value = [0u8; 480];
    value.copy_from_slice(&bytes[NODE_BYTES - 480..]);
    NodeRow {
        key,
        // Zeroed, exactly as `ogar-loco` writes slot 1: "reserve, don't
        // reclaim". A blockly function has no adjacency yet; when it does,
        // the block is already here at its canon offset.
        edges: EdgeBlock {
            in_family: [0; 12],
            out_family: [0; 4],
        },
        value,
    }
}

/// Read a row back as a function body. `shape` comes from the ClassView,
/// never from the row — storing it would be a second source of truth that
/// could disagree with the first.
#[must_use]
pub fn body_of(row: &NodeRow, shape: LaneShape) -> FunctionBody {
    let mut bytes = [0u8; NODE_BYTES];
    bytes[0..16].copy_from_slice(row.key.as_bytes());
    bytes[NODE_BYTES - 480..].copy_from_slice(&row.value);
    FunctionNode::from_le_bytes(&bytes, shape).body
}

#[cfg(feature = "lance")]
mod lance_store;
#[cfg(feature = "lance")]
pub use lance_store::{LanceError, LanceStore};

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_program() -> Program {
        let json = blockly_shim::templates::ALL[0].1;
        let scripts = blockly_shim::from_workspace_json(json).expect("template parses");
        blockly_shim::templates::cast(LaneShape::Pairs, &scripts[0]).expect("template casts")
    }

    /// The whole claim of this crate in one assertion: a row IS the stored
    /// node, byte for byte. Not "equivalent", not "round-trips" — the same
    /// 512 bytes in the same order.
    #[test]
    fn a_row_is_byte_identical_to_the_stored_node() {
        let prog = demo_program();
        let rows = ProgramRows::from_program(&prog, CLASSID).expect("lays out");
        assert!(rows.len() > 1, "the demo must have branches to be a test");

        let bytes = rows.as_le_bytes();
        assert_eq!(bytes.len(), rows.len() * NODE_BYTES);
        for (i, body) in prog.functions.iter().enumerate() {
            let index = u16::try_from(i).expect("bounded");
            let key = mint_key(CLASSID, index);
            let expect = FunctionNode::new(*key.as_bytes(), *body).to_le_bytes();
            assert_eq!(
                &bytes[i * NODE_BYTES..(i + 1) * NODE_BYTES],
                &expect[..],
                "row {i} is not the stored node"
            );
        }
    }

    /// ...and the bytes come back as the program that went in.
    #[test]
    fn a_program_round_trips_through_rows() {
        let prog = demo_program();
        let rows = ProgramRows::from_program(&prog, CLASSID).expect("lays out");
        let back: Vec<FunctionBody> = rows
            .rows()
            .iter()
            .map(|r| body_of(r, LaneShape::Pairs))
            .collect();
        assert_eq!(back, prog.functions);
    }

    /// The envelope is a VIEW of the rows, not a copy of them: the packet's
    /// bytes must be the same memory the array owns.
    ///
    /// Anti-vacuity: a `[0u8; N] == [0u8; N]` comparison would pass for any
    /// implementation, so this asserts pointer identity and that the rows
    /// are not all-zero in the first place.
    #[test]
    fn the_packet_borrows_the_rows_rather_than_materialising_them() {
        use lance_graph_contract::soa_envelope::SoaEnvelope;
        let prog = demo_program();
        let rows = ProgramRows::from_program(&prog, CLASSID).expect("lays out");
        let packet = rows.packet(7);
        assert_eq!(packet.n_rows(), rows.len());
        assert_eq!(packet.cycle(), 7);
        assert_eq!(packet.row_stride(), NODE_BYTES);
        assert!(
            packet.as_le_bytes().iter().any(|&b| b != 0),
            "an all-zero slab would make the identity below vacuous"
        );
        assert_eq!(
            packet.as_le_bytes().as_ptr(),
            rows.as_le_bytes().as_ptr(),
            "the packet copied instead of borrowing"
        );
        packet
            .verify_layout()
            .expect("the canon column table is sound");
    }

    /// Keys are minted through the substrate and are distinct per function.
    ///
    /// Two-sided: distinct identities must differ, and the classid half must
    /// be the SAME for every row (they are one program, one class) — an
    /// implementation that varied the classid would pass a bare
    /// all-different check.
    ///
    /// # Read the tail you minted
    ///
    /// The accessors here are the `_v2` family, because a V3 key stores
    /// `leaf(u16)·family(u16)·identity(u16)` at bytes 10..12/12..14/14..16 —
    /// the same bytes V2 mints, read through the V3 lens. The V1
    /// [`NodeGuid::identity`] reads bytes 13..15 instead, so on a V3 key it
    /// straddles the family/identity boundary and returns a SHIFTED value:
    /// minting identity 1 reads back as 256. This is not a contract bug —
    /// it is I-LEGACY-API-FEATURE-GATED working as designed, and the
    /// contract says so at the accessor: *"different name, different bytes —
    /// no silent semantic swap."*
    ///
    /// It is worth a comment because this test asserted the V1 accessor and
    /// passed for as long as this crate minted V1 keys. It went red the
    /// moment the mint became genuinely V3 — which is the whole point of
    /// the change, and the assertion is what proved the tail actually moved.
    #[test]
    fn every_function_gets_its_own_identity_under_one_classid() {
        let cid = CLASSID;
        let keys: Vec<NodeGuid> = (0..8).map(|i| mint_key(cid, i)).collect();
        for (i, k) in keys.iter().enumerate() {
            let i16 = u16::try_from(i).expect("bounded");
            assert_eq!(k.classid(), cid, "row {i} drifted to another class");
            assert_eq!(k.identity_v2(), i16, "row {i} lost its V3 identity");
            assert_eq!(
                k.family_v2(),
                0,
                "family is dormant until an operator mints one"
            );
            assert_eq!(k.leaf(), 0, "the 4th HHTL tier is dormant too");

            // The V1 accessor on a V3 key is the trap this crate must not
            // fall into again: it reads bytes 13..15 and shifts. Pinned so a
            // future edit cannot quietly reintroduce it and look correct on
            // row 0, where both readings happen to agree.
            if i > 0 {
                assert_ne!(
                    k.identity(),
                    u32::from(i16),
                    "row {i}: the V1 u24 accessor must NOT agree with the V3 tail"
                );
            }
        }
        let mut seen: Vec<[u8; 16]> = keys.iter().map(|k| *k.as_bytes()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), keys.len(), "two functions minted the same key");
    }

    /// The classid is the seat owner's composition, and it still agrees
    /// with the canon's own composer for the canon-high/custom-low split.
    ///
    /// Two separate facts, both load-bearing. The first is the address
    /// itself. The second checks `blockly-abi`'s own composer (it is
    /// dep-free by design and spells canon-high itself) against the
    /// contract's `render_classid` — with prefixes whose halves are NOT
    /// symmetric, so a canon/custom swap cannot pass.
    #[test]
    fn the_classid_is_composed_by_the_seats_owner() {
        use lance_graph_contract::ogar_codebook::{
            ConceptDomain, classid_canon, classid_concept_domain, classid_custom, render_classid,
        };
        assert_eq!(CLASSID, 0x1717_1000);
        assert_eq!(
            classid_canon(CLASSID),
            blockly_abi::palette::PALETTE_CONCEPT
        );
        assert_eq!(classid_custom(CLASSID), 0x1000, "the V3 generation marker");
        assert_eq!(classid_concept_domain(CLASSID), ConceptDomain::Blocks);

        // The palette's own composer, checked rather than trusted.
        for prefix in [0x0000_u16, 0x0001, 0x0005, 0x00FF, 0x1000, 0xFFFF] {
            let ours = blockly_abi::palette::render_classid(prefix);
            assert_eq!(
                ours,
                render_classid(prefix, blockly_abi::palette::PALETTE_CONCEPT)
            );
            assert_eq!(classid_canon(ours), blockly_abi::palette::PALETTE_CONCEPT);
            assert_eq!(classid_custom(ours), prefix);
        }
        // ...and composing the palette over the V3 marker reproduces
        // CLASSID exactly, so the two spellings cannot drift.
        assert_eq!(blockly_abi::palette::render_classid(0x1000), CLASSID);
    }

    /// Keys mint on the **V3** tail — and the canon registry is NOT what
    /// says so.
    ///
    /// Two-sided on the exact thing D-BLOCKS-HOTPLUG-1 changed. The first
    /// half is the point of the whole exercise: a minted key carries the V3
    /// facet, not the legacy u24 tail. The second is the anti-vacuity half
    /// and the more important one — the **canon** registry must still answer
    /// `DEFAULT` for this seat, because `0x1717` is a hot-plugged consumer
    /// slot the substrate does not know.
    ///
    /// Without that half the test would pass equally well if the class were
    /// re-registered upstream, which is precisely the regression the ruling
    /// forbids. With it, a passing run proves the V3 reading came from
    /// [`READ_MODE`] — this seat's owner — and could not have come from the
    /// registry.
    ///
    /// This replaced a version that asserted
    /// `classid_read_mode(CLASSID).tail_variant == V3` and read as success
    /// while depending on a registry row that has since been withdrawn.
    #[test]
    fn keys_mint_on_the_v3_tail_and_the_canon_registry_does_not_know_the_seat() {
        use lance_graph_contract::canonical_node::classid_read_mode;

        assert_eq!(READ_MODE.tail_variant, TailVariant::V3);

        // The canon registry does NOT carry this seat — same classid, and
        // it falls through to the conservative default.
        let canon = classid_read_mode(CLASSID);
        assert_eq!(
            canon,
            ReadMode::DEFAULT,
            "0x1717 is a hot-plugged consumer seat; registering it in \
             BUILTIN_READ_MODES is what D-BLOCKS-HOTPLUG-1 withdrew"
        );
        assert_ne!(
            canon.tail_variant, READ_MODE.tail_variant,
            "if the canon answered V3 too, this test could not tell where \
             the reading came from"
        );

        // ...and mint_key really USES it. Without this the test above is a
        // tautology on a `const`: it would pass with `mint_key` still asking
        // the canon registry, which is exactly the defect. Verified by a
        // disable run — reverting `mint_key` to `classid_read_mode` turns
        // this red.
        let key = mint_key(CLASSID, 7);
        assert_eq!(key.classid(), CLASSID);
        assert_eq!(key.identity_v2(), 7, "the V3 tail carries the identity");
        assert_ne!(
            key.identity(),
            7,
            "a V1-tailed mint would put 7 where the u24 accessor finds it; \
             that it does not is what proves the tail came from READ_MODE"
        );
    }

    #[test]
    fn a_program_past_one_index_byte_is_reported_not_truncated() {
        let body = *demo_program().entry();
        let program = Program {
            functions: vec![body; 300],
        };
        assert_eq!(
            ProgramRows::from_program(&program, CLASSID).err(),
            Some(StoreError::TooManyFunctions(300))
        );
        // ...and the boundary itself is storable, so the guard is a real
        // edge rather than a conservative margin.
        let ok = Program {
            functions: vec![body; 256],
        };
        assert!(ProgramRows::from_program(&ok, CLASSID).is_ok());
    }
}
