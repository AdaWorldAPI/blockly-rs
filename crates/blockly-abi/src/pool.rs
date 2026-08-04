//! The constant pool — where a wide literal spends its immediate byte.
//!
//! # Why a pool at all
//!
//! A [`Call`](ogar_blockly::Call)'s immediate is one byte under
//! [`LaneShape::Pairs`](ogar_blockly::LaneShape). A `math_number` field is a
//! JavaScript double; a `text` field is arbitrary UTF-8. Neither fits. So the
//! byte becomes an **index**, and the only real design question is where the
//! indexed bytes live and what classid governs their reading.
//!
//! # The shape: a sibling node, not a wider row
//!
//! A pool is a second 512-byte V3 node whose identity IS the owning function's
//! — same 30 content slots, same 16-byte stride, same `classid(4) + 12` facet.
//! Each facet carries its own classid naming the constant's **type**, because
//! "your classid defines the schema, period": an `f64` and a UTF-8 string are
//! different readings of 12 bytes, so they are different classids, and the
//! per-facet classid is exactly where the substrate lets a slot say so.
//!
//! Three alternatives were considered and rejected, each by a specific
//! constraint rather than by taste:
//!
//! - **Encode the literal as a run of calls** (no pool). Killed by the W1
//!   falsifier, not by aesthetics: the call count would depend on the
//!   literal's width, so editing `255` to `1000000` would shift every
//!   subsequent call and rewrite the tail of the body. An operand edit must
//!   produce ONE write.
//! - **Steal content slots from the body node.** Killed by the call-index
//!   arithmetic: `BODY_BYTES` and the capacity asserts all assume 30 call
//!   lanes, so the budget would become per-function, and "add one string"
//!   could make a program that fit stop fitting — with the overflow blaming
//!   the calls rather than the literals.
//! - **Hold the pool in the Inventory SoA.** Killed by ownership: Inventory is
//!   shared by every function, so a per-function pool living there is a
//!   shared-mutable sink with N writers. Inventory indexes *functions*, which
//!   are shared by definition; constants are *per-function data*, which are
//!   owned by definition. Same one-byte index shape, opposite ownership.
//!
//! # Index arithmetic
//!
//! ```text
//!   idx ∈ 1..=255                       (0 = zero-fallback: NO constant)
//!   node_ordinal = (idx - 1) / 30
//!   slot_j       = (idx - 1) % 30
//!   payload      = slot_j * 16 + 4      (classid at slot_j * 16)
//! ```
//!
//! `idx = 0` is the zero-fallback rung and is never reclaimed as a real index,
//! so a zeroed value byte reads as "no constant" rather than as constant zero.
//!
//! # Capacity, and where it can actually bind
//!
//! | shape | calls | value bytes/call | distinct indices a body can name |
//! |---|---|---|---|
//! | `Pairs` | 180 | 1 | 180 — under 255, pool can never overflow first |
//! | `Triples` | 120 | 2 | 240 — still under |
//! | `Quads` | 90 | 3 | 270 — **can** exhaust the pool |
//!
//! So [`PoolError::Full`] is reachable only under `Quads`, only with ≥256
//! distinct constants, and only without dedup. It is implemented and tested
//! anyway, because "unreachable" is a measurement and not a guarantee. The
//! remedy at 255 is the same as at `BodyError::Overflow` — **split the
//! function**. A `u16` index is not the remedy; it would re-open field-widening
//! at the one place the ABI is least defended.
//!
//! # Repoint, never mutate
//!
//! Interning is content-addressed, so one `3.14` referenced by two calls is one
//! entry. Editing ONE of those call sites therefore must not move the other:
//! an edit **interns the new value and repoints that call's index byte**. It
//! never rewrites a pool entry in place. Mutating in place would be the
//! shared-mutable-sink defect one layer down, and would make a local edit
//! change a program elsewhere in the same function.
//!
//! Repointing leaves orphans. They are **not** compacted implicitly:
//! compaction renumbers, renumbering rewrites every referencing body byte, and
//! that turns a one-byte edit into a whole-program write. Reserve, don't
//! reclaim — an orphan holds its index until an explicit, versioned pass.
//!
//! # The classids are PARAMETERS, and that is deliberate
//!
//! `ConstantPool` never names a concept id. The caller supplies the facet
//! classid, because minting `0x1703..` is an operator decision with a ledger
//! entry, and this crate does not get to assume one. The arithmetic and the
//! dedup — which is where the defects live — are testable today against
//! [`placeholder`] classids; the mint changes two constants, not the logic.

use ogar_blockly::{CLASSID_BYTES, CONTENT_SLOTS, PAYLOAD_BYTES_PER_SLOT, SLOT_STRIDE};

/// Payload bytes one constant facet carries.
pub const CONSTANT_BYTES: usize = PAYLOAD_BYTES_PER_SLOT;

