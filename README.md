# blockly-rs

The **ABI half** of the block-editor arc: the cast between a Blockly workspace
block and the V3 ABI-shaped SoA function body.

- **Semantics only.** Presentation (`x`, `y`, `collapsed`, `inline`) lives in a
  separate `BlockView` that the cast cannot read.
- **Zero serialization.** `to_le_bytes` / `from_le_bytes` are the wire format;
  no JSON, no serde.
- **Permissive.** Never links the GPL `rash` JIT — that boundary lives in
  `scratch-rs`.

Plan + rulings: `AdaWorldAPI/OGAR docs/BLOCK-EDITOR-PLAN.md`.
Ledger: `docs/DISCOVERY-MAP.md` `D-BLOCKS-DOMAIN` / `D-BLOCKS-PALETTE` /
`D-BLOCKS-KLICKWEGE`.

```sh
cargo test
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc -p blockly-abi --no-deps
```

## The W1 falsifier

> A drag produces **zero** ABI writes. An operand change produces **exactly one**.

Both halves route through `Workspace::apply`, so a handler that let a drag reach
the record fails the test — verified by injecting exactly that leak.

## What is in the crate

| module | what it owns |
|---|---|
| root | `BlockRecord` / `BlockView` / `Workspace`, `lower_script`, `raise_calls` |
| `codebook` | Blockly type (+ dropdown code) → `FnIndex`, and the ValueParam byte encoding |
| `pool` | the constant pool — where a wide literal spends its immediate byte |
| `klickweg` | a click's **address**: `{class_id, ordinal, predicate}` |

### The cast is post-order, because Blockly is already a tree

A block's nested `inputs` are its operands, so they are emitted before it. That
is exactly the ABI's stack discipline, and it is not imposed — it falls out of
Blockly's own nesting.

### Two kinds of dropdown, and the difference is load-bearing

A **Selector** code chooses which function (`logic_compare[LT]` *is*
`FnIndex::LT`) and is spent by resolution. A **ValueParam** code is an argument
(`math_constant` is always `CONSTANT`; π or e is a value) and becomes a byte —
its ordinal in the codebook's own pinned table.

That ordinal is anchored here rather than to Blockly's live array order on
purpose. Reordering an options array is cosmetic upstream; if the encoding
tracked it, that cosmetic change would silently reinterpret every stored
program. Anchoring it in the codebook turns the hazard into a loud drift-test
failure instead.

### The constant pool is a sibling node, not a wider row

A wide literal spends its value byte as an index into a pool node — same 30
slots, same 16-byte stride, per-facet classid naming the constant's type (an
`f64` and a UTF-8 string are different readings of 12 bytes). Index `0` is the
zero-fallback, so a zeroed byte reads as *no constant*, never *constant zero*.
At 255 the remedy is a **function split**, never a wider index.

It is **opt-in**: `lower_script` still refuses a wide literal, and only
`lower_script_with_pool` interns — under caller-supplied classids, with
deliberately invalid placeholders until the concepts are minted, so a
placeholder cannot reach stored data.

### A click is an address, never a handler

`klickweg` produces `{class_id, ordinal, predicate}`; `from_key` and `seq`
belong to the session. The **ordinal is the call index**, so
`raise_calls(body)[ordinal]` is the clicked block's call by construction —
which makes an address checkable against the ABI rather than merely
self-consistent. Nothing in that module returns anything invocable, so an
`onClick` lambda cannot be expressed through it even by accident (a2ui charter
T2).

## Tests are falsifiers, checked by disabling the thing they guard

Every guard has a can-fire half and a can-stay-silent half, and each was
verified by breaking the code and confirming the right test fails:

| injection | what failed |
|---|---|
| dedup off in the pool | 3 tests |
| narrow literals routed through the pool | the silence half |
| pool returns a fixed index | 6 tests |
| `klickweg` walks pre-order | 2 tests |
| a drag reaches the record | 2 tests |

