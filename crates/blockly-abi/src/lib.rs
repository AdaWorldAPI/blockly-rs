//! `blockly-abi` — the **cast** between a Blockly workspace block and the V3
//! ABI-shaped SoA function body.
//!
//! # What this is
//!
//! Not a parser, not a serializer, not a renderer. One thing: a Blockly block
//! record goes in, [`ogar_loco::FunctionBody`] comes out, and back again.
//! `to_le_bytes` / `from_le_bytes` on the far side ARE the wire format — there
//! is no JSON, no intermediate DTO, no serde anywhere in this crate.
//!
//! JavaScript keeps dragging the puzzle pieces. This owns the semantics.
//!
//! # The split that makes it work
//!
//! Blockly's own save record (`core/serialization/blocks.ts`, the `State`
//! interface) mixes **three** unlike things, not two:
//!
//! | bucket | fields | where it goes |
//! |---|---|---|
//! | **semantic** | `type`, `fields`, `inputs`, `next`, `extraState`, `disabledReasons` | the ABI |
//! | **presentation** | `x`, `y`, `collapsed`, `inline` | [`BlockView`] |
//! | **editor policy** | `deletable`, `movable`, `editable` | neither — see below |
//!
//! Two of those placements are easy to get wrong, so they are stated rather
//! than assumed:
//!
//! - **`extraState` is SEMANTIC.** It is the block's mutator payload — how many
//!   sockets a `controls_if` has, how many items a `lists_create_with` holds.
//!   It changes the block's shape and therefore the program. Filing it under
//!   presentation because it "isn't a field" would make the cast quietly lossy.
//!   **Not yet modelled here** — see [`BlockRecord::extra_state`], which
//!   preserves it as opaque text so it cannot be silently dropped.
//! - **`disabledReasons` is SEMANTIC.** A disabled block is excluded from code
//!   generation; that is program behaviour, not a grey tint.
//! - **Editor policy is a third bucket.** `deletable` / `movable` / `editable`
//!   gate what a *user* may do in the editor. They change neither what the
//!   program computes nor how the block looks, so forcing them into either
//!   bucket is a mistake; they belong to the editing session (and, eventually,
//!   to RBAC — the a2ui `WideFieldMask` projection, not this cast).
//!
//! **Moving a block 40 pixels must not touch the ABI.** That is the whole
//! thesis, and it is the [W1 falsifier](#the-falsifier): a drag produces ZERO
//! writes; an operand change produces EXACTLY ONE. Both directions are tested
//! — a cast that never changed would pass the first half vacuously.
//!
//! The falsifier's two cases correspond to real Blockly events: a pure drag is
//! a `BlockMove` whose `reason` includes `'drag'` with `oldParentId ==
//! newParentId` and only the coordinates differing; an operand edit is a
//! `BlockChange` with `element == "field"`.
//!
//! # The shape of the cast: post-order, because Blockly is already a tree
//!
//! A top-level block plus its `next` chain is one **function**. Each block is
//! one [`Call`]. A block's nested `inputs` are its **operands** — so they are
//! emitted BEFORE it:
//!
//! ```text
//!   Blockly tree                     calls (execution order)
//!   ────────────                     ───────────────────────
//!   math_arithmetic[ADD]             (NUMBER:5)      ← input A, emitted first
//!     A: math_number(5)              (NUMBER:3)      ← input B
//!     B: math_number(3)              (ADD:0)         ← then the parent
//! ```
//!
//! That is exactly the stack discipline the ABI specifies, and it is not
//! imposed here — it falls out of Blockly's own nesting. A post-order walk of
//! the input tree IS operand-then-operator.
//!
//! `next` is the statement sequence, walked in order after the block's own
//! call is emitted.
//!
//! # What this crate does NOT do
//!
//! - **No block-renderer port.** Blockly (Apache-2.0) keeps rendering.
//! - **No GPL.** This half never links `rash`; that boundary lives in
//!   `scratch-rs`. See OGAR `docs/BLOCK-EDITOR-PLAN.md` § "Where the seam falls".
//! - **No wire framing.** `NodeDelta` / `ActionInvoke` are a2ui-rs's job (W2).

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use ogar_loco::{BodyError, Call, FnIndex, FunctionBody, LaneShape};

pub mod codebook;
pub mod klickweg;
pub mod palette;
pub mod program;
pub mod projection;
pub mod registry;

pub use codebook::{OpcodeMapping, resolve_opcode};
pub use klickweg::{BlockAddress, address_of};
pub use palette::{
    BlocklyPalette, DEVICE_FAMILY_FLOOR, PALETTE_CONCEPT, plug_into, render_classid,
};
pub use program::lower_program;
pub use projection::{ProjectionError, parse_text, render_text};
pub use registry::registry;

// ── The shared ABI, consumed rather than copied (the flip, ruling R1/Q6) ────
// The generic machinery this crate carried during the float — the stored
// node, the constant pool, the program/reference rules, the flow tables —
// now lives ONCE in `ogar-loco`, and this crate's local copies are DELETED.
// The re-exports keep the old paths compiling; the flow tables are reached
// through `ogar_loco::vocabulary::shared_core` (the Blockly palette is
// entirely shared-core today) and, where a proof is required, through
// [`checked_vocabulary`]. CI forbids the deleted definitions from returning.
pub use ogar_loco::node::NODE_BYTES;
pub use ogar_loco::{
    CheckedVocabulary, Constant, ConstantPool, FunctionNode, PoolError, Program, branches_of,
};

/// The Blockly palette, validated — the proof-carrying token
/// [`Program::references_are_resolvable`] and [`branches_of`] require.
///
/// Validation is a 256-byte sweep done once (lazily) per process;
/// [`BlocklyPalette`]'s conformance is additionally pinned by
/// [`palette`]'s own tests, so the `expect` here is a near-static fact,
/// not a runtime hope.
#[must_use]
pub fn checked_vocabulary() -> &'static CheckedVocabulary<crate::palette::BlocklyPalette> {
    static CHECKED: std::sync::LazyLock<CheckedVocabulary<crate::palette::BlocklyPalette>> =
        std::sync::LazyLock::new(|| {
            ogar_loco::vocabulary::conformance::validate(crate::palette::BlocklyPalette)
                .expect("BlocklyPalette conforms; pinned by palette::tests")
        });
    &CHECKED
}

// ── The semantic record ─────────────────────────────────────────────────────