/// Constants per pool node — one per content slot.
pub const CONSTANTS_PER_NODE: usize = CONTENT_SLOTS;

/// The largest index a value byte can name. `0` is the zero-fallback, so the
/// usable domain is `1..=255`.
pub const MAX_CONSTANTS: usize = 255;

/// Placeholder facet classids, for use until the concepts are minted.
///
/// These are **not** proposed ids and must not become them: they sit in a
/// deliberately invalid range so that a placeholder escaping into stored data
/// is loud rather than plausible. The mint proposal (`ConstantPool`,
/// `ConstF64`, `ConstUtf8Inline`) lives in OGAR `docs/BLOCK-EDITOR-PLAN.md`
/// D4; nothing here assumes it.
pub mod placeholder {
    /// Placeholder for an `f64` constant.
    pub const CONST_F64: u32 = 0xDEAD_0001;
    /// Placeholder for an inline UTF-8 constant.
    pub const CONST_UTF8_INLINE: u32 = 0xDEAD_0002;
}

/// Why a constant could not be interned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolError {
    /// All 255 indices are spoken for. The remedy is a function split, never a
    /// wider index.
    Full,
    /// The value does not fit one facet's 12 payload bytes. Refused rather
    /// than truncated — a truncated constant would look like success, which is
    /// strictly worse than today's refusal.
    TooWide {
        /// How many bytes the value needed.
        needed: usize,
    },
}

impl core::fmt::Display for PoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            PoolError::Full => write!(
                f,
                "the constant pool is full at {MAX_CONSTANTS} entries; split the function"
            ),
            PoolError::TooWide { needed } => write!(
                f,
                "constant needs {needed} bytes, more than the {CONSTANT_BYTES} a facet holds"
            ),
        }
    }
}

impl core::error::Error for PoolError {}

/// One interned constant: its type classid and its 12 payload bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constant {
    /// The facet classid naming HOW the payload reads.
    pub classid: u32,
    /// The payload, zero-padded to the facet width.
    pub bytes: [u8; CONSTANT_BYTES],
}

/// A function's constant pool.
///
/// Held beside the [`FunctionBody`](ogar_blockly::FunctionBody), never inside
/// it — the body's 30 lanes stay 30 call lanes, and its capacity arithmetic is
/// untouched by how many constants a program uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConstantPool {
    entries: Vec<Constant>,
}

impl ConstantPool {
    /// An empty pool.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// How many constants are interned.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the pool holds nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many pool NODES this pool currently occupies.
    #[must_use]
    pub fn node_count(&self) -> usize {
        self.entries.len().div_ceil(CONSTANTS_PER_NODE)
    }

    /// Intern a value, returning the index a call's value byte carries.
    ///
    /// **Content-addressed**: interning the same `(classid, bytes)` twice
    /// returns the same index. That is what keeps one `3.14` used twice from
    /// spending two slots — and it is what makes the repoint-don't-mutate rule
    /// necessary, since an entry may have several referents.
    ///
    /// # Errors
    ///
    /// [`PoolError::TooWide`] if the value exceeds a facet; [`PoolError::Full`]
    /// at 255 entries.
    pub fn intern(&mut self, classid: u32, value: &[u8]) -> Result<u8, PoolError> {
        if value.len() > CONSTANT_BYTES {
            return Err(PoolError::TooWide {
                needed: value.len(),
            });
        }
        let mut bytes = [0u8; CONSTANT_BYTES];
        bytes[..value.len()].copy_from_slice(value);
        let candidate = Constant { classid, bytes };

        if let Some(pos) = self.entries.iter().position(|c| *c == candidate) {
            // Position is < len <= MAX_CONSTANTS, so the +1 cannot overflow.
            return Ok(u8::try_from(pos + 1).expect("interned index within u8"));
        }
        if self.entries.len() >= MAX_CONSTANTS {
            return Err(PoolError::Full);
        }
        self.entries.push(candidate);
        u8::try_from(self.entries.len()).map_err(|_| PoolError::Full)
    }

    /// Intern an `f64` under the caller's classid.
    ///
    /// # Errors
    ///
    /// As [`intern`](Self::intern).
    pub fn intern_f64(&mut self, classid: u32, value: f64) -> Result<u8, PoolError> {
        self.intern(classid, &value.to_le_bytes())
    }

    /// Intern a string under the caller's classid — inline only.
    ///
    /// A string longer than one facet is **refused**, not chained: the
    /// continuation encoding is a named follow-up, and shipping a chaining
    /// rule with no corpus behind it would be a guess wearing a pool costume.
    ///
    /// # Errors
    ///
    /// As [`intern`](Self::intern).
    pub fn intern_str(&mut self, classid: u32, value: &str) -> Result<u8, PoolError> {
        self.intern(classid, value.as_bytes())
    }