A guard that fires on everything carries as much information as one that never
fires, so both directions are asserted. Fixtures are checked too — the capacity
test's chain asserts its own `block_count`, because `with_next` *replaces* the
successor and a forward-building loop silently yields a two-block script.

## Storage: the rows were already V3 rows

`blockly-store` is the storage layer, and it is a binding rather than a port.
`ogar-loco` stores a function as 512 bytes — key at `0..16`, a reserved slot
at `16..32` written as zeroes, value at `32..512`.
`lance-graph-contract`'s `NodeRow` is the V3 canon row —
`key(16) | edges(16) | value(480)`, locked at 512 by `const _` size asserts.
Those are the same bytes, so the crate supplies only what `ogar-loco`
deliberately leaves out ("this crate does not mint GUIDs: the canonical
layout is the substrate's"):

- **a minted key** — `mint_key` routes through `NodeGuid::mint_for`. Three
  sites used to spell a key out (`k[0..4] = classid`, `k[10..16] = tail`),
  which is bit math on a layout the contract owns. It showed, too: writing
  the tail by hand put the index in the *most significant* byte of the
  24-bit identity, so function `i` addressed as `i << 16`.
- **an envelope over the array** — `ProgramRows::packet` hands the rows to
  `NodeRowPacket`, the zero-copy `SoaEnvelope` Lance's columnar I/O reads
  directly. Nothing is serialized on the way to storage.

The `lance` feature (off by default) writes them. Its Arrow schema is
*derived* from the canon's own `NODE_ROW_COLUMNS` descriptors rather than
written down, so the three columns cannot drift from the layout they
partition; a test asserts they sum to the locked stride and reassemble into
the exact row bytes.

### The V3 tail is reached through the registry, not asked for

The first cut of this composed its own classid over a `0xFF00` app-prefix
placeholder. That classid was in no registry, so `classid_read_mode` fell
through to its default and every key minted a **V1** `family:identity` u24
tail — the shape the canon calls forbidden for new units. The mechanism was
right and the address was wrong, which is the failure mode that looks like
success: keys appeared, were distinct, round-tripped, and were legacy.

The contract leaves no way around it — *"there is NO public `new_v3`
dispatch — the `tail_variant` registry field IS the mechanism"* — so
`lance-graph-contract` gained `CLASSID_BLOCKS_V3` (`0x1717_1000`) and
`ReadMode::BLOCKS_V3`, following the pattern OSINT/FMA/CPIC/PROJECT/ERP
already use. Canon `0x1717` HIGH is the Blocks domain's per-frontend
palette seat, already reserved; `0x1000` in the custom half is the canon's
own V3 generation marker, which replaces the invented placeholder outright
— and it can never be mistaken for an app prefix, since `ogar-vocab`
reserves it with a test asserting it "must never be allocatable as a port's
`APP_PREFIX`". `mint_key` never changed; registering the class is what made
its answer V3.

Its `ValueSchema` is `Bootstrap`, and that is the correct schema rather
than a placeholder: a stored function's 480-byte slab is the interleaved
call lanes, so zero tenants are materialised and the slab is entirely the
class-resolved carve-out the canon defers to the ClassView. Naming
`Cognitive` would claim tenants that are not there and invite a reader to
decode call bytes as qualia.

A real app prefix for this frontend remains the unminted operator decision
M1.

## Known gaps, deliberately

- **Three codebook gaps** (`math_on_list[RANDOM]`, `lists_reverse`,
  `lists_getIndex[GET_REMOVE]`) resolve to `None`. Codebook ids are permanent,
  so a mint is an operator decision with a ledger entry — an invented mapping is
  worse than a refusal.
- **Mutators** (`extraState`) are carried but not interpreted; a block with one
  is refused rather than lowered to a call that omits its shape.
- **Variable references** are refused — this crate does not own the variable
  table and will not guess a slot.
- **Strings longer than 12 bytes** are refused; the continuation encoding is a
  named follow-up, not a guess.