/// One field value on a block — the immediate a [`Call`] carries.
///
/// Blockly field values are strings on the wire even when they denote numbers,
/// so this preserves the distinction the ABI needs: a value that fits a byte,
/// versus one that must eventually spend its byte as a constant-pool index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// A value that fits in one immediate byte, `0..=255`.
    Byte(u8),
    /// A dropdown option code (`"LT"`, `"ADD"`, …). Selects WHICH function,
    /// not a value — resolved by [`resolve_opcode`] together with the block
    /// type, never stored as an immediate.
    Code(String),
    /// A value too wide for one byte (a large number, a string, a float).
    /// Spends the call's value byte as a [`ConstantPool`] index.
    ///
    /// Which cast is used decides what happens: [`lower_script`] still
    /// **refuses** it ([`CastError::WideLiteral`]), because the pool's facet
    /// concepts are an operator mint that has not landed;
    /// [`lower_script_with_pool`] interns it under caller-supplied classids.
    /// The refusal stays the default so a placeholder classid cannot reach
    /// stored data by accident.
    Wide(String),
    /// A **reference**, not a value — a variable field serializes to an object
    /// (`{id}`, or `{id, name, type}` under full serialization), never a bare
    /// string. The id resolves to a variable slot, and the slot index is what
    /// becomes the call's immediate byte.
    ///
    /// Unresolved here: this crate does not own the variable table. A `Ref`
    /// lowers to [`CastError::UnresolvedRef`] rather than inventing a slot,
    /// for the same reason an unknown opcode is refused — an invented index is
    /// worse than a refusal.
    Ref {
        /// The variable model id Blockly wrote.
        id: String,
    },
}

/// The **semantic** half of a Blockly block record. Presentation lives in
/// [`BlockView`] and is deliberately not reachable from here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
    /// Blockly's block `type` — e.g. `"math_arithmetic"`, `"controls_if"`.
    pub ty: String,
    /// Blockly's block id. Identity for the view layer and for nesting; never
    /// stored in the ABI (the node's own identity names the function).
    pub id: String,
    /// Field values by field name, e.g. `{"OP": Code("ADD")}`, `{"NUM": Byte(5)}`.
    pub fields: Vec<(String, FieldValue)>,
    /// Nested operand blocks by input name, in the block's own input order.
    ///
    /// Blockly's `ConnectionState` can carry a `shadow` and a `block`
    /// **simultaneously** — a real block plugged into a socket whose default
    /// placeholder is remembered underneath it. This holds the EFFECTIVE
    /// operand: the real `block` when one is connected, otherwise the `shadow`
    /// (which is the socket's default value, and is genuinely semantic). The
    /// remembered-underneath shadow is editor state and belongs with
    /// [`BlockView`], not here.
    pub inputs: Vec<(String, BlockRecord)>,
    /// **Statement** inputs — a `controls_repeat`'s `DO`, a `controls_if`'s
    /// `SUBSTACK`. Held separately from [`inputs`](Self::inputs) because
    /// Blockly's own model separates them: a value input carries a reporter
    /// whose result is consumed, a statement input carries a stack of
    /// statements that is BRANCHED TO.
    ///
    /// The distinction is load-bearing rather than tidy. A statement input
    /// lowered as an operand would be evaluated before its parent and pushed
    /// onto the stack — so `repeat 10 [move]` would run the body once,
    /// unconditionally, and then hand the loop a count it never computed.
    /// Each statement input becomes its **own function**, referenced by index
    /// (the W0 nesting-by-reference ruling), never spliced inline.
    pub statements: Vec<(String, BlockRecord)>,
    /// The next statement in the chain, if any.
    pub next: Option<Box<BlockRecord>>,
    /// The block's mutator payload, verbatim, if it has one.
    ///
    /// **Semantic** — it decides how many sockets the block has. This crate
    /// does not yet interpret it (per-block-type mutator schemas are their own
    /// wave), but it is carried rather than dropped so that
    /// [`lower_script`] can REFUSE a block whose shape it cannot faithfully
    /// represent instead of silently emitting a call that omits it.
    pub extra_state: Option<String>,
    /// Whether the block is excluded from code generation.
    ///
    /// **Semantic** (Blockly's `disabledReasons`): a disabled block does not
    /// run. Carried so the cast can refuse rather than silently emit it as
    /// live code.
    pub disabled: bool,
}

impl BlockRecord {
    /// A leaf block with no fields, inputs, or successor.
    #[must_use]
    pub fn leaf(ty: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            ty: ty.into(),
            id: id.into(),
            fields: Vec::new(),
            inputs: Vec::new(),
            statements: Vec::new(),
            next: None,
            extra_state: None,
            disabled: false,
        }
    }

    /// Builder: attach a mutator payload (`extraState`).
    #[must_use]
    pub fn with_extra_state(mut self, state: impl Into<String>) -> Self {
        self.extra_state = Some(state.into());
        self
    }

    /// Builder: attach a field.
    #[must_use]
    pub fn with_field(mut self, name: impl Into<String>, value: FieldValue) -> Self {
        self.fields.push((name.into(), value));
        self
    }

    /// Builder: attach a nested operand.
    #[must_use]
    pub fn with_input(mut self, name: impl Into<String>, block: BlockRecord) -> Self {
        self.inputs.push((name.into(), block));
        self
    }

    /// Builder: attach the next statement.
    #[must_use]
    pub fn with_next(mut self, block: BlockRecord) -> Self {
        self.next = Some(Box::new(block));
        self
    }

    /// Builder: attach a STATEMENT input (a loop or branch body).
    ///
    /// Distinct from [`with_input`](Self::with_input): that attaches an
    /// operand, this attaches a body that becomes its own function.
    #[must_use]
    pub fn with_statement(mut self, name: impl Into<String>, block: BlockRecord) -> Self {
        self.statements.push((name.into(), block));
        self
    }

    /// Look up a field value by name.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&FieldValue> {
        self.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }

    /// The first dropdown *code* among this block's fields, if any — the half
    /// of the opcode that a dropdown carries (`logic_compare` + `"LT"`).
    #[must_use]
    pub fn dropdown_code(&self) -> Option<&str> {
        self.fields.iter().find_map(|(_, v)| match v {
            FieldValue::Code(c) => Some(c.as_str()),
            _ => None,
        })
    }

    /// Total blocks in this subtree — the block itself, its nested inputs, and
    /// its `next` chain. This is the number of [`Call`]s the cast will emit.
    #[must_use]
    pub fn block_count(&self) -> usize {
        1 + self
            .inputs
            .iter()
            .map(|(_, b)| b.block_count())
            .sum::<usize>()
            + self
                .statements
                .iter()
                .map(|(_, b)| b.block_count())
                .sum::<usize>()
            + self.next.as_ref().map_or(0, |n| n.block_count())
    }
}

