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
