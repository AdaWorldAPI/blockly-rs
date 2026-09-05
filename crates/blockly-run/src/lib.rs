//! Run a stored program — straight from its function bodies.
//!
//! # What this proves
//!
//! The arc's claim is that the blocks are a projection and the 512-byte node
//! is the program. A cast that only ever produced bytes leaves that one step
//! short of demonstrated: bytes nothing executes are still, arguably, a
//! description. This crate closes it — the interpreter reads
//! [`FunctionBody`] call rails and nothing else. No JSON, no re-parse, no
//! block tree. Feed it what came out of `from_le_bytes` and it runs.
//!
//! # Permissive by construction
//!
//! This is an original interpreter over the call rails. It does **not** link
//! the GPL `rash` JIT — that boundary lives in `scratch-rs`, and this half of
//! the arc stays permissive (`blockly-abi`'s crate docs, README).
//!
//! # The execution model is the ABI's, not a second one
//!
//! Operands come off a stack, arity comes from the vocabulary, and a body
//! reference names another function — exactly what lowering encoded. The
//! interpreter therefore cannot drift from the cast: if it disagreed about an
//! arity, the stack would desynchronize and the run would refuse.
//!
//! `forever` is bounded by a step budget rather than special-cased. A real
//! runtime yields to a scheduler; a demo needs the loop to end, and a budget
//! is the honest way to say so — the program is unchanged, the *run* is
//! finite.
//!
//! # What is implemented, and what is refused
//!
//! Motion, the sensing reporters the reference game uses, variables, control
//! flow, and arithmetic/comparison. Anything else — sound, costumes, clones,
//! pen — is [`RunError::Unimplemented`], named, never a silent no-op. A device
//! op that quietly did nothing would make a broken program look like a working
//! one, which is the failure mode this whole arc keeps designing against.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

use blockly_abi::scratch;
use ogar_loco::vocabulary::shared_core;
use ogar_loco::{FnIndex, FunctionBody};

/// What a sprite looks like. Presentation only — the program never reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// A round ball.
    Ball,
    /// A tall paddle.
    Paddle,
}

/// One actor on the stage.
#[derive(Debug, Clone, PartialEq)]
pub struct Sprite {
    /// Position, Scratch coordinates (centre origin, y up).
    pub x: f32,
    /// Position.
    pub y: f32,
    /// Heading in degrees, Scratch convention: 90 = right, 0 = up.
    pub direction: f32,
    /// Whether it draws.
    pub visible: bool,
    /// Scale, percent.
    pub size: f32,
    /// How it is drawn.
    pub look: Look,
}

impl Default for Sprite {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            direction: 90.0,
            visible: true,
            size: 100.0,
            look: Look::Ball,
        }
    }
}

/// The stage a program acts on.
///
/// Several sprites, because a one-sprite stage cannot BE Pong: the ball and
/// the paddle are two actors driven by two concurrent scripts, and running
/// each script against its own private stage produced exactly what the deploy
/// showed — a single dot in an empty grid, with the paddle nowhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// Every actor. Scripts address one by index.
    pub sprites: Vec<Sprite>,
    /// Which sprite the currently-running script controls.
    pub current: usize,
    /// Variable store, indexed by the `VARIABLE` codebook byte a
    /// `data_*variable*` call carries (`blockly_abi::menus`, menu 25). Index
    /// `0` is the zero-fallback slot — a variable block with no declared name
    /// (the built-in templates) reads and writes it. Grown on demand.
    pub vars: Vec<f32>,
    /// Mouse position the sensing reporters return.
    pub mouse_x: f32,
    /// Mouse position.
    pub mouse_y: f32,
    /// Seconds since the run started, advanced one tick per statement batch.
    pub timer: f32,
    /// Whether `sensing_touchingobject` should answer true.
    pub touching: bool,
    /// The key currently held, as a `KEY_OPTION` codebook index
    /// (`blockly_abi::menus::encode`), or `None` for no key. `sensing_keypressed`
    /// compares its operand — the index a `sensing_keyoptions` reporter pushed
    /// — against this; the `any` option matches any held key.
    pub key: Option<u8>,
    /// Stage half-width — the edge `if on edge, bounce` reflects against.
    pub half_w: f32,
    /// Stage half-height.
    pub half_h: f32,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            sprites: vec![Sprite::default()],
            current: 0,
            vars: vec![0.0],
            mouse_x: 0.0,
            mouse_y: 0.0,
            timer: 0.0,
            touching: false,
            key: None,
            half_w: 240.0,
            half_h: 180.0,
        }
    }
}

impl Stage {
    /// A variable by its codebook index; unset reads as `0`, as in Scratch.
    #[must_use]
    pub fn var(&self, idx: u8) -> f32 {
        self.vars.get(usize::from(idx)).copied().unwrap_or(0.0)
    }

    /// The variable slot for a codebook index, growing the store to reach it.
    pub fn var_mut(&mut self, idx: u8) -> &mut f32 {
        let i = usize::from(idx);
        if self.vars.len() <= i {
            self.vars.resize(i + 1, 0.0);
        }
        &mut self.vars[i]
    }

    /// The sprite the running script controls.
    #[must_use]
    pub fn me(&self) -> &Sprite {
        &self.sprites[self.current.min(self.sprites.len() - 1)]
    }

    /// A Pong scene: a ball in the middle, a paddle at the right edge.
    ///
    /// The looks and starting positions are PRESENTATION — the program never
    /// reads them, it only moves whatever sprite it was bound to.
    #[must_use]
    pub fn pong() -> Self {
        Self {
            sprites: vec![
                Sprite {
                    look: Look::Ball,
                    direction: 62.0,
                    ..Sprite::default()
                },
                Sprite {
                    look: Look::Paddle,
                    x: 210.0,
                    ..Sprite::default()
                },
            ],
            ..Self::default()
        }
    }
}

