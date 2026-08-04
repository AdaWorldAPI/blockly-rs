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

## The W1 falsifier

> A drag produces **zero** ABI writes. An operand change produces **exactly one**.

Both halves route through `Workspace::apply`, so a handler that let a drag reach
the record fails the test — verified by injecting exactly that leak.

```sh
cargo test
```