/// The **presentation** half — everything the ABI must never see.
///
/// Held per block id, alongside the program rather than inside it. Mutating a
/// `BlockView` is by construction incapable of changing a [`FunctionBody`]:
/// the cast cannot read this type, because it is not an input to it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BlockView {
    /// Workspace x, in Blockly's own units.
    pub x: f64,
    /// Workspace y.
    pub y: f64,
    /// Whether the block is rendered collapsed.
    pub collapsed: bool,
    /// Whether inputs render inline.
    pub inline: bool,
}

// ── Errors ──────────────────────────────────────────────────────────────────

/// Why a block record could not be cast into a function body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastError {
    /// The block type (with its dropdown code, if any) has no entry in the
    /// `ogar-blockly` codebook. NEVER guessed: an invented mapping baked into
    /// a permanent codebook is worse than a refusal.
    UnknownOpcode {
        /// The Blockly block `type`.
        ty: String,
        /// The dropdown code that accompanied it, if any.
        code: Option<String>,
    },
    /// A field value exceeds one immediate byte and needs the constant pool,
    /// which is a named follow-up rather than a shipped surface.
    WideLiteral {
        /// The block whose field is too wide.
        ty: String,
        /// The literal text.
        value: String,
    },
    /// A variable reference whose slot this crate cannot resolve — it does not
    /// own the variable table. Refused rather than assigned a guessed index.
    UnresolvedRef {
        /// The block carrying the reference.
        ty: String,
        /// The Blockly variable-model id.
        id: String,
    },
    /// The block carries a mutator payload (`extraState`) that decides its
    /// shape — how many sockets it has. Refused rather than lowered to a call
    /// that silently omits it; per-block-type mutator schemas are their own
    /// wave.
    MutatorUnsupported {
        /// The block carrying the payload.
        ty: String,
    },
    /// A dropdown code the codebook cannot turn into a byte — either an
    /// [argument](codebook::CodeRole::ValueParam) whose option set does not
    /// name it, or a code on a block whose resolution takes no code at all.
    /// Refused rather than dropped: dropping it would lower
    /// `math_constant[PI]` and `math_constant[E]` to identical calls.
    UnencodedValueParam {
        /// The block type.
        ty: String,
        /// The dropdown code with no encoding.
        code: String,
    },
    /// A wide literal could not be interned — wider than a facet, or the pool
    /// is full. The remedy for a full pool is a function split, never a wider
    /// index; the remedy for an oversized literal is the continuation
    /// encoding, which is a named follow-up.
    ConstantPool {
        /// The block carrying the literal.
        ty: String,
        /// The literal text.
        value: String,
        /// What the pool said.
        source: PoolError,
    },
    /// The block carries statement inputs but its function branches to
    /// nothing — the record and the codebook disagree about its shape.
    /// Refused rather than dropped, since dropping discards a whole body.
    UnexpectedStatements {
        /// The block type.
        ty: String,
        /// How many statement inputs it carried.
        found: usize,
    },
    /// The call needs more body-reference immediates than the shape holds.
    /// `IF_ELSE` under `Pairs` would truncate the else arm into nothing, and
    /// the program would run the then branch and silently skip the else.
    ShapeTooNarrow {
        /// The block type.
        ty: String,
        /// Body references the call needs.
        needed: usize,
        /// The shape that cannot hold them.
        shape: LaneShape,
    },
    /// More than 255 functions — a body index is one byte. The remedy is the
    /// same as everywhere else in this ABI: split, never widen.
    TooManyFunctions {
        /// How many the script reached.
        count: usize,
    },
    /// The block carries more immediates than one call can hold. The remedy is
    /// a wider [`LaneShape`], or a block shape the palette models with a
    /// dedicated slot — never a silently truncated call.
    TooManyValues {
        /// The block type.
        ty: String,
        /// How many fields it offered.
        offered: usize,
    },
    /// The program did not fit the node's call budget, or a call carried an
    /// immediate the lane shape cannot store. The remedy is a split into two
    /// functions (or a wider shape), never a bigger row.
    Body(BodyError),
}

impl core::fmt::Display for CastError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CastError::UnknownOpcode { ty, code } => match code {
                Some(c) => write!(f, "no codebook entry for block `{ty}` with code `{c}`"),
                None => write!(f, "no codebook entry for block `{ty}`"),
            },
            CastError::WideLiteral { ty, value } => write!(
                f,
                "block `{ty}` carries the literal `{value}`, which exceeds one \
                 immediate byte; the constant pool is a named follow-up"
            ),
            CastError::UnresolvedRef { ty, id } => write!(
                f,
                "block `{ty}` references variable `{id}`; this crate does not \
                 own the variable table and will not guess a slot"
            ),
            CastError::MutatorUnsupported { ty } => write!(
                f,
                "block `{ty}` carries an extraState mutator payload that decides \
                 its shape; lowering it would silently drop that shape"
            ),
            CastError::UnencodedValueParam { ty, code } => write!(
                f,
                "block `{ty}` dropdown `{code}` has no byte encoding in the \
                 codebook; dropping it would make distinct programs identical"
            ),
            CastError::ConstantPool { ty, value, source } => write!(
                f,
                "block `{ty}` literal `{value}` could not be interned: {source}"
            ),
            CastError::UnexpectedStatements { ty, found } => write!(
                f,
                "block `{ty}` carries {found} statement input(s) but its function \
                 branches to nothing"
            ),
            CastError::ShapeTooNarrow { ty, needed, shape } => write!(
                f,
                "block `{ty}` needs {needed} body references, more than {shape:?} \
                 can hold; a narrower shape would truncate a branch away"
            ),
            CastError::TooManyFunctions { count } => write!(
                f,
                "the script reached {count} functions; a body index is one byte, \
                 so split rather than widen"
            ),
            CastError::TooManyValues { ty, offered } => write!(
                f,
                "block `{ty}` offers {offered} fields, more immediates than one \
                 call can hold; truncating would emit a call that looks complete"
            ),
            CastError::Body(e) => write!(f, "{e}"),
        }
    }
}

impl core::error::Error for CastError {}

impl From<BodyError> for CastError {
    fn from(e: BodyError) -> Self {
        CastError::Body(e)
    }
}

// ── The cast: down ──────────────────────────────────────────────────────────