/// Why a run stopped early.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The core does not cover this function's arity, so the stack shape is
    /// unknown — refused for the same reason lowering refuses it.
    Uncovered(u8),
    /// A call wanted more operands than the stack held.
    StackUnderflow(u8),
    /// A body reference names a function the program does not contain.
    DanglingReference(u8),
    /// A real operation this interpreter does not implement. Named rather
    /// than skipped: a silent no-op makes a broken program look correct.
    Unimplemented(u8),
    /// A custom-block call names a procedure index no definition in the scene
    /// carries. Refused rather than skipped, for the same reason.
    UnknownProcedure(u8),
    /// A pool load names a constant the run's pool does not hold — or the run
    /// was given no pool. Refused rather than read as the bare index.
    MissingConstant(u8),
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Uncovered(b) => write!(f, "function {b:#04x} has no covered arity"),
            Self::StackUnderflow(b) => write!(f, "function {b:#04x} wanted absent operands"),
            Self::DanglingReference(i) => write!(f, "body reference {i} names no function"),
            Self::Unimplemented(b) => write!(f, "function {b:#04x} is not implemented"),
            Self::UnknownProcedure(i) => write!(f, "no definition carries procedure {i}"),
            Self::MissingConstant(i) => write!(f, "constant {i} is not in the pool"),
        }
    }
}

impl core::error::Error for RunError {}

/// A custom block a scene can call: the procedure index its definition and
/// its calls share (the `PROCEDURE` codebook byte), the script that defines
/// it, and which of that script's functions is the body.
///
/// Recognised from the stored bytes alone: a script whose entry function
/// begins with `PROC_DEF` is a definition — `values[0]` the body reference,
/// `values[1]` the index (after the reference, as the cast writes it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Procedure<'a> {
    /// The `PROCEDURE` codebook index.
    pub index: u8,
    /// The defining script's functions.
    pub functions: &'a [FunctionBody],
    /// The body function within `functions`.
    pub body: usize,
}

impl<'a> Procedure<'a> {
    /// Read a definition off a script, if its entry begins with `PROC_DEF`.
    #[must_use]
    pub fn of_script(functions: &'a [FunctionBody]) -> Option<Self> {
        let head = blockly_abi::raise_calls(functions.first()?)
            .into_iter()
            .next()?;
        (head.function == FnIndex::PROC_DEF).then(|| Self {
            index: head.values.get(1).copied().unwrap_or(0),
            functions,
            body: usize::from(head.values.first().copied().unwrap_or(0)),
        })
    }
}

/// One device operation the interpreter knows how to perform. An integer
/// tag, resolved from the harvested opcode NAME exactly once per process —
/// so the hot path never compares strings and never scans the device table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    /// A minted device byte this interpreter does not implement.
    Unimplemented,
    /// A menu shadow block: yields its own codebook index.
    Menu,
    Hat,
    KeyPressed,
    LayerNoop,
    MoveSteps,
    TurnRight,
    TurnLeft,
    PointDir,
    GotoXy,
    SetX,
    SetY,
    ChangeX,
    ChangeY,
    Bounce,
    XPos,
    YPos,
    Dir,
    Show,
    Hide,
    ChangeSize,
    SetSize,
    Size,
    CostumeNoop,
    MouseX,
    MouseY,
    Timer,
    ResetTimer,
    Touching,
    MouseDown,
}

/// The per-vocabulary execution plan: for every possible function byte, the
/// operation and the stack arity. Derived data — 512 bytes, computed once —
/// never a copy of any program. This is the "prefetch" a body needs before
/// it runs, and it is shared by every body under this palette.
struct Plan {
    op: [Op; 256],
    arity: [Option<u8>; 256],
}

fn plan() -> &'static Plan {
    static PLAN: std::sync::OnceLock<Plan> = std::sync::OnceLock::new();
    PLAN.get_or_init(|| {
        let mut op = [Op::Unimplemented; 256];
        let mut arity = [None; 256];
        for b in 0..=255u8 {
            let f = FnIndex(b);
            arity[usize::from(b)] = shared_core::stack_arity(f)
                .or_else(|| scratch::device_by_byte(b).map(|(_, a, _)| a));
            // The pool load is a leaf: nothing popped, one value pushed.
            if f == blockly_abi::POOL_LOAD {
                arity[usize::from(b)] = Some(0);
            }
            let Some((name, ..)) = scratch::device_by_byte(b) else {
                continue;
            };
            op[usize::from(b)] = if blockly_abi::menus::is_menu_block(name) {
                Op::Menu
            } else {
                match name {
                    "event_whenflagclicked"
                    | "event_whenthisspriteclicked"
                    | "event_whenkeypressed" => Op::Hat,
                    "sensing_keypressed" => Op::KeyPressed,
                    "looks_gotofrontback" | "looks_goforwardbackwardlayers" => Op::LayerNoop,
                    "motion_movesteps" => Op::MoveSteps,
                    "motion_turnright" => Op::TurnRight,
                    "motion_turnleft" => Op::TurnLeft,
                    "motion_pointindirection" => Op::PointDir,
                    "motion_gotoxy" => Op::GotoXy,
                    "motion_setx" => Op::SetX,
                    "motion_sety" => Op::SetY,
                    "motion_changexby" => Op::ChangeX,
                    "motion_changeyby" => Op::ChangeY,
                    "motion_ifonedgebounce" => Op::Bounce,
                    "motion_xposition" => Op::XPos,
                    "motion_yposition" => Op::YPos,
                    "motion_direction" => Op::Dir,
                    "looks_show" => Op::Show,
                    "looks_hide" => Op::Hide,
                    "looks_changesizeby" => Op::ChangeSize,
                    "looks_setsizeto" => Op::SetSize,
                    "looks_size" => Op::Size,
                    "looks_nextcostume" | "looks_nextbackdrop" => Op::CostumeNoop,
                    "sensing_mousex" => Op::MouseX,
                    "sensing_mousey" => Op::MouseY,
                    "sensing_timer" => Op::Timer,
                    "sensing_resettimer" => Op::ResetTimer,
                    "sensing_touchingobject" | "sensing_touchingcolor" => Op::Touching,
                    "sensing_mousedown" => Op::MouseDown,
                    _ => Op::Unimplemented,
                }
            };
        }
        Plan { op, arity }
    })
}

