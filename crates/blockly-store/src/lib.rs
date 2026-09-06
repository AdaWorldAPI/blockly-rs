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
//! # The V3 tail is reached through the registry, not asked for
//!
//! An earlier cut of this crate composed its own classid over a
//! placeholder app prefix. That classid was in no registry, so
//! [`classid_read_mode`] fell through to its conservative default and
//! every key minted a **V1** `family:identity` u24 tail — the shape the
//! canon calls forbidden for new units. The mechanism was right and the
//! address was wrong, which is the failure mode that looks like success:
//! keys appeared, were distinct, round-tripped, and were legacy.
//!
//! The contract is explicit that there is no way around this — *"there is
//! NO public `new_v3` dispatch — the `tail_variant` registry field IS the
//! mechanism"*. So a consumer reaches V3 only by its classid being
//! registered, which `NodeGuid::CLASSID_BLOCKS_V3` (`0x1717_1000`) now is:
//! canon `0x1717` HIGH — the Blocks domain's per-frontend palette seat,
//! already reserved — and the V3 generation marker `0x1000` in the custom
//! LOW half. The marker replaces the invented placeholder outright: the
//! canon has a convention for "V3, no app prefix minted yet", and using it
//! is strictly less invention than a made-up value.
//!
//! [`mint_key`] is unchanged — it always passed
//! `classid_read_mode(classid).tail_variant` through. Registering the
//! class is what made that answer V3.
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
    EdgeBlock, NodeGuid, NodeRow, NodeRowPacket, classid_read_mode,
};
use ogar_loco::node::NODE_BYTES;
use ogar_loco::{FunctionBody, LaneShape, Program};

/// The classid every stored blockly function is minted under.
///
/// Not composed here. It is [`NodeGuid::CLASSID_BLOCKS_V3`] — the registry
/// entry is the whole point (see the module docs), and a locally composed
/// equivalent would be a second spelling of an address the canon owns, one
/// that resolves to a V3 read mode only by coincidence.
///
/// The custom half is the `0x1000` V3 generation marker rather than an app
/// prefix. Minting a real prefix for this frontend is still the operator
/// decision this workspace calls M1; when it lands, the class gets a
/// sibling classid and this constant moves — the rows do not, because the
/// tail is a reading of the same 16 key bytes either way.
pub const CLASSID: u32 = NodeGuid::CLASSID_BLOCKS_V3;

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
        classid_read_mode(classid).tail_variant,
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
        let rows =
            ProgramRows::from_program(&prog, CLASSID).expect("lays out");
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
        let rows =
            ProgramRows::from_program(&prog, CLASSID).expect("lays out");
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
        let rows =
            ProgramRows::from_program(&prog, CLASSID).expect("lays out");
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
    #[test]
    fn every_function_gets_its_own_identity_under_one_classid() {
        let cid = CLASSID;
        let keys: Vec<NodeGuid> = (0..8).map(|i| mint_key(cid, i)).collect();
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(k.classid(), cid, "row {i} drifted to another class");
            assert_eq!(k.identity(), u32::from(u16::try_from(i).expect("bounded")));
            assert!(
                k.is_unbasined(),
                "family is dormant until an operator mints one"
            );
        }
        let mut seen: Vec<[u8; 16]> = keys.iter().map(|k| *k.as_bytes()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), keys.len(), "two functions minted the same key");
    }

    /// The classid is the canon's own registered address, and the palette's
    /// dep-free composition still agrees with the canon's composer.
    ///
    /// Two separate facts, both load-bearing. The first is why the tail is
    /// V3 at all. The second checks the sanctioned hand-copy
    /// (`blockly-abi` is dep-free by design and spells canon-high itself)
    /// against `render_classid` — with prefixes whose halves are NOT
    /// symmetric, so a canon/custom swap cannot pass.
    #[test]
    fn the_classid_is_the_registered_blocks_v3_address() {
        use lance_graph_contract::ogar_codebook::{
            ConceptDomain, classid_canon, classid_concept_domain, classid_custom, render_classid,
        };
        assert_eq!(CLASSID, 0x1717_1000);
        assert_eq!(classid_canon(CLASSID), blockly_abi::palette::PALETTE_CONCEPT);
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
        // ...and composing the palette over the V3 marker reproduces the
        // registered address exactly, so the two spellings cannot drift.
        assert_eq!(blockly_abi::palette::render_classid(0x1000), CLASSID);
    }

    /// Keys mint on the **V3** tail — the point of the whole exercise.
    ///
    /// This test replaced one that pinned `TailVariant::V1` and called it
    /// "the substrate's answer". It was: for an unregistered classid the
    /// registry answers V1, and every key this crate minted carried the
    /// legacy u24 tail. Registering `CLASSID_BLOCKS_V3` upstream is what
    /// changed the answer; `mint_key` never changed at all.
    ///
    /// Anti-vacuity: the default classid is asserted to still answer V1 in
    /// the same test, so this cannot pass by the registry having been
    /// flattened to V3 everywhere.
    #[test]
    fn keys_mint_on_the_v3_tail_because_the_class_is_registered() {
        use lance_graph_contract::canonical_node::TailVariant;
        assert_eq!(classid_read_mode(CLASSID).tail_variant, TailVariant::V3);
        assert_eq!(
            classid_read_mode(NodeGuid::CLASSID_DEFAULT).tail_variant,
            TailVariant::V1,
            "an UNregistered class must still fall through to the legacy tail, \
             or this test proves nothing about registration"
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