/// Cast a script — a top-level block and its `next` chain — into a function
/// body.
///
/// Post-order: a block's nested inputs are emitted before the block itself,
/// which is the stack discipline. `next` follows.
///
/// # Errors
///
/// [`CastError::UnknownOpcode`] for a block the codebook does not name;
/// [`CastError::WideLiteral`] for an immediate that needs the constant pool;
/// [`CastError::Body`] if the program overruns the shape's call budget.
pub fn lower_script(shape: LaneShape, top: &BlockRecord) -> Result<FunctionBody, CastError> {
    let mut body = FunctionBody::new(shape);
    lower_chain(top, &mut body, None)?;
    Ok(body)
}

/// The classids under which wide literals are interned, plus the pool itself.
///
/// The classids are **parameters** rather than constants because minting them
/// is an operator decision with a ledger entry (OGAR `BLOCK-EDITOR-PLAN.md`
/// D4). Until that mint lands, [`ogar_loco::pool::placeholder`] supplies deliberately
/// invalid ids so a placeholder escaping into stored data is loud rather than
/// plausible.
#[derive(Debug, Clone)]
pub struct LoweringContext {
    /// The pool wide literals intern into.
    pub pool: ConstantPool,
    /// Facet classid for an `f64` constant.
    pub f64_classid: u32,
    /// Facet classid for an inline UTF-8 constant.
    pub utf8_classid: u32,
}

impl LoweringContext {
    /// A context using the placeholder classids — for tests and for callers
    /// that have not yet been handed minted ids.
    #[must_use]
    pub fn placeholder() -> Self {
        Self {
            pool: ConstantPool::new(),
            f64_classid: ogar_loco::pool::placeholder::CONST_F64,
            utf8_classid: ogar_loco::pool::placeholder::CONST_UTF8_INLINE,
        }
    }
}

/// Cast a script, interning wide literals into a constant pool.
///
/// The difference from [`lower_script`] is exactly one: a
/// [`FieldValue::Wide`] spends its immediate byte as a pool index instead of
/// being refused. Everything else — post-order, selector spending, ValueParam
/// ordinals — is identical, and a script with no wide literals produces a
/// **byte-identical** body and an **empty** pool.
///
/// # Errors
///
/// As [`lower_script`], plus [`CastError::ConstantPool`] when a literal is
/// wider than a facet or the pool is full.
pub fn lower_script_with_pool(
    shape: LaneShape,
    top: &BlockRecord,
    ctx: &mut LoweringContext,
) -> Result<FunctionBody, CastError> {
    let mut body = FunctionBody::new(shape);
    lower_chain(top, &mut body, Some(ctx))?;
    Ok(body)
}

/// Walk a statement chain: each block's operands, then the block, then `next`.
fn lower_chain(
    block: &BlockRecord,
    body: &mut FunctionBody,
    mut ctx: Option<&mut LoweringContext>,
) -> Result<(), CastError> {
    lower_block(block, body, ctx.as_deref_mut())?;
    if let Some(next) = &block.next {
        lower_chain(next, body, ctx)?;
    }
    Ok(())
}

/// Emit one block: operands first (post-order), then the block's own call.
fn lower_block(
    block: &BlockRecord,
    body: &mut FunctionBody,
    mut ctx: Option<&mut LoweringContext>,
) -> Result<(), CastError> {
    // Operands first — this IS the stack discipline, and it is Blockly's own
    // nesting rather than a convention imposed on it.
    for (_, operand) in &block.inputs {
        lower_block(operand, body, ctx.as_deref_mut())?;
    }
    let call = call_for(block, ctx)?;
    body.push(call)?;
    Ok(())
}

/// The single call one block lowers to — its function index plus its immediate.
pub(crate) fn call_for(
    block: &BlockRecord,
    mut ctx: Option<&mut LoweringContext>,
) -> Result<Call, CastError> {
    // A mutator payload decides the block's SHAPE. Lowering without it would
    // emit a call that looks complete and is not.
    if block.extra_state.is_some() {
        return Err(CastError::MutatorUnsupported {
            ty: block.ty.clone(),
        });
    }

    let code = block.dropdown_code();
    let mapping = codebook::resolve(&block.ty, code).ok_or_else(|| CastError::UnknownOpcode {
        ty: block.ty.clone(),
        code: code.map(str::to_owned),
    })?;

    // The immediates, in the block's own field order. A Selector dropdown
    // chose WHICH function and is already spent; a ValueParam dropdown is an
    // ARGUMENT and becomes a byte, or the cast refuses.
    let mut values = [0u8; ogar_loco::MAX_VALUES_PER_CALL];
    let mut n = 0usize;
    let mut push = |b: u8| -> Result<(), CastError> {
        // Silently dropping the overflow would emit a call that looks complete
        // and is not — the same failure the mutator guard above exists to stop.
        if n >= ogar_loco::MAX_VALUES_PER_CALL {
            return Err(CastError::TooManyValues {
                ty: block.ty.clone(),
                offered: block.fields.len(),
            });
        }
        values[n] = b;
        n += 1;
        Ok(())
    };

    for (name, v) in &block.fields {
        match v {
            FieldValue::Byte(b) => push(*b)?,
            FieldValue::Wide(text) => match ctx.as_deref_mut() {
                // No pool: the historical refusal. A guessed index is worse
                // than no answer.
                None => {
                    return Err(CastError::WideLiteral {
                        ty: block.ty.clone(),
                        value: text.clone(),
                    });
                }
                Some(cx) => {
                    // A Blockly field is a string even when it denotes a
                    // number, so the reading is decided HERE and named by the
                    // facet classid — never by a tag byte inside the payload,
                    // which would be a second schema under one classid.
                    let idx = match text.parse::<f64>() {
                        Ok(n) => cx.pool.intern_f64(cx.f64_classid, n),
                        Err(_) => cx.pool.intern_str(cx.utf8_classid, text),
                    }
                    .map_err(|e| CastError::ConstantPool {
                        ty: block.ty.clone(),
                        value: text.clone(),
                        source: e,
                    })?;
                    push(idx)?;
                }
            },
            FieldValue::Ref { id } => {
                return Err(CastError::UnresolvedRef {
                    ty: block.ty.clone(),
                    id: id.clone(),
                });
            }
            FieldValue::Code(c) => match mapping.role {
                // Spent by resolution — `logic_compare[LT]` IS `FnIndex::LT`.
                codebook::CodeRole::Selector => {}
                // An argument: its ordinal in the codebook's pinned option set.
                codebook::CodeRole::ValueParam => {
                    let byte =
                        codebook::encode_value_param(&block.ty, name, c).ok_or_else(|| {
                            CastError::UnencodedValueParam {
                                ty: block.ty.clone(),
                                code: c.clone(),
                            }
                        })?;
                    push(byte)?;
                }
                // A code on a block whose resolution ignores codes. Refused
                // rather than dropped: it means the record and the codebook
                // disagree about the block's shape.
                codebook::CodeRole::None => {
                    return Err(CastError::UnencodedValueParam {
                        ty: block.ty.clone(),
                        code: c.clone(),
                    });
                }
            },
        }
    }
    Ok(Call::with_values(mapping.function, values))
}