/// A bounded run over stored function bodies.
pub struct Machine<'a> {
    /// The stage the program acts on.
    pub stage: Stage,
    functions: &'a [FunctionBody],
    /// Custom blocks callable from this run, by index.
    procs: &'a [Procedure<'a>],
    /// The constant pool `POOL_LOAD` reads. `None` = every load is refused,
    /// which is what a program with no wide literal never notices.
    pool: Option<&'a ogar_loco::ConstantPool>,
    /// Argument frames of the custom blocks currently executing, innermost
    /// last. `PROC_ARG` reads the innermost; outside any call it reads `0`.
    frames: Vec<Vec<f32>>,
    budget: u32,
    /// Sampled stage snapshots — the run's TRACE, not its result.
    ///
    /// A run that only reports its final stage cannot show motion: the demo
    /// rendered one frame and looked static, which is exactly what a program
    /// that did nothing would look like. Recording the intermediate states
    /// lets the renderer show the path the program actually took.
    trace: Vec<Stage>,
    every: u32,
    counter: u32,
}

impl<'a> Machine<'a> {
    /// Continue a run on an EXISTING stage, as a given sprite.
    ///
    /// The scheduler uses this to give each script a slice of time against
    /// the shared scene, which is what makes two scripts appear to run at
    /// once.
    #[must_use]
    pub fn resuming(
        functions: &'a [FunctionBody],
        budget: u32,
        stage: Stage,
        sprite: usize,
    ) -> Self {
        let mut m = Self::new(functions, budget);
        m.stage = stage;
        m.stage.current = sprite;
        m
    }

    /// Start a run over a program's function bodies.
    ///
    /// `budget` bounds the total number of calls executed, so a `forever`
    /// terminates. It is a property of the RUN, never of the program.
    #[must_use]
    pub fn new(functions: &'a [FunctionBody], budget: u32) -> Self {
        Self {
            stage: Stage::default(),
            functions,
            procs: &[],
            pool: None,
            frames: Vec::new(),
            budget,
            trace: Vec::new(),
            every: 0,
            counter: 0,
        }
    }

