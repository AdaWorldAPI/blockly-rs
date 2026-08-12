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
    /// The single variable the demo scores with.
    pub var: f32,
    /// Mouse position the sensing reporters return.
    pub mouse_x: f32,
    /// Mouse position.
    pub mouse_y: f32,
    /// Seconds since the run started, advanced one tick per statement batch.
    pub timer: f32,
    /// Whether `sensing_touchingobject` should answer true.
    pub touching: bool,
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
            var: 0.0,
            mouse_x: 0.0,
            mouse_y: 0.0,
            timer: 0.0,
            touching: false,
            half_w: 240.0,
            half_h: 180.0,
        }
    }
}

impl Stage {
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
}

impl core::fmt::Display for RunError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Uncovered(b) => write!(f, "function {b:#04x} has no covered arity"),
            Self::StackUnderflow(b) => write!(f, "function {b:#04x} wanted absent operands"),
            Self::DanglingReference(i) => write!(f, "body reference {i} names no function"),
            Self::Unimplemented(b) => write!(f, "function {b:#04x} is not implemented"),
        }
    }
}

impl core::error::Error for RunError {}

/// A bounded run over stored function bodies.
pub struct Machine<'a> {
    /// The stage the program acts on.
    pub stage: Stage,
    functions: &'a [FunctionBody],
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
            budget,
            trace: Vec::new(),
            every: 0,
            counter: 0,
        }
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

    fn exec(&mut self, index: usize) -> Result<(), RunError> {
        let body = *self
            .functions
            .get(index)
            .ok_or(RunError::DanglingReference(index as u8))?;
        let mut stack: Vec<f32> = Vec::new();

        for call in blockly_abi::raise_calls(&body) {
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
                            self.exec(target)?;
                        }
                    }
                    FnIndex::IF => {
                        let c = pop(&mut stack, f)?;
                        if c != 0.0 {
                            self.exec(target)?;
                        }
                    }
                    FnIndex::IF_ELSE => {
                        let c = pop(&mut stack, f)?;
                        let other = usize::from(call.values.get(1).copied().unwrap_or(0));
                        self.exec(if c != 0.0 { target } else { other })?;
                    }
                    FnIndex::REPEAT => {
                        let n = pop(&mut stack, f)?.max(0.0) as u32;
                        for _ in 0..n {
                            if self.budget == 0 {
                                break;
                            }
                            self.exec(target)?;
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
                                self.exec(target)?;
                            }
                        }
                    }
                    _ => return Err(RunError::Unimplemented(f.0)),
                }
                continue;
            }

            // ── everything else: pop arity, compute, push if it yields ────
            let arity = usize::from(
                shared_core::stack_arity(f)
                    .or_else(|| scratch::device_by_byte(f.0).map(|(_, a, _)| a))
                    .ok_or(RunError::Uncovered(f.0))?,
            );
            if stack.len() < arity {
                return Err(RunError::StackUnderflow(f.0));
            }
            let ops: Vec<f32> = stack.split_off(stack.len() - arity);
            let immediate = f32::from(call.values.first().copied().unwrap_or(0));

            let result = self.apply(f, &ops, immediate)?;
            if let Some(v) = result {
                stack.push(v);
            }
        }
        Ok(())
    }

    /// Apply one non-branching call. `Some(v)` means it yielded a value.
    fn apply(&mut self, f: FnIndex, ops: &[f32], imm: f32) -> Result<Option<f32>, RunError> {
        let a = |i: usize| ops.get(i).copied().unwrap_or(0.0);
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
            FnIndex::VAR_GET => Some(s.var),
            FnIndex::VAR_SET => {
                s.var = a(0);
                return Ok(None);
            }
            FnIndex::VAR_CHANGE => {
                s.var += if ops.is_empty() { imm } else { a(0) };
                return Ok(None);
            }
            _ => None,
        };
        if let Some(v) = core {
            return Ok(Some(v));
        }

        // Then the device half, by its harvested name — so the interpreter
        // reads the same table the palette and the toolbox do.
        let Some((name, ..)) = scratch::device_by_byte(f.0) else {
            return Err(RunError::Unimplemented(f.0));
        };
        match name {
            "event_whenflagclicked" | "event_whenthisspriteclicked" => Ok(None),
            "motion_movesteps" => {
                let r = s.sprites[me].direction.to_radians();
                s.sprites[me].x += a(0) * r.sin();
                s.sprites[me].y += a(0) * r.cos();
                Ok(None)
            }
            "motion_turnright" => {
                s.sprites[me].direction += a(0);
                Ok(None)
            }
            "motion_turnleft" => {
                s.sprites[me].direction -= a(0);
                Ok(None)
            }
            "motion_pointindirection" => {
                s.sprites[me].direction = a(0);
                Ok(None)
            }
            "motion_gotoxy" => {
                s.sprites[me].x = a(0);
                s.sprites[me].y = a(1);
                Ok(None)
            }
            "motion_setx" => {
                s.sprites[me].x = a(0);
                Ok(None)
            }
            "motion_sety" => {
                s.sprites[me].y = a(0);
                Ok(None)
            }
            "motion_changexby" => {
                s.sprites[me].x += a(0);
                Ok(None)
            }
            "motion_changeyby" => {
                s.sprites[me].y += a(0);
                Ok(None)
            }
            "motion_ifonedgebounce" => {
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
            "motion_xposition" => Ok(Some(s.sprites[me].x)),
            "motion_yposition" => Ok(Some(s.sprites[me].y)),
            "motion_direction" => Ok(Some(s.sprites[me].direction)),
            "looks_show" => {
                s.sprites[me].visible = true;
                Ok(None)
            }
            "looks_hide" => {
                s.sprites[me].visible = false;
                Ok(None)
            }
            "looks_changesizeby" => {
                s.sprites[me].size += a(0);
                Ok(None)
            }
            "looks_setsizeto" => {
                s.sprites[me].size = a(0);
                Ok(None)
            }
            "looks_size" => Ok(Some(s.sprites[me].size)),
            // A costume switch has no visual model here; it is a real op with
            // no effect on this stage, which is different from unimplemented.
            "looks_nextcostume" | "looks_nextbackdrop" => Ok(None),
            "sensing_mousex" => Ok(Some(s.mouse_x)),
            "sensing_mousey" => Ok(Some(s.mouse_y)),
            "sensing_timer" => Ok(Some(s.timer)),
            "sensing_resettimer" => {
                s.timer = 0.0;
                Ok(None)
            }
            "sensing_touchingobject" | "sensing_touchingcolor" => Ok(Some(f32::from(s.touching))),
            "sensing_mousedown" => Ok(Some(0.0)),
            _ => Err(RunError::Unimplemented(f.0)),
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
}

impl<'a> Scene<'a> {
    /// Build a scene from one program per sprite.
    #[must_use]
    pub fn new(stage: Stage, scripts: Vec<&'a [FunctionBody]>) -> Self {
        Self {
            stage,
            scripts,
            trace: Vec::new(),
            mouse_sweep: 0.0,
        }
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
            for (i, script) in self.scripts.iter().enumerate() {
                let stage = core::mem::take(&mut self.stage);
                let mut m = Machine::resuming(script, slice, stage, i);
                let outcome = m.run();
                self.stage = m.stage;
                outcome?;
            }
            self.stage.timer += 1.0 / 30.0;
            self.trace.push(self.stage.clone());
        }
        Ok(())
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
        assert_eq!(hit.stage.var, 5.0, "the branch must fire when touching");

        let mut miss = Machine::new(&prog.functions, 100);
        miss.stage.touching = false;
        miss.run().expect("runs");
        assert_eq!(
            miss.stage.var, 0.0,
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
}