// ── The editing surface ─────────────────────────────────────────────────────

/// A workspace edit, in the two shapes the W1 falsifier distinguishes.
///
/// These mirror real Blockly events (`core/events/`):
///
/// - [`Drag`](WorkspaceEdit::Drag) is a `BlockMove` whose `reason` includes
///   `'drag'` with `oldParentId == newParentId` and only the coordinates
///   differing — a reposition with no structural change.
/// - [`SetField`](WorkspaceEdit::SetField) is a `BlockChange` with
///   `element == "field"`.
#[derive(Debug, Clone, PartialEq)]
pub enum WorkspaceEdit {
    /// Reposition a block. Presentation only.
    Drag {
        /// Which block moved.
        id: String,
        /// Horizontal delta, workspace units.
        dx: f64,
        /// Vertical delta.
        dy: f64,
    },
    /// Collapse or expand a block. Presentation only.
    SetCollapsed {
        /// Which block.
        id: String,
        /// The new state.
        collapsed: bool,
    },
    /// Change a field's value. Semantic.
    SetField {
        /// Which block.
        id: String,
        /// Which field.
        field: String,
        /// The new value.
        value: FieldValue,
    },
}

/// A script plus its per-block view state — the two halves held side by side
/// rather than interleaved.
///
/// This is what makes the W1 falsifier meaningful: edits are applied HERE, so
/// a handler that let a drag reach the record would be caught. Without an
/// apply step, "a drag changes nothing" is just `f(x) == f(x)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Workspace {
    /// The program.
    pub script: BlockRecord,
    /// Presentation, keyed by block id. Never an input to the cast.
    pub views: Vec<(String, BlockView)>,
}

impl Workspace {
    /// A workspace holding one script, with default views for every block.
    #[must_use]
    pub fn new(script: BlockRecord) -> Self {
        let mut views = Vec::new();
        collect_ids(&script, &mut views);
        Self { script, views }
    }

    /// The view state for a block id.
    #[must_use]
    pub fn view(&self, id: &str) -> Option<&BlockView> {
        self.views.iter().find(|(i, _)| i == id).map(|(_, v)| v)
    }

    /// Apply one edit.
    ///
    /// Presentation edits touch [`views`](Self::views) only; a semantic edit
    /// touches [`script`](Self::script) only. Returns whether anything changed.
    pub fn apply(&mut self, edit: &WorkspaceEdit) -> bool {
        match edit {
            WorkspaceEdit::Drag { id, dx, dy } => {
                if let Some((_, v)) = self.views.iter_mut().find(|(i, _)| i == id) {
                    v.x += dx;
                    v.y += dy;
                    return true;
                }
                false
            }
            WorkspaceEdit::SetCollapsed { id, collapsed } => {
                if let Some((_, v)) = self.views.iter_mut().find(|(i, _)| i == id) {
                    v.collapsed = *collapsed;
                    return true;
                }
                false
            }
            WorkspaceEdit::SetField { id, field, value } => {
                set_field(&mut self.script, id, field, value)
            }
        }
    }

    /// Cast the workspace's program into a function body. Views are not an
    /// input — they are not reachable from here.
    ///
    /// # Errors
    ///
    /// Whatever [`lower_script`] refuses.
    pub fn lower(&self, shape: LaneShape) -> Result<FunctionBody, CastError> {
        lower_script(shape, &self.script)
    }
}

fn collect_ids(block: &BlockRecord, out: &mut Vec<(String, BlockView)>) {
    out.push((block.id.clone(), BlockView::default()));
    for (_, b) in &block.inputs {
        collect_ids(b, out);
    }
    if let Some(n) = &block.next {
        collect_ids(n, out);
    }
}

fn set_field(block: &mut BlockRecord, id: &str, field: &str, value: &FieldValue) -> bool {
    if block.id == id {
        if let Some((_, v)) = block.fields.iter_mut().find(|(n, _)| n == field) {
            *v = value.clone();
            return true;
        }
        return false;
    }
    for (_, b) in &mut block.inputs {
        if set_field(b, id, field, value) {
            return true;
        }
    }
    if let Some(n) = &mut block.next {
        return set_field(n, id, field, value);
    }
    false
}

// ── The cast: up ────────────────────────────────────────────────────────────

/// A call read back out of a body, paired with the block type it came from.
///
/// The inverse cast is **not** total: a body knows its calls, but block ids and
/// input NAMES are presentation-adjacent and were never stored. Raising
/// therefore reconstructs the program's shape and semantics, and mints fresh
/// ids — which is correct, and is why the round-trip test asserts on calls
/// rather than on ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaisedCall {
    /// The function this call names.
    pub function: FnIndex,
    /// Its immediate value bytes, shape-truncated.
    pub values: Vec<u8>,
}