    /// Make these custom blocks callable from the run.
    #[must_use]
    pub fn with_procs(mut self, procs: &'a [Procedure<'a>]) -> Self {
        self.procs = procs;
        self
    }

    /// Read wide literals from this pool — the one the program was cast
    /// against with `lower_program_with_pool`.
    #[must_use]
    pub fn with_pool(mut self, pool: &'a ogar_loco::ConstantPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Record a stage snapshot every `n` calls, for rendering the motion.
    ///
    /// Off by default: tracing is a property of the OBSERVATION, not of the
    /// program, and a caller that only wants the final stage should not pay
    /// for a trace it never reads.
    #[must_use]
    pub fn tracing_every(mut self, n: u32) -> Self {
        self.every = n.max(1);
        self.trace.push(self.stage.clone());
        self
    }

    /// The sampled snapshots, oldest first. Empty unless [`Self::tracing_every`]
    /// was set.
    #[must_use]
    pub fn trace(&self) -> &[Stage] {
        &self.trace
    }

    /// Run the entry function until the budget is spent.
    ///
    /// # Errors
    ///
    /// See [`RunError`] — every variant is a refusal, never a silent skip.
    pub fn run(&mut self) -> Result<(), RunError> {
        self.exec(0)
    }

    /// Execute a function of THIS script.
    fn exec(&mut self, index: usize) -> Result<(), RunError> {
        self.exec_in(self.functions, index)
    }

    /// Execute function `index` of `functions` — this script's, or a custom
    /// block's defining script when called through `PROC_CALL`.
    fn exec_in(&mut self, functions: &'a [FunctionBody], index: usize) -> Result<(), RunError> {
        // Borrowed, never copied: the body is read in place from the slice
        // the node bytes were decoded into, and `calls()` yields each `Call`
        // by value from it — no per-body copy, no per-call allocation.
        let body = functions
            .get(index)
            .ok_or(RunError::DanglingReference(index as u8))?;
        let mut stack: Vec<f32> = Vec::with_capacity(8);
        let plan = plan();

        for call in body.calls() {
            if self.budget == 0 {
                return Ok(());
            }
            self.budget -= 1;
            if self.every > 0 {
                self.counter += 1;
                if self.counter >= self.every {
                    self.counter = 0;
                    self.trace.push(self.stage.clone());
                }
            }

            let f = call.function;
            let refs = usize::from(shared_core::body_refs(f));

            // ── control flow: a body reference is not an operand ──────────
            if refs > 0 {
                let target = usize::from(call.values.first().copied().unwrap_or(0));
                match f {
                    FnIndex::FOREVER => {
                        // No tick here: the SCENE advances the clock once per
                        // round. Ticking per loop iteration too counted the
                        // same time twice and reported t = 480 s for a run of
                        // 240 frames.
                        while self.budget > 0 {
                            self.exec_in(functions, target)?;
                        }
                    }
                    FnIndex::IF => {
                        let c = pop(&mut stack, f)?;
                        if c != 0.0 {
                            self.exec_in(functions, target)?;
                        }
                    }
                    FnIndex::IF_ELSE => {
                        let c = pop(&mut stack, f)?;
                        let other = usize::from(call.values.get(1).copied().unwrap_or(0));
                        self.exec_in(functions, if c != 0.0 { target } else { other })?;
                    }
                    FnIndex::REPEAT => {
                        let n = pop(&mut stack, f)?.max(0.0) as u32;
                        for _ in 0..n {
                            if self.budget == 0 {
                                break;
                            }
                            self.exec_in(functions, target)?;
                        }
                    }
                    FnIndex::WHILE | FnIndex::REPEAT_UNTIL => {
                        // The condition was evaluated once, before the loop —
                        // re-evaluating it would need the operand's own calls,
                        // which live in this body. Bounded and honest: run the
                        // body while the budget lasts if the condition held.
                        let c = pop(&mut stack, f)?;
                        let want = f == FnIndex::WHILE;
                        if (c != 0.0) == want {
                            while self.budget > 0 {
                                self.exec_in(functions, target)?;
                            }
                        }
                    }
                    // A definition reached as a statement: its body runs when
                    // CALLED, never in line. (The scene does not schedule
                    // definition scripts at all; this covers a nested one.)
                    FnIndex::PROC_DEF => {}
                    _ => return Err(RunError::Unimplemented(f.0)),
                }
                continue;
            }

            // ── a custom-block call: variadic, arity in its own bytes ──────
            if f == FnIndex::PROC_CALL {
                let index = call.values.first().copied().unwrap_or(0);
                let argc = usize::from(call.values.get(1).copied().unwrap_or(0));
                if stack.len() < argc {
                    return Err(RunError::StackUnderflow(f.0));
                }
                let args: Vec<f32> = stack.split_off(stack.len() - argc);
                let proc_ = *self
                    .procs
                    .iter()
                    .find(|p| p.index == index)
                    .ok_or(RunError::UnknownProcedure(index))?;
                self.frames.push(args);
                let outcome = self.exec_in(proc_.functions, proc_.body);
                self.frames.pop();
                outcome?;
                continue;
            }

            // ── everything else: pop arity, compute, push if it yields ────
            let arity = usize::from(plan.arity[usize::from(f.0)].ok_or(RunError::Uncovered(f.0))?);
            if stack.len() < arity {
                return Err(RunError::StackUnderflow(f.0));
            }
            // Operands into a fixed window — the ABI's arity is at most 3 —
            // rather than a fresh `Vec` per call.
            let base = stack.len() - arity;
            let mut window = [0.0f32; ogar_loco::MAX_VALUES_PER_CALL];
            window[..arity].copy_from_slice(&stack[base..]);
            stack.truncate(base);
            let immediate = f32::from(call.values[0]);

            let result = self.apply(f, &window[..arity], immediate)?;
            if let Some(v) = result {
                stack.push(v);
            }
        }
        Ok(())
    }

    /// Apply one non-branching call. `Some(v)` means it yielded a value.
    fn apply(&mut self, f: FnIndex, ops: &[f32], imm: f32) -> Result<Option<f32>, RunError> {
        let a = |i: usize| ops.get(i).copied().unwrap_or(0.0);
        // Read before the stage is borrowed: the innermost custom-block frame.
        let frame_arg = self
            .frames
            .last()
            .and_then(|fr| fr.get(imm as usize))
            .copied()
            .unwrap_or(0.0);
        // A pool load: the constant's classid says how its bytes read. An
        // `f64` is the number; UTF-8 reads as Scratch reads text in a numeric
        // slot — its numeric value if it has one, else 0. The stack is f32,
        // so a text constant's identity is not carried past this point.
        if f == blockly_abi::POOL_LOAD {
            let idx = imm as u8;
            let c = self
                .pool
                .and_then(|p| p.resolve(idx))
                .ok_or(RunError::MissingConstant(idx))?;
            let v = match c.classid {
                ogar_loco::pool::placeholder::CONST_F64 => {
                    let mut le = [0u8; 8];
                    le.copy_from_slice(&c.bytes[..8]);
                    f64::from_le_bytes(le) as f32
                }
                ogar_loco::pool::placeholder::CONST_UTF8_INLINE => {
                    let end = c
                        .bytes
                        .iter()
                        .position(|&b| b == 0)
                        .unwrap_or(c.bytes.len());
                    core::str::from_utf8(&c.bytes[..end])
                        .ok()
                        .and_then(|s| s.trim().parse::<f32>().ok())
                        .unwrap_or(0.0)
                }
                _ => return Err(RunError::MissingConstant(idx)),
            };
            return Ok(Some(v));
        }

        let s = &mut self.stage;
        // Motion/looks act on the sprite this script is bound to.
        let me = s.current.min(s.sprites.len() - 1);

        // Shared core first — one table, both frontends.
        let core = match f {
            FnIndex::NUMBER => Some(imm),
            FnIndex::TRUE => Some(1.0),
            FnIndex::FALSE => Some(0.0),
            FnIndex::ADD => Some(a(0) + a(1)),
            FnIndex::SUB => Some(a(0) - a(1)),
            FnIndex::MUL => Some(a(0) * a(1)),
            FnIndex::DIV => Some(if a(1) == 0.0 { 0.0 } else { a(0) / a(1) }),
            FnIndex::MOD => Some(if a(1) == 0.0 { 0.0 } else { a(0) % a(1) }),
            FnIndex::LT => Some(f32::from(a(0) < a(1))),
            FnIndex::GT => Some(f32::from(a(0) > a(1))),
            FnIndex::EQ => Some(f32::from((a(0) - a(1)).abs() < f32::EPSILON)),
            FnIndex::AND => Some(f32::from(a(0) != 0.0 && a(1) != 0.0)),
            FnIndex::OR => Some(f32::from(a(0) != 0.0 || a(1) != 0.0)),
            FnIndex::NOT => Some(f32::from(a(0) == 0.0)),
            FnIndex::ABS => Some(a(0).abs()),
            FnIndex::ROUND => Some(a(0).round()),
            // The immediate is the VARIABLE codebook byte — which variable.
            FnIndex::VAR_GET => Some(s.var(imm as u8)),
            // The immediate is the argument's POSITION in the innermost
            // custom-block frame; outside any call it reads 0, as an unset
            // Scratch value does.
            FnIndex::PROC_ARG => Some(frame_arg),
            FnIndex::VAR_SET => {
                *s.var_mut(imm as u8) = a(0);
                return Ok(None);
            }
            FnIndex::VAR_CHANGE => {
                *s.var_mut(imm as u8) += a(0);
                return Ok(None);
            }
            _ => None,
        };
        if let Some(v) = core {
            return Ok(Some(v));
        }

        // Then the device half, through the plan — the harvested NAME table
        // resolved to integer tags once, so the palette, the toolbox and the
        // interpreter still read one source and the hot path reads none.
        match plan().op[usize::from(f.0)] {
            Op::Unimplemented => Err(RunError::Unimplemented(f.0)),
            // A menu shadow block is a value: it yields its own codebook
            // index, which the consuming block pops as an operand.
            Op::Menu => Ok(Some(imm)),
            Op::Hat => Ok(None),
            Op::KeyPressed => {
                // The operand is the KEY_OPTION index the menu reporter
                // pushed; `any` is the harvested option 6.
                let want = a(0) as u8;
                let any = blockly_abi::menus::menu_by_id(1)
                    .and_then(|m| blockly_abi::menus::encode(m, "any"))
                    .unwrap_or(0);
                let held = s.key.is_some_and(|k| k == want || want == any);
                Ok(Some(f32::from(held)))
            }
            // Front/back layering has no visual model here; a real op with
            // no effect on this stage, not an unimplemented one.
            Op::LayerNoop => Ok(None),
            Op::MoveSteps => {
                let r = s.sprites[me].direction.to_radians();
                s.sprites[me].x += a(0) * r.sin();
                s.sprites[me].y += a(0) * r.cos();
                Ok(None)
            }
            Op::TurnRight => {
                s.sprites[me].direction += a(0);
                Ok(None)
            }
            Op::TurnLeft => {
                s.sprites[me].direction -= a(0);
                Ok(None)
            }
            Op::PointDir => {
                s.sprites[me].direction = a(0);
                Ok(None)
            }
            Op::GotoXy => {
                s.sprites[me].x = a(0);
                s.sprites[me].y = a(1);
                Ok(None)
            }
            Op::SetX => {
                s.sprites[me].x = a(0);
                Ok(None)
            }
            Op::SetY => {
                s.sprites[me].y = a(0);
                Ok(None)
            }
            Op::ChangeX => {
                s.sprites[me].x += a(0);
                Ok(None)
            }
            Op::ChangeY => {
                s.sprites[me].y += a(0);
                Ok(None)
            }
            Op::Bounce => {
                // Clamp strictly INSIDE the edge. Clamping to exactly the
                // boundary leaves `abs() >= half` true on the next call, so
                // the sprite flips direction every frame and sticks to the
                // wall — measured: the ball ended pinned at y = half_h.
                const INSET: f32 = 1.0;
                if s.sprites[me].x.abs() >= s.half_w {
                    s.sprites[me].direction = -s.sprites[me].direction;
                    let lim = s.half_w - INSET;
                    s.sprites[me].x = s.sprites[me].x.clamp(-lim, lim);
                }
                if s.sprites[me].y.abs() >= s.half_h {
                    s.sprites[me].direction = 180.0 - s.sprites[me].direction;
                    let lim = s.half_h - INSET;
                    s.sprites[me].y = s.sprites[me].y.clamp(-lim, lim);
                }
                Ok(None)
            }
            Op::XPos => Ok(Some(s.sprites[me].x)),
            Op::YPos => Ok(Some(s.sprites[me].y)),
            Op::Dir => Ok(Some(s.sprites[me].direction)),
            Op::Show => {
                s.sprites[me].visible = true;
                Ok(None)
            }
            Op::Hide => {
                s.sprites[me].visible = false;
                Ok(None)
            }
            Op::ChangeSize => {
                s.sprites[me].size += a(0);
                Ok(None)
            }
            Op::SetSize => {
                s.sprites[me].size = a(0);
                Ok(None)
            }
            Op::Size => Ok(Some(s.sprites[me].size)),
            // A costume switch has no visual model here; it is a real op with
            // no effect on this stage, which is different from unimplemented.
            Op::CostumeNoop => Ok(None),
            Op::MouseX => Ok(Some(s.mouse_x)),
            Op::MouseY => Ok(Some(s.mouse_y)),
            Op::Timer => Ok(Some(s.timer)),
            Op::ResetTimer => {
                s.timer = 0.0;
                Ok(None)
            }
            Op::Touching => Ok(Some(f32::from(s.touching))),
            Op::MouseDown => Ok(Some(0.0)),
        }
    }
}