    /// Resolve an index back to its constant.
    ///
    /// `0` is the zero-fallback and yields `None` — a zeroed value byte means
    /// "no constant", never "constant zero".
    #[must_use]
    pub fn resolve(&self, idx: u8) -> Option<&Constant> {
        if idx == 0 {
            return None;
        }
        self.entries.get(usize::from(idx) - 1)
    }

    /// Which pool node an index lives in, and which slot within it.
    ///
    /// Returns `None` for the zero-fallback.
    #[must_use]
    pub fn locate(idx: u8) -> Option<(usize, usize)> {
        if idx == 0 {
            return None;
        }
        let zero_based = usize::from(idx) - 1;
        Some((
            zero_based / CONSTANTS_PER_NODE,
            zero_based % CONSTANTS_PER_NODE,
        ))
    }

    /// The byte offset of a slot's PAYLOAD inside a pool node's value slab.
    ///
    /// The facet's classid sits at `slot_j * SLOT_STRIDE`, immediately before.
    #[must_use]
    pub const fn slot_payload_offset(slot_j: usize) -> usize {
        slot_j * SLOT_STRIDE + CLASSID_BYTES
    }

    /// Write one pool node's value slab: classid + payload per occupied slot,
    /// zeroes elsewhere.
    ///
    /// Unoccupied slots are **zeroed, not skipped** — reserve, don't reclaim.
    #[must_use]
    pub fn write_node(&self, node_ordinal: usize) -> [u8; CONTENT_SLOTS * SLOT_STRIDE] {
        let mut slab = [0u8; CONTENT_SLOTS * SLOT_STRIDE];
        let base = node_ordinal * CONSTANTS_PER_NODE;
        for slot_j in 0..CONSTANTS_PER_NODE {
            let Some(c) = self.entries.get(base + slot_j) else {
                break;
            };
            let at = slot_j * SLOT_STRIDE;
            slab[at..at + CLASSID_BYTES].copy_from_slice(&c.classid.to_le_bytes());
            let p = Self::slot_payload_offset(slot_j);
            slab[p..p + CONSTANT_BYTES].copy_from_slice(&c.bytes);
        }
        slab
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use placeholder::{CONST_F64, CONST_UTF8_INLINE};

    #[test]
    fn interning_two_distinct_values_yields_two_distinct_indices() {
        // Part A of the falsifier: it must FIRE, and carry information. A pool
        // that returned a fixed index for every literal would pass "lowered
        // ok" and fail right here.
        let mut pool = ConstantPool::new();
        let small = pool.intern_f64(CONST_F64, 7.25).unwrap();
        let big = pool.intern_f64(CONST_F64, 1_000_000.0).unwrap();
        assert_ne!(
            small, big,
            "two distinct constants collapsed onto one index"
        );
        assert_eq!(pool.len(), 2);

        // …and the values read back BIT-EXACT. Asserting only that interning
        // succeeded would be truncation wearing a pool costume, which is the
        // exact defect the whole design exists to prevent.
        let back = |idx: u8| {
            let c = pool.resolve(idx).unwrap();
            assert_eq!(c.classid, CONST_F64);
            f64::from_le_bytes(c.bytes[..8].try_into().unwrap())
        };
        assert_eq!(back(small), 7.25);
        assert_eq!(back(big), 1_000_000.0);
    }

    #[test]
    fn the_same_value_interns_once_and_two_values_interns_twice() {
        // Part C, two-sided. The first half alone passes a pool that returns
        // index 1 for everything; the second alone passes a pool that never
        // dedups. Both are needed.
        let mut pool = ConstantPool::new();
        let a = pool.intern_f64(CONST_F64, 1.5).unwrap();
        let b = pool.intern_f64(CONST_F64, 1.5).unwrap();
        assert_eq!(a, b, "the same value must intern once");
        assert_eq!(pool.len(), 1);

        let c = pool.intern_f64(CONST_F64, 1.25).unwrap();
        assert_ne!(a, c, "different values must not share an index");
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn the_classid_participates_in_identity() {
        // The same bytes under two different readings are two constants —
        // "your classid defines the schema, period". Deduping on bytes alone
        // would make an f64 and a string alias.
        let mut pool = ConstantPool::new();
        let a = pool.intern(CONST_F64, &[1, 2, 3]).unwrap();
        let b = pool.intern(CONST_UTF8_INLINE, &[1, 2, 3]).unwrap();
        assert_ne!(a, b);
        assert_eq!(pool.len(), 2);
    }

    #[test]
    fn index_zero_is_the_fallback_and_never_a_constant() {
        let mut pool = ConstantPool::new();
        assert_eq!(pool.resolve(0), None, "empty pool");
        let first = pool.intern_f64(CONST_F64, 1.0).unwrap();
        // The first real constant is index 1, NOT 0 — otherwise a zeroed value
        // byte would read as a live constant reference.
        assert_eq!(first, 1);
        assert_eq!(pool.resolve(0), None, "populated pool");
        assert!(pool.resolve(1).is_some());
        assert_eq!(ConstantPool::locate(0), None);
    }

    #[test]
    fn the_node_and_slot_arithmetic_matches_the_documented_boundaries() {
        // The four boundaries the module doc names, asserted rather than
        // narrated. A `<=` vs `<` slip in the divisor shows up here.
        assert_eq!(ConstantPool::locate(1), Some((0, 0)));
        assert_eq!(ConstantPool::locate(30), Some((0, 29)));
        assert_eq!(ConstantPool::locate(31), Some((1, 0)));
        assert_eq!(ConstantPool::locate(255), Some((8, 14)));

        assert_eq!(ConstantPool::slot_payload_offset(0), 4);
        assert_eq!(ConstantPool::slot_payload_offset(29), 468);
        // The last payload must END inside the slab, not past it.
        assert_eq!(
            ConstantPool::slot_payload_offset(29) + CONSTANT_BYTES,
            CONTENT_SLOTS * SLOT_STRIDE
        );
    }

    #[test]
    fn a_value_wider_than_a_facet_is_refused_not_truncated() {
        let mut pool = ConstantPool::new();
        // Twelve bytes fit exactly; thirteen do not. Two-sided, so a guard
        // that refused everything would fail the first half.
        assert!(pool.intern_str(CONST_UTF8_INLINE, "abcdefghijkl").is_ok());
        assert_eq!(
            pool.intern_str(CONST_UTF8_INLINE, "abcdefghijklm"),
            Err(PoolError::TooWide { needed: 13 })
        );
        assert_eq!(pool.len(), 1, "the refused value must not have landed");
    }

    #[test]
    fn the_pool_fills_at_255_and_refuses_the_256th() {
        // Reachable only under Quads with 256 distinct constants and no dedup
        // — but implemented and tested, because "unreachable" is a
        // measurement, not a guarantee.
        let mut pool = ConstantPool::new();
        for i in 0..MAX_CONSTANTS {
            let v = u32::try_from(i).unwrap();
            let idx = pool.intern(CONST_F64, &v.to_le_bytes()).unwrap();
            assert_eq!(usize::from(idx), i + 1);
        }
        assert_eq!(pool.len(), MAX_CONSTANTS);
        assert_eq!(pool.node_count(), 9);
        assert_eq!(
            pool.intern(CONST_F64, &999_u32.to_le_bytes()),
            Err(PoolError::Full)
        );
        // Silence twin: a FULL pool still resolves an already-interned value
        // rather than erroring — dedup must not be collateral damage.
        assert_eq!(pool.intern(CONST_F64, &7_u32.to_le_bytes()), Ok(8));
    }

    #[test]
    fn a_written_node_places_classid_then_payload_at_the_documented_offsets() {
        let mut pool = ConstantPool::new();
        pool.intern_f64(CONST_F64, 1.0).unwrap();
        pool.intern_str(CONST_UTF8_INLINE, "hi").unwrap();
        let slab = pool.write_node(0);

        assert_eq!(&slab[0..4], &CONST_F64.to_le_bytes());
        assert_eq!(
            f64::from_le_bytes(slab[4..12].try_into().unwrap()),
            1.0,
            "payload must sit at slot*16 + 4"
        );
        assert_eq!(&slab[16..20], &CONST_UTF8_INLINE.to_le_bytes());
        assert_eq!(&slab[20..22], b"hi");
        // Unoccupied slots are zeroed, not skipped — reserve, don't reclaim.
        assert!(slab[32..].iter().all(|b| *b == 0));
        // And the two facets did NOT overlap: slot 1's classid begins exactly
        // where slot 0's 12-byte payload ends.
        assert_eq!(
            ConstantPool::slot_payload_offset(0) + CONSTANT_BYTES,
            SLOT_STRIDE
        );
    }

    #[test]
    fn the_second_node_holds_the_31st_constant_and_not_the_30th() {
        let mut pool = ConstantPool::new();
        for i in 0..31u32 {
            pool.intern(CONST_F64, &i.to_le_bytes()).unwrap();
        }
        assert_eq!(pool.node_count(), 2);
        let node1 = pool.write_node(1);
        // Constant #31 (index 31) is node 1, slot 0.
        assert_eq!(&node1[4..8], &30_u32.to_le_bytes());
        // …and node 1's slot 1 onward is still zero.
        assert!(node1[16..].iter().all(|b| *b == 0));
        // Node 0's LAST slot holds #30, proving the boundary is not off by one.
        let node0 = pool.write_node(0);
        let last = ConstantPool::slot_payload_offset(29);
        assert_eq!(&node0[last..last + 4], &29_u32.to_le_bytes());
    }
}