/// Read a body back as a flat call sequence.
///
/// Rebuilding the *tree* (which calls are operands of which) requires each
/// function's arity, which is a codebook property and lands with the arity
/// table — see OGAR `BLOCK-EDITOR-PLAN.md`. The flat sequence is exact and is
/// what the round-trip falsifier compares.
#[must_use]
pub fn raise_calls(body: &FunctionBody) -> Vec<RaisedCall> {
    let vals = body.shape().values_per_call();
    body.calls()
        .map(|c| RaisedCall {
            function: c.function,
            values: c.values[..vals].to_vec(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `5 + 3` as Blockly builds it: an `ADD` with two `math_number` operands.
    fn five_plus_three() -> BlockRecord {
        BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input(
                "A",
                BlockRecord::leaf("math_number", "a").with_field("NUM", FieldValue::Byte(5)),
            )
            .with_input(
                "B",
                BlockRecord::leaf("math_number", "b").with_field("NUM", FieldValue::Byte(3)),
            )
    }

    #[test]
    fn operands_are_emitted_before_their_operator() {
        // The post-order claim, and the anti-vacuity guard is the ORDER: a
        // pre-order walk would put ADD first and would fail here.
        let body = lower_script(LaneShape::Pairs, &five_plus_three()).unwrap();
        let calls = raise_calls(&body);
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].function, FnIndex::NUMBER);
        assert_eq!(calls[0].values, vec![5]);
        assert_eq!(calls[1].function, FnIndex::NUMBER);
        assert_eq!(calls[1].values, vec![3]);
        assert_eq!(
            calls[2].function,
            FnIndex::ADD,
            "the operator must come LAST"
        );
    }

    #[test]
    fn a_next_chain_becomes_sequential_calls() {
        let script = BlockRecord::leaf("controls_whileUntil", "w")
            .with_field("MODE", FieldValue::Code("WHILE".into()))
            .with_next(
                BlockRecord::leaf("controls_flow_statements", "b")
                    .with_field("FLOW", FieldValue::Code("BREAK".into())),
            );
        let body = lower_script(LaneShape::Pairs, &script).unwrap();
        let calls = raise_calls(&body);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].function, FnIndex::WHILE);
        assert_eq!(calls[1].function, FnIndex::BREAK);
    }

    // ── THE W1 FALSIFIER ────────────────────────────────────────────────────

    /// Byte positions where two bodies differ.
    fn byte_diff(a: &FunctionBody, b: &FunctionBody) -> Vec<usize> {
        a.as_body_bytes()
            .iter()
            .zip(b.as_body_bytes().iter())
            .enumerate()
            .filter(|(_, (x, y))| x != y)
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn a_drag_produces_zero_abi_writes() {
        // THE W1 FALSIFIER, first half. The edit is APPLIED to a workspace —
        // an earlier version of this test called lower() twice on the same
        // record, which is f(x) == f(x) and proves nothing. Going through
        // `apply` means a handler that let a drag reach the record fails here.
        let mut ws = Workspace::new(five_plus_three());
        let before = ws.lower(LaneShape::Pairs).unwrap();

        assert!(ws.apply(&WorkspaceEdit::Drag {
            id: "root".into(),
            dx: 40.0,
            dy: -17.5,
        }));
        assert!(ws.apply(&WorkspaceEdit::SetCollapsed {
            id: "a".into(),
            collapsed: true,
        }));

        // The edits really landed — otherwise "nothing changed" is free.
        assert_eq!(ws.view("root").unwrap().x, 40.0);
        assert!(ws.view("a").unwrap().collapsed);

        let after = ws.lower(LaneShape::Pairs).unwrap();
        assert_eq!(
            byte_diff(&before, &after),
            Vec::<usize>::new(),
            "a drag must not change one byte of the ABI"
        );
        // Anti-vacuity: a non-trivial body, or "identical" is free.
        assert_eq!(before.len(), 3);
        assert!(before.as_body_bytes().iter().any(|&b| b != 0));
    }

    #[test]
    fn an_operand_change_produces_exactly_one_abi_write() {
        // THE W1 FALSIFIER, second half — and the anti-vacuity twin of the
        // first: a cast that never reacted would pass the drag test trivially.
        // Same workspace, same apply path, one byte moves.
        let mut ws = Workspace::new(five_plus_three());
        let before = ws.lower(LaneShape::Pairs).unwrap();

        assert!(ws.apply(&WorkspaceEdit::SetField {
            id: "b".into(),
            field: "NUM".into(),
            value: FieldValue::Byte(4), // 3 → 4
        }));

        let after = ws.lower(LaneShape::Pairs).unwrap();
        let diff = byte_diff(&before, &after);
        assert_eq!(
            diff.len(),
            1,
            "an operand change must move exactly one byte, moved {diff:?}"
        );
        // And it must be the VALUE byte of call 1, not its function byte.
        assert_eq!(diff[0], 3, "the differing byte must be call 1's immediate");
        assert_eq!(after.as_body_bytes()[3], 4);
    }

    #[test]
    fn the_two_halves_of_the_falsifier_use_the_same_workspace_path() {
        // Guards the falsifier itself: both halves must route through `apply`,
        // so neither can pass because of how the test set its state up. A drag
        // and a field edit applied to the SAME workspace must produce
        // different write counts.
        let mut dragged = Workspace::new(five_plus_three());
        let base = dragged.lower(LaneShape::Pairs).unwrap();
        dragged.apply(&WorkspaceEdit::Drag {
            id: "root".into(),
            dx: 1.0,
            dy: 1.0,
        });

        let mut edited = Workspace::new(five_plus_three());
        edited.apply(&WorkspaceEdit::SetField {
            id: "b".into(),
            field: "NUM".into(),
            value: FieldValue::Byte(9),
        });

        assert_eq!(
            byte_diff(&base, &dragged.lower(LaneShape::Pairs).unwrap()).len(),
            0
        );
        assert_eq!(
            byte_diff(&base, &edited.lower(LaneShape::Pairs).unwrap()).len(),
            1
        );
    }

    #[test]
    fn applying_an_edit_to_an_unknown_id_reports_no_change() {
        // The can-stay-silent half of `apply`: it must not claim success for a
        // block that isn't there, or the falsifier's `assert!(ws.apply(..))`
        // would be worthless.
        let mut ws = Workspace::new(five_plus_three());
        assert!(!ws.apply(&WorkspaceEdit::Drag {
            id: "nope".into(),
            dx: 1.0,
            dy: 1.0,
        }));
        assert!(!ws.apply(&WorkspaceEdit::SetField {
            id: "nope".into(),
            field: "NUM".into(),
            value: FieldValue::Byte(1),
        }));
        // ...and a field that doesn't exist on a block that does.
        assert!(!ws.apply(&WorkspaceEdit::SetField {
            id: "root".into(),
            field: "NOT_A_FIELD".into(),
            value: FieldValue::Byte(1),
        }));
    }

    #[test]
    fn changing_the_operator_also_moves_exactly_one_byte() {
        // The dropdown half of the same claim: ADD → MINUS moves the FUNCTION
        // byte, not a value byte. Together with the test above this proves the
        // cast distinguishes the two halves of a call.
        let before = lower_script(LaneShape::Pairs, &five_plus_three()).unwrap();
        let mut changed = five_plus_three();
        changed.fields[0].1 = FieldValue::Code("MINUS".into());
        let after = lower_script(LaneShape::Pairs, &changed).unwrap();

        let diff: Vec<usize> = before
            .as_body_bytes()
            .iter()
            .zip(after.as_body_bytes().iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            diff.len(),
            1,
            "an operator change must move exactly one byte"
        );
        assert_eq!(
            diff[0], 4,
            "the differing byte must be call 2's FUNCTION byte"
        );
        assert_eq!(after.as_body_bytes()[4], FnIndex::SUB.0);
    }

    #[test]
    fn an_unknown_opcode_is_refused_not_guessed() {
        // A codebook id is permanent; an invented mapping is worse than a
        // refusal. Two-sided: a known block must still succeed.
        let bogus = BlockRecord::leaf("totally_not_a_block", "x");
        match lower_script(LaneShape::Pairs, &bogus) {
            Err(CastError::UnknownOpcode { ty, code }) => {
                assert_eq!(ty, "totally_not_a_block");
                assert_eq!(code, None);
            }
            other => panic!("unknown opcode must be refused, got {other:?}"),
        }
        assert!(lower_script(LaneShape::Pairs, &five_plus_three()).is_ok());
    }

    #[test]
    fn a_wide_literal_is_refused_rather_than_truncated() {
        // The constant pool is a named follow-up; silently narrowing a literal
        // to its low byte would be the truncation defect one layer up.
        let big = BlockRecord::leaf("math_number", "n")
            .with_field("NUM", FieldValue::Wide("1000".into()));
        match lower_script(LaneShape::Pairs, &big) {
            Err(CastError::WideLiteral { value, .. }) => assert_eq!(value, "1000"),
            other => panic!("a wide literal must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_program_past_the_call_budget_is_refused() {
        // The split-don't-widen rule reaching the cast. Build a chain longer
        // than Pairs' 180-call budget.
        let mut script = BlockRecord::leaf("controls_flow_statements", "b0")
            .with_field("FLOW", FieldValue::Code("BREAK".into()));
        for i in 1..LaneShape::Pairs.calls_per_function() + 1 {
            let mut next = BlockRecord::leaf("controls_flow_statements", format!("b{i}"));
            next.fields
                .push(("FLOW".into(), FieldValue::Code("BREAK".into())));
            // Append at the tail.
            let mut cur = &mut script;
            while cur.next.is_some() {
                cur = cur.next.as_mut().unwrap();
            }
            cur.next = Some(Box::new(next));
        }
        assert!(matches!(
            lower_script(LaneShape::Pairs, &script),
            Err(CastError::Body(BodyError::Overflow { .. }))
        ));
    }

    #[test]
    fn a_variable_reference_is_refused_rather_than_slotted() {
        // This crate does not own the variable table. A guessed slot index is
        // the same class of defect as a guessed opcode.
        let get = BlockRecord::leaf("variables_get", "g").with_field(
            "VAR",
            FieldValue::Ref {
                id: "abc123".into(),
            },
        );
        match lower_script(LaneShape::Pairs, &get) {
            Err(CastError::UnresolvedRef { ty, id }) => {
                assert_eq!(ty, "variables_get");
                assert_eq!(id, "abc123");
            }
            other => panic!("a variable ref must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_mutator_payload_is_refused_rather_than_silently_dropped() {
        // extraState decides how many sockets the block has. Emitting a call
        // without it would look complete and be wrong. Two-sided: the SAME
        // block without the payload must lower fine.
        let plain = BlockRecord::leaf("controls_if", "i");
        assert!(lower_script(LaneShape::Pairs, &plain).is_ok());

        let mutated = BlockRecord::leaf("controls_if", "i").with_extra_state("{\"elseIfCount\":2}");
        match lower_script(LaneShape::Pairs, &mutated) {
            Err(CastError::MutatorUnsupported { ty }) => assert_eq!(ty, "controls_if"),
            other => panic!("a mutator payload must be refused, got {other:?}"),
        }
    }

    #[test]
    fn a_value_param_dropdown_encodes_rather_than_conflating() {
        // RE-PINNED, deliberately. This test previously asserted the dropdown
        // was REFUSED (`UnencodedValueParam`), because no byte encoding
        // existed. The encoding now exists, so that assertion became wrong —
        // but the PROPERTY it stood in for did not change, and is asserted
        // here directly instead of through the refusal that proxied it:
        // math_constant[PI] and math_constant[E] are different programs.
        let of = |code: &str| {
            let b = BlockRecord::leaf("math_constant", "c")
                .with_field("CONSTANT", FieldValue::Code(code.into()));
            raise_calls(&lower_script(LaneShape::Pairs, &b).unwrap())
        };
        let pi = of("PI");
        let e = of("E");
        assert_eq!(pi.len(), 1);
        // Same function — a ValueParam must NOT fan the palette out…
        assert_eq!(pi[0].function, e[0].function);
        assert_eq!(pi[0].function, ogar_loco::FnIndex::CONSTANT);
        // …and the two programs stay distinguishable anyway, which is the
        // whole point. Drop the code and these become equal.
        assert_ne!(pi[0].values[0], e[0].values[0]);
        assert_eq!(pi[0].values[0], 0);
        assert_eq!(e[0].values[0], 1);

        // Silence twin: a code the option set does not name is still refused,
        // not defaulted to ordinal 0.
        let tau = BlockRecord::leaf("math_constant", "c")
            .with_field("CONSTANT", FieldValue::Code("TAU".into()));
        assert!(matches!(
            lower_script(LaneShape::Pairs, &tau),
            Err(CastError::UnencodedValueParam { .. })
        ));

        // The Selector case is untouched: its code is spent by resolution and
        // never becomes an immediate.
        let lt =
            BlockRecord::leaf("logic_compare", "c").with_field("OP", FieldValue::Code("LT".into()));
        let calls = raise_calls(&lower_script(LaneShape::Pairs, &lt).unwrap());
        assert_eq!(calls[0].function, ogar_loco::FnIndex::LT);
        assert_eq!(calls[0].values[0], 0);
    }

    #[test]
    fn the_three_codebook_gaps_surface_as_refusals_at_the_cast() {
        // The gaps are refused end-to-end, not just in the codebook — and each
        // names itself, so a caller learns WHICH block is unmintable.
        for (ty, code) in [
            ("lists_reverse", None),
            ("math_on_list", Some("RANDOM")),
            ("lists_getIndex", Some("GET_REMOVE")),
        ] {
            let mut b = BlockRecord::leaf(ty, "x");
            if let Some(c) = code {
                b = b.with_field("OP", FieldValue::Code(c.into()));
            }
            match lower_script(LaneShape::Pairs, &b) {
                Err(CastError::UnknownOpcode { ty: got, .. }) => assert_eq!(got, ty),
                other => panic!("{ty} must be refused, got {other:?}"),
            }
        }
    }

    #[test]
    fn block_count_predicts_the_call_count() {
        // A cheap pre-flight a caller can run before casting: does this script
        // fit? Must agree with what the cast actually emits.
        let script = five_plus_three();
        assert_eq!(script.block_count(), 3);
        let body = lower_script(LaneShape::Pairs, &script).unwrap();
        assert_eq!(body.len(), script.block_count());
    }

    // ── the constant pool, end to end ───────────────────────────────────────

    #[test]
    fn a_narrow_program_produces_an_empty_pool_and_an_identical_body() {
        // The SILENCE half, and the reason it uses `five_plus_three()` rather
        // than an empty script: an empty program trivially has an empty pool
        // and proves only that nothing crashed. This input really does carry
        // literals — they are just narrow enough to stay immediates.
        let script = five_plus_three();
        let plain = lower_script(LaneShape::Pairs, &script).unwrap();

        let mut ctx = LoweringContext::placeholder();
        let pooled = lower_script_with_pool(LaneShape::Pairs, &script, &mut ctx).unwrap();

        assert!(
            ctx.pool.is_empty(),
            "a narrow literal must not reach the pool"
        );
        assert_eq!(
            plain.as_body_bytes(),
            pooled.as_body_bytes(),
            "enabling the pool changed a program that has no wide literals"
        );
        // And the body really is the pinned one, so "identical" is not two
        // identically-broken results.
        assert_eq!(&pooled.as_body_bytes()[..6], &[0x46, 5, 0x46, 3, 0x40, 0]);
    }

    #[test]
    fn a_wide_literal_interns_and_the_call_carries_its_index() {
        // The FIRE half. Two DISTINCT wide literals, because with only one an
        // "index was assigned" assertion is free — there is nothing it could
        // be confused with.
        let script = BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input(
                "A",
                BlockRecord::leaf("math_number", "a")
                    .with_field("NUM", FieldValue::Wide("7.25".into())),
            )
            .with_input(
                "B",
                BlockRecord::leaf("math_number", "b")
                    .with_field("NUM", FieldValue::Wide("1000000".into())),
            );

        // Without a pool this is still refused — the default did not move.
        assert!(matches!(
            lower_script(LaneShape::Pairs, &script),
            Err(CastError::WideLiteral { .. })
        ));

        let mut ctx = LoweringContext::placeholder();
        let body = lower_script_with_pool(LaneShape::Pairs, &script, &mut ctx).unwrap();
        let calls = raise_calls(&body);
        assert_eq!(calls.len(), 3);
        assert_eq!(ctx.pool.len(), 2);

        // The two index bytes must DIFFER — a pool returning a fixed index
        // would lower both literals identically and still "succeed".
        let (a, b) = (calls[0].values[0], calls[1].values[0]);
        assert_ne!(a, b, "two distinct literals collapsed onto one index");
        // Neither is the zero-fallback.
        assert_ne!(a, 0);
        assert_ne!(b, 0);

        // …and they read back BIT-EXACT under the right classid. Asserting
        // only `is_ok()` would be truncation wearing a pool costume.
        let f = |idx: u8| {
            let c = ctx.pool.resolve(idx).unwrap();
            assert_eq!(c.classid, ogar_loco::pool::placeholder::CONST_F64);
            f64::from_le_bytes(c.bytes[..8].try_into().unwrap())
        };
        assert_eq!(f(a), 7.25);
        assert_eq!(f(b), 1_000_000.0);
    }

    #[test]
    fn a_string_literal_takes_the_utf8_classid_not_the_f64_one() {
        // The reading is decided at the cast and NAMED by the facet classid.
        // A tag byte inside the payload would be a second schema under one
        // classid, which is what "your classid defines the schema" forbids.
        let script =
            BlockRecord::leaf("text", "t").with_field("TEXT", FieldValue::Wide("hello".into()));
        let mut ctx = LoweringContext::placeholder();
        let body = lower_script_with_pool(LaneShape::Pairs, &script, &mut ctx).unwrap();
        let idx = raise_calls(&body)[0].values[0];
        let c = ctx.pool.resolve(idx).unwrap();
        assert_eq!(c.classid, ogar_loco::pool::placeholder::CONST_UTF8_INLINE);
        assert_eq!(&c.bytes[..5], b"hello");

        // Two-sided: a numeric-looking string still goes to the f64 side, so
        // the classid is chosen by the VALUE and not by the block type.
        let n = BlockRecord::leaf("text", "t2").with_field("TEXT", FieldValue::Wide("42".into()));
        let body2 = lower_script_with_pool(LaneShape::Pairs, &n, &mut ctx).unwrap();
        let idx2 = raise_calls(&body2)[0].values[0];
        assert_eq!(
            ctx.pool.resolve(idx2).unwrap().classid,
            ogar_loco::pool::placeholder::CONST_F64
        );
    }

    #[test]
    fn the_same_wide_literal_used_twice_interns_once() {
        // Content-addressing end to end — and the reason the repoint-never-
        // mutate rule exists: one entry may have several referents, so an
        // in-place edit would change a call site the user did not touch.
        let script = BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input(
                "A",
                BlockRecord::leaf("math_number", "a")
                    .with_field("NUM", FieldValue::Wide("7.25".into())),
            )
            .with_input(
                "B",
                BlockRecord::leaf("math_number", "b")
                    .with_field("NUM", FieldValue::Wide("7.25".into())),
            );
        let mut ctx = LoweringContext::placeholder();
        let body = lower_script_with_pool(LaneShape::Pairs, &script, &mut ctx).unwrap();
        let calls = raise_calls(&body);
        assert_eq!(ctx.pool.len(), 1, "one value must occupy one slot");
        assert_eq!(calls[0].values[0], calls[1].values[0]);
    }

    #[test]
    fn an_oversized_literal_is_refused_by_the_cast_too() {
        // The refusal survives the pool: a literal wider than a facet does not
        // silently truncate, which would be worse than today's WideLiteral
        // error because it would look like success.
        let long = "x".repeat(13);
        let script = BlockRecord::leaf("text", "t").with_field("TEXT", FieldValue::Wide(long));
        let mut ctx = LoweringContext::placeholder();
        assert!(matches!(
            lower_script_with_pool(LaneShape::Pairs, &script, &mut ctx),
            Err(CastError::ConstantPool {
                source: PoolError::TooWide { needed: 13 },
                ..
            })
        ));
        assert!(
            ctx.pool.is_empty(),
            "the refused literal must not have landed"
        );
        // Twelve still fits, so the guard discriminates rather than refusing
        // every string.
        let ok =
            BlockRecord::leaf("text", "t").with_field("TEXT", FieldValue::Wide("x".repeat(12)));
        assert!(lower_script_with_pool(LaneShape::Pairs, &ok, &mut ctx).is_ok());
    }
}