fn pop(stack: &mut Vec<f32>, f: FnIndex) -> Result<f32, RunError> {
    stack.pop().ok_or(RunError::StackUnderflow(f.0))
}

/// Several scripts sharing one stage — the thing that makes a scene.
///
/// # Why a scheduler at all
///
/// Running each script to completion on its own stage is what the demo did
/// first, and it produced a frozen dot: the ball script never saw the paddle,
/// the paddle script's result was discarded, and only one sprite was ever
/// drawn. Pong is two concurrent scripts over one scene, so the runner has to
/// be too.
///
/// # How concurrency is approximated, honestly
///
/// The interpreter is recursive, not resumable — it cannot be paused mid-body
/// and continued. So instead of pretending, each round gives every script a
/// SLICE of calls against the shared stage, restarting that script's own
/// entry each time. A `forever` therefore advances a little per round, which
/// is exactly the visible behaviour a cooperative scheduler produces, and the
/// stage carries the accumulated state across rounds.
///
/// It is an approximation, and it is written down as one: a script whose
/// prologue does real work before its loop would re-run that prologue each
/// round. Pong's scripts are `hat → forever { … }`, where the prologue is the
/// hat, so the approximation is exact for them.
pub struct Scene<'a> {
    /// The shared stage every script acts on.
    pub stage: Stage,
    scripts: Vec<&'a [FunctionBody]>,
    /// Custom blocks: definition scripts are NOT scheduled — a definition
    /// runs when called — they are the scene's procedure table.
    procs: Vec<Procedure<'a>>,
    /// The ONE constant pool every scheduled script and procedure reads —
    /// the unit is the sprite: cast all of a sprite's scripts against one
    /// `LoweringContext` and hand its pool here.
    pool: Option<&'a ogar_loco::ConstantPool>,
    trace: Vec<Stage>,
    /// Simulated pointer travel, in stage units per round.
    ///
    /// The mouse is an INPUT. Held constant, a paddle that tracks it has
    /// nothing to track and renders as two positions — which reads as frozen,
    /// and was: the demo showed a motionless paddle because the input never
    /// changed, not because the program was wrong. A demo therefore has to
    /// supply an input, and this makes that explicit rather than pretending
    /// the stage moves it.
    mouse_sweep: f32,
    /// Simulated keyboard: alternate `up arrow` / `down arrow` every this
    /// many rounds. Same reasoning as the mouse — a key-driven paddle under a
    /// constant (or absent) key is a frozen paddle. Zero = leave `stage.key`
    /// as the caller set it.
    key_period: u32,
}

impl<'a> Scene<'a> {
    /// Build a scene from one program per sprite.
    #[must_use]
    pub fn new(stage: Stage, scripts: Vec<&'a [FunctionBody]>) -> Self {
        let (defs, scheduled): (Vec<_>, Vec<_>) = scripts
            .into_iter()
            .partition(|s| Procedure::of_script(s).is_some());
        let procs = defs.into_iter().filter_map(Procedure::of_script).collect();
        Self {
            stage,
            scripts: scheduled,
            procs,
            pool: None,
            trace: Vec::new(),
            mouse_sweep: 0.0,
            key_period: 0,
        }
    }

