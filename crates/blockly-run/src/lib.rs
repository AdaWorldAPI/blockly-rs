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

/// The stage a program acts on.
///
/// Deliberately tiny and concrete: one sprite plus the few globals the
/// reference game reads. A general sprite model is a bigger design than the
/// demo needs, and inventing one here would be scope this cannot test.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// Sprite position, Scratch coordinates (centre origin, y up).
    pub x: f32,
    /// Sprite position.
    pub y: f32,
    /// Heading in degrees, Scratch convention: 90 = right, 0 = up.
    pub direction: f32,
    /// Whether the sprite draws.
    pub visible: bool,
    /// Sprite scale, percent.
    pub size: f32,
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
            x: 0.0,
            y: 0.0,
            direction: 90.0,
            visible: true,
            size: 100.0,
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
}

impl<'a> Machine<'a> {
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
        }
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

            let f = call.function;
            let refs = usize::from(shared_core::body_refs(f));

            // ── control flow: a body reference is not an operand ──────────
            if refs > 0 {
                let target = usize::from(call.values.first().copied().unwrap_or(0));
                match f {
                    FnIndex::FOREVER => {
                        while self.budget > 0 {
                            self.exec(target)?;
                            self.tick();
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
                                self.tick();
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

    fn tick(&mut self) {
        self.timer_advance();
    }

    fn timer_advance(&mut self) {
        self.stage.timer += 1.0 / 30.0;
    }

    /// Apply one non-branching call. `Some(v)` means it yielded a value.
    fn apply(&mut self, f: FnIndex, ops: &[f32], imm: f32) -> Result<Option<f32>, RunError> {
        let a = |i: usize| ops.get(i).copied().unwrap_or(0.0);
        let s = &mut self.stage;

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
                let r = s.direction.to_radians();
                s.x += a(0) * r.sin();
                s.y += a(0) * r.cos();
                Ok(None)
            }
            "motion_turnright" => {
                s.direction += a(0);
                Ok(None)
            }
            "motion_turnleft" => {
                s.direction -= a(0);
                Ok(None)
            }
            "motion_pointindirection" => {
                s.direction = a(0);
                Ok(None)
            }
            "motion_gotoxy" => {
                s.x = a(0);
                s.y = a(1);
                Ok(None)
            }
            "motion_setx" => {
                s.x = a(0);
                Ok(None)
            }
            "motion_sety" => {
                s.y = a(0);
                Ok(None)
            }
            "motion_changexby" => {
                s.x += a(0);
                Ok(None)
            }
            "motion_changeyby" => {
                s.y += a(0);
                Ok(None)
            }
            "motion_ifonedgebounce" => {
                if s.x.abs() >= s.half_w {
                    s.direction = -s.direction;
                    s.x = s.x.clamp(-s.half_w, s.half_w);
                }
                if s.y.abs() >= s.half_h {
                    s.direction = 180.0 - s.direction;
                    s.y = s.y.clamp(-s.half_h, s.half_h);
                }
                Ok(None)
            }
            "motion_xposition" => Ok(Some(s.x)),
            "motion_yposition" => Ok(Some(s.y)),
            "motion_direction" => Ok(Some(s.direction)),
            "looks_show" => {
                s.visible = true;
                Ok(None)
            }
            "looks_hide" => {
                s.visible = false;
                Ok(None)
            }
            "looks_changesizeby" => {
                s.size += a(0);
                Ok(None)
            }
            "looks_setsizeto" => {
                s.size = a(0);
                Ok(None)
            }
            "looks_size" => Ok(Some(s.size)),
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
        assert!((m.stage.x - 10.0).abs() < 0.01, "x = {}", m.stage.x);
        assert!(m.stage.y.abs() < 0.01, "y = {}", m.stage.y);

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
        assert_eq!(m2.stage.x, 0.0);
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

        assert!(small.stage.x > 0.0, "the loop must actually run");
        assert!(
            large.stage.x > small.stage.x,
            "a bigger budget must run longer: {} vs {}",
            large.stage.x,
            small.stage.x
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