    /// Read wide literals from this pool in every script the scene runs.
    #[must_use]
    pub fn with_pool(mut self, pool: &'a ogar_loco::ConstantPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Alternate the held key between `up arrow` and `down arrow` every
    /// `period` rounds while the scene runs. Zero (the default) leaves the
    /// key wherever the caller put it.
    #[must_use]
    pub fn with_key_sweep(mut self, period: u32) -> Self {
        self.key_period = period;
        self
    }

    /// Sweep the simulated pointer up and down while the scene runs.
    ///
    /// `amplitude` is how far it travels from centre. Zero (the default)
    /// leaves the mouse wherever the caller put it.
    #[must_use]
    pub fn with_mouse_sweep(mut self, amplitude: f32) -> Self {
        self.mouse_sweep = amplitude;
        self
    }

    /// Run `rounds` rounds, giving each script `slice` calls per round, and
    /// record one trace frame per round.
    ///
    /// # Errors
    ///
    /// The first refusal any script raises, so a broken program is named
    /// rather than silently producing a stage that merely looks still.
    pub fn run(&mut self, rounds: u32, slice: u32) -> Result<(), RunError> {
        self.trace.push(self.stage.clone());
        for round in 0..rounds {
            // Move the simulated pointer BEFORE the scripts read it, so a
            // paddle that tracks the mouse sees a fresh value each round.
            if self.mouse_sweep != 0.0 {
                let phase = f32::from(u16::try_from(round % 1000).unwrap_or(0));
                self.stage.mouse_y = (phase * 0.07).sin() * self.mouse_sweep;
            }
            // `checked_div` is the "sweep off" test: a zero period divides
            // to `None` and the key is left where the caller put it.
            if let Some(phase) = round.checked_div(self.key_period) {
                let keys = blockly_abi::menus::menu_by_id(1).expect("KEY_OPTION is menu 1");
                let code = if phase.is_multiple_of(2) {
                    "up arrow"
                } else {
                    "down arrow"
                };
                self.stage.key = blockly_abi::menus::encode(keys, code);
            }
            let procs = &self.procs;
            for (i, script) in self.scripts.iter().enumerate() {
                let stage = core::mem::take(&mut self.stage);
                let mut m = Machine::resuming(script, slice, stage, i).with_procs(procs);
                if let Some(pool) = self.pool {
                    m = m.with_pool(pool);
                }
                let outcome = m.run();
                self.stage = m.stage;
                outcome?;
            }
            self.stage.timer += 1.0 / 30.0;
            self.trace.push(self.stage.clone());
        }
        Ok(())
    }

    /// The custom blocks this scene can call.
    #[must_use]
    pub fn procedures(&self) -> &[Procedure<'a>] {
        &self.procs
    }

    /// One stage snapshot per round, oldest first.
    #[must_use]
    pub fn trace(&self) -> &[Stage] {
        &self.trace
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blockly_abi::{BlockRecord, FieldValue, lower_program};
    use ogar_loco::LaneShape;

    fn rec(
        ty: &str,
        fields: Vec<(String, FieldValue)>,
        inputs: Vec<(String, BlockRecord)>,
        statements: Vec<(String, BlockRecord)>,
        next: Option<BlockRecord>,
    ) -> BlockRecord {
        BlockRecord {
            ty: ty.into(),
            id: ty.into(),
            fields,
            inputs,
            statements,
            next: next.map(Box::new),
            extra_state: None,
            disabled: false,
        }
    }
    fn num(v: u8) -> BlockRecord {
        rec(
            "math_number",
            vec![("NUM".into(), FieldValue::Byte(v))],
            vec![],
            vec![],
            None,
        )
    }

    /// The interpreter moves the sprite by running the STORED BYTES.
    ///
    /// The program is lowered first, so what executes is a `FunctionBody` —
    /// the same thing `from_le_bytes` yields. Nothing here reads a block.
    #[test]
    fn a_stored_program_moves_the_sprite() {
        // point in direction 90 (right), move 10 steps.
        let prog = lower_program(
            LaneShape::Pairs,
            &rec(
                "motion_pointindirection",
                vec![],
                vec![("DIRECTION".into(), num(90))],
                vec![],
                Some(rec(
                    "motion_movesteps",
                    vec![],
                    vec![("STEPS".into(), num(10))],
                    vec![],
                    None,
                )),
            ),
        )
        .expect("casts");

        let mut m = Machine::new(&prog.functions, 1000);
        m.run().expect("runs");
        assert!(
            (m.stage.sprites[0].x - 10.0).abs() < 0.01,
            "x = {}",
            m.stage.sprites[0].x
        );
        assert!(
            m.stage.sprites[0].y.abs() < 0.01,
            "y = {}",
            m.stage.sprites[0].y
        );

        // Can-stay-silent: a program that moves nothing leaves the stage at
        // its default, so the assertion above measures the RUN, not the
        // constructor.
        let idle = lower_program(
            LaneShape::Pairs,
            &rec("looks_show", vec![], vec![], vec![], None),
        )
        .expect("casts");
        let mut m2 = Machine::new(&idle.functions, 1000);
        m2.run().expect("runs");
        assert_eq!(m2.stage.sprites[0].x, 0.0);
    }

    /// A keyboard paddle: the key arrives as a CODEBOOK INDEX through a menu
    /// reporter, and the run reads it — can-fire, can-stay-silent, and the
    /// `any` option, on the same program.
    #[test]
    fn a_key_option_menu_drives_the_sprite_through_its_codebook_index() {
        use blockly_abi::menus;
        let code = |ty: &str, field: &str, c: &str| {
            rec(
                ty,
                vec![(field.into(), FieldValue::Code(c.into()))],
                vec![],
                vec![],
                None,
            )
        };
        // if <key [up arrow] pressed?> then change y by 10
        let prog = lower_program(
            LaneShape::Pairs,
            &rec(
                "control_if",
                vec![],
                vec![(
                    "CONDITION".into(),
                    rec(
                        "sensing_keypressed",
                        vec![],
                        vec![(
                            "KEY_OPTION".into(),
                            code("sensing_keyoptions", "KEY_OPTION", "up arrow"),
                        )],
                        vec![],
                        None,
                    ),
                )],
                vec![(
                    "SUBSTACK".into(),
                    rec(
                        "motion_changeyby",
                        vec![],
                        vec![("DY".into(), num(10))],
                        vec![],
                        None,
                    ),
                )],
                None,
            ),
        )
        .expect("a menu shadow block casts");
        let keys = menus::menu_by_id(1).unwrap();
        let up = menus::encode(keys, "up arrow").unwrap();
        let down = menus::encode(keys, "down arrow").unwrap();

        let run_with = |key: Option<u8>| {
            let mut m = Machine::new(&prog.functions, 1000);
            m.stage.key = key;
            m.run().expect("runs");
            m.stage.sprites[0].y
        };
        assert_eq!(run_with(Some(up)), 10.0, "held key matches: moves");
        assert_eq!(run_with(None), 0.0, "no key: stays");
        assert_eq!(run_with(Some(down)), 0.0, "a different key: stays");

        // `any` matches whatever is held — and only when something is.
        let any_prog = lower_program(
            LaneShape::Pairs,
            &rec(
                "control_if",
                vec![],
                vec![(
                    "CONDITION".into(),
                    rec(
                        "sensing_keypressed",
                        vec![],
                        vec![(
                            "KEY_OPTION".into(),
                            code("sensing_keyoptions", "KEY_OPTION", "any"),
                        )],
                        vec![],
                        None,
                    ),
                )],
                vec![(
                    "SUBSTACK".into(),
                    rec(
                        "motion_changeyby",
                        vec![],
                        vec![("DY".into(), num(10))],
                        vec![],
                        None,
                    ),
                )],
                None,
            ),
        )
        .unwrap();
        let mut m = Machine::new(&any_prog.functions, 1000);
        m.stage.key = Some(down);
        m.run().unwrap();
        assert_eq!(m.stage.sprites[0].y, 10.0);
        let mut m = Machine::new(&any_prog.functions, 1000);
        m.run().unwrap();
        assert_eq!(m.stage.sprites[0].y, 0.0);
    }

    /// Two variables are two slots: the VARIABLE codebook byte a `data_*`
    /// call carries selects which one, so `score` and `lives` do not alias —
    /// and the zero-fallback slot the templates use is a third, untouched.
    #[test]
    fn variables_are_addressed_by_their_codebook_byte_not_shared() {
        use blockly_abi::menus;
        use ogar_loco::basin::BasinCodebooks;
        let vars = menus::SCRATCH_MENUS
            .iter()
            .find(|m| m.name == "VARIABLE")
            .unwrap();
        let mut b = menus::builder(
            vars,
            ogar_loco::pool::placeholder::CONST_UTF8_INLINE,
            menus::PLACEHOLDER_DIGEST_CLASSID,
        )
        .unwrap();
        let score = b
            .intern(ogar_loco::pool::placeholder::CONST_UTF8_INLINE, b"score")
            .unwrap();
        let lives = b
            .intern(ogar_loco::pool::placeholder::CONST_UTF8_INLINE, b"lives")
            .unwrap();
        let mut basin = BasinCodebooks::new();
        basin.plug(b.seal()).unwrap();
        assert_ne!(score, lives);

        // set score to 7; change lives by 3
        let set = |var: &str, ty: &str, v: u8, next: Option<BlockRecord>| {
            rec(
                ty,
                vec![("VARIABLE".into(), FieldValue::Code(var.into()))],
                vec![("VALUE".into(), num(v))],
                vec![],
                next,
            )
        };
        let prog = blockly_abi::lower_program_in(
            LaneShape::Pairs,
            &set(
                "score",
                "data_setvariableto",
                7,
                Some(set("lives", "data_changevariableby", 3, None)),
            ),
            &basin,
        )
        .expect("named variables cast against the project basin");
        let mut m = Machine::new(&prog.functions, 1000);
        m.run().unwrap();
        assert_eq!(m.stage.var(score), 7.0);
        assert_eq!(m.stage.var(lives), 3.0);
        assert_eq!(m.stage.var(0), 0.0, "the zero-fallback slot is untouched");
        // Silence half: the static basin knows no variable names, so the
        // same program is refused rather than silently written to slot 0.
        assert!(
            blockly_abi::lower_program(
                LaneShape::Pairs,
                &set("score", "data_setvariableto", 7, None)
            )
            .is_err()
        );
    }

    /// A custom block runs from its DEFINITION script when another script
    /// calls it, with its argument read by position — and a definition is
    /// never scheduled on its own.
    #[test]
    fn a_custom_block_runs_when_called_with_its_arguments_and_never_on_its_own() {
        use blockly_abi::menus;
        use ogar_loco::basin::BasinCodebooks;
        let procs = menus::SCRATCH_MENUS
            .iter()
            .find(|m| m.name == "PROCEDURE")
            .unwrap();
        let mut b = menus::builder(
            procs,
            ogar_loco::pool::placeholder::CONST_UTF8_INLINE,
            menus::PLACEHOLDER_DIGEST_CLASSID,
        )
        .unwrap();
        let walk = b
            .intern(ogar_loco::pool::placeholder::CONST_UTF8_INLINE, b"walk %n")
            .unwrap();
        let mut basin = BasinCodebooks::new();
        basin.plug(b.seal()).unwrap();
        let code = |c: &str| FieldValue::Code(c.into());

        // define walk %n: change x by (n)
        let arg = rec(
            "argument_reporter_string_number",
            vec![("VALUE".into(), FieldValue::Byte(0))],
            vec![],
            vec![],
            None,
        );
        let body = rec(
            "motion_changexby",
            vec![],
            vec![("DX".into(), arg)],
            vec![],
            None,
        );
        let def = rec(
            "procedures_definition",
            vec![("PROCCODE".into(), code("walk %n"))],
            vec![],
            vec![("SUBSTACK".into(), body)],
            None,
        );
        // when flag clicked: walk(7); walk(7)
        let call = |next: Option<BlockRecord>| {
            rec(
                "procedures_call",
                vec![
                    ("PROCCODE".into(), code("walk %n")),
                    ("ARGC".into(), FieldValue::Byte(1)),
                ],
                vec![("input0".into(), num(7))],
                vec![],
                next,
            )
        };
        let hat = rec(
            "event_whenflagclicked",
            vec![],
            vec![],
            vec![],
            Some(call(Some(call(None)))),
        );

        let d = blockly_abi::lower_program_in(LaneShape::Triples, &def, &basin).expect("def casts");
        let h =
            blockly_abi::lower_program_in(LaneShape::Triples, &hat, &basin).expect("call casts");
        let proc_ = Procedure::of_script(&d.functions).expect("a definition");
        assert_eq!((proc_.index, proc_.body), (walk, 1));
        assert!(Procedure::of_script(&h.functions).is_none());

        // One round is enough: two calls, x = 14.
        let mut scene = Scene::new(Stage::default(), vec![&d.functions, &h.functions]);
        assert_eq!(scene.procedures().len(), 1);
        scene.run(1, 100).expect("runs");
        assert_eq!(
            scene.stage.sprites[0].x, 14.0,
            "walk(7) twice through the frame"
        );

        // Silence half: the definition alone does nothing — it is a table
        // entry, not a script that runs.
        let mut alone = Scene::new(Stage::default(), vec![&d.functions]);
        alone.run(3, 100).expect("runs");
        assert_eq!(alone.stage.sprites[0].x, 0.0);

        // A call whose procedure is not in the scene is refused, not skipped.
        let mut orphan = Scene::new(Stage::default(), vec![&h.functions]);
        assert!(matches!(orphan.run(1, 100), Err(RunError::UnknownProcedure(i)) if i == walk));

        // An argument read outside any call is 0, not a stale frame.
        let loose = rec(
            "motion_changexby",
            vec![],
            vec![(
                "DX".into(),
                rec(
                    "argument_reporter_string_number",
                    vec![("VALUE".into(), FieldValue::Byte(0))],
                    vec![],
                    vec![],
                    None,
                ),
            )],
            vec![],
            None,
        );
        let l = blockly_abi::lower_program(LaneShape::Pairs, &loose).unwrap();
        let mut m = Machine::new(&l.functions, 100);
        m.run().unwrap();
        assert_eq!(m.stage.sprites[0].x, 0.0);
    }

    /// `forever` terminates on the budget, and the budget bounds the RUN only.
    #[test]
    fn forever_is_bounded_by_the_budget_not_by_a_special_case() {
        let prog = lower_program(
            LaneShape::Pairs,
            &rec(
                "control_forever",
                vec![],
                vec![],
                vec![(
                    "SUBSTACK".into(),
                    rec(
                        "motion_changexby",
                        vec![],
                        vec![("DX".into(), num(1))],
                        vec![],
                        None,
                    ),
                )],
                None,
            ),
        )
        .expect("casts");

        let mut small = Machine::new(&prog.functions, 40);
        small.run().expect("runs");
        let mut large = Machine::new(&prog.functions, 200);
        large.run().expect("runs");

        assert!(small.stage.sprites[0].x > 0.0, "the loop must actually run");
        assert!(
            large.stage.sprites[0].x > small.stage.sprites[0].x,
            "a bigger budget must run longer: {} vs {}",
            large.stage.sprites[0].x,
            small.stage.sprites[0].x
        );
    }

    /// A conditional reads the stage and branches on it — both ways.
    #[test]
    fn a_condition_reads_the_stage_and_branches_both_ways() {
        let prog = lower_program(
            LaneShape::Pairs,
            &rec(
                "control_if",
                vec![],
                vec![(
                    "CONDITION".into(),
                    rec(
                        "sensing_touchingobject",
                        vec![],
                        vec![("TOUCHINGOBJECTMENU".into(), num(1))],
                        vec![],
                        None,
                    ),
                )],
                vec![(
                    "SUBSTACK".into(),
                    rec(
                        "data_changevariableby",
                        vec![],
                        vec![("VALUE".into(), num(5))],
                        vec![],
                        None,
                    ),
                )],
                None,
            ),
        )
        .expect("casts");

        let mut hit = Machine::new(&prog.functions, 100);
        hit.stage.touching = true;
        hit.run().expect("runs");
        assert_eq!(hit.stage.var(0), 5.0, "the branch must fire when touching");

        let mut miss = Machine::new(&prog.functions, 100);
        miss.stage.touching = false;
        miss.run().expect("runs");
        assert_eq!(
            miss.stage.var(0),
            0.0,
            "the branch must NOT fire when not touching"
        );
    }

    /// An unimplemented device op is NAMED, never silently skipped.
    #[test]
    fn an_unimplemented_operation_refuses_loudly() {
        let prog = lower_program(
            LaneShape::Pairs,
            &rec("sound_stopallsounds", vec![], vec![], vec![], None),
        )
        .expect("casts");
        let mut m = Machine::new(&prog.functions, 100);
        let err = m.run().unwrap_err();
        assert!(matches!(err, RunError::Unimplemented(_)), "{err:?}");

        // Can-fire: an implemented op does NOT refuse, so the refusal above
        // discriminates instead of rejecting every device call.
        let ok = lower_program(
            LaneShape::Pairs,
            &rec("looks_hide", vec![], vec![], vec![], None),
        )
        .expect("casts");
        assert!(Machine::new(&ok.functions, 100).run().is_ok());
    }

    /// A wide literal is READ FROM THE POOL at run time: `set x to 1000000`
    /// lands the sprite at a million, not at the pool index. The same body
    /// run without its pool is refused, not run with the index as the value.
    #[test]
    fn a_pooled_literal_is_read_from_the_pool_and_a_missing_pool_is_refused() {
        use blockly_abi::{LoweringContext, lower_program_with_pool};
        let script = rec(
            "motion_setx",
            vec![],
            vec![(
                "X".into(),
                rec(
                    "math_number",
                    vec![("NUM".into(), FieldValue::Wide("1000000".into()))],
                    vec![],
                    vec![],
                    None,
                ),
            )],
            vec![],
            None,
        );
        let basin = blockly_abi::menus::static_basin();
        let mut ctx = LoweringContext::placeholder();
        let prog = lower_program_with_pool(LaneShape::Pairs, &script, basin, &mut ctx).unwrap();
        let idx = blockly_abi::raise_calls(prog.entry())[0].values[0];
        assert_ne!(idx, 0);

        let mut m = Machine::new(&prog.functions, 100).with_pool(&ctx.pool);
        m.run().unwrap();
        assert_eq!(m.stage.me().x, 1_000_000.0);
        // Anti-vacuity: a run that read the INDEX would have landed here.
        assert_ne!(m.stage.me().x, f32::from(idx));

        let mut bare = Machine::new(&prog.functions, 100);
        assert_eq!(bare.run(), Err(RunError::MissingConstant(idx)));
    }
}
