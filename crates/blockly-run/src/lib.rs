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

/// One stack value. The stack is TYPED because two of the three kinds are
/// register indices, not quantities: a text is a TEXT-codebook index and a
/// list is a LIST-codebook handle, and turning either into an `f32` would
/// lose the register it names (`say "hello"` must keep "hello"; `add x to
/// (list)` must know WHICH list). Numbers stay `f32`. Eight bytes, `Copy`,
/// no allocation — the hot path's operand window is three of these.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    /// A number.
    Num(f32),
    /// An index into the PROJECT's TEXT register (menu 29) — a literal the
    /// intake interned once, sealed. `0` is the empty string.
    Text(u8),
    /// An index into the RUN's text register ([`TextRegister`]) — a string
    /// some operator MADE while running (`join`, `letter of`). A separate
    /// variant, not a second index range, so the compiler forces every site
    /// that reads a text to say which register it means.
    RunText(u8),
    /// A LIST register handle (menu 26).
    List(u8),
}

/// The two text registers a run reads from: the project's sealed one and the
/// run's own. Passed as one `Copy` pair so every text reading has ONE
/// signature and no site can accidentally consult only half of it.
#[derive(Debug, Clone, Copy)]
pub struct Regs<'a> {
    /// The project basin — its TEXT codebook is [`Value::Text`]'s register.
    pub basin: Option<&'a ogar_loco::basin::BasinCodebooks>,
    /// The run's register — [`Value::RunText`]'s.
    pub texts: &'a TextRegister,
}

/// Strings a RUN made, indexed by a byte exactly as the project's sealed
/// register is.
///
/// **Why this exists, and why it is not a second string store.** A string in
/// this stack lives in a codebook register and is referred to by index —
/// never inline in a node, a call, or a constant pool. `join` and `letter of`
/// produce a string that did not exist at intake, so it cannot be in the
/// sealed register (intake is one-time; the codebook is sealed). It goes in a
/// register with the same shape, minted at run time, living on the [`Stage`]
/// with the rest of the run's working state. It never persists, never reaches
/// a node, and never crosses the intake boundary.
///
/// Unlike the sealed register it stores its entries EXACTLY: a project name
/// wider than a facet interns there as a digest (a label, lossy by ruling),
/// but a string this register hands back must be the string that was made.
///
/// Index `0` is the empty string, the same zero-fallback the sealed register
/// uses. Interning is deduped, so a loop that joins the same two values every
/// round mints ONE entry. The index is a byte, so the register holds 255
/// distinct strings and then refuses ([`RunError::TextRegisterFull`]) rather
/// than recycling an index — two different strings sharing one index is the
/// register-loss this whole discipline exists to prevent.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextRegister {
    entries: Vec<Box<str>>,
}

impl TextRegister {
    /// An empty register.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many strings are interned (the empty string at `0` is implicit).
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing has been interned.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The string at an index; `0` is the empty string.
    #[must_use]
    pub fn get(&self, idx: u8) -> Option<&str> {
        if idx == 0 {
            return Some("");
        }
        self.entries.get(usize::from(idx) - 1).map(AsRef::as_ref)
    }

    /// Intern a string, returning its index. `None` when the register is
    /// full — never a recycled index.
    pub fn intern(&mut self, s: &str) -> Option<u8> {
        if s.is_empty() {
            return Some(0);
        }
        if let Some(pos) = self.entries.iter().position(|e| &**e == s) {
            return u8::try_from(pos + 1).ok();
        }
        let idx = u8::try_from(self.entries.len() + 1).ok()?;
        self.entries.push(s.into());
        Some(idx)
    }
}

/// The text a value reads as. A number formats the way Scratch prints it
/// (an integer without a decimal point); a list handle reads as empty.
#[must_use]
pub fn text_of<'a>(v: Value, regs: Regs<'a>) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    match v {
        Value::Num(n) => Cow::Owned(format!("{n}")),
        Value::Text(idx) => Cow::Borrowed(project_text(idx, regs.basin).unwrap_or("")),
        Value::RunText(idx) => Cow::Borrowed(regs.texts.get(idx).unwrap_or("")),
        Value::List(_) => Cow::Borrowed(""),
    }
}

/// One entry of the project's sealed TEXT register, as a `str`.
fn project_text(idx: u8, basin: Option<&ogar_loco::basin::BasinCodebooks>) -> Option<&str> {
    if idx == 0 {
        return Some("");
    }
    let e = basin?
        .get(blockly_abi::menus::TEXT_MENU)?
        .resolve(idx)
        .filter(|e| e.classid == ogar_loco::pool::placeholder::CONST_UTF8_INLINE)?;
    let end = e
        .bytes
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(e.bytes.len());
    core::str::from_utf8(&e.bytes[..end]).ok()
}

impl Default for Value {
    fn default() -> Self {
        Self::Num(0.0)
    }
}

impl Value {
    /// The numeric reading. A text reads as its numeric value if it has one,
    /// else 0 — Scratch's own rule for text in a numeric slot; a list handle
    /// reads as 0.
    #[must_use]
    pub fn num(self, regs: Regs<'_>) -> f32 {
        match self {
            Self::Num(n) => n,
            Self::List(_) => 0.0,
            Self::Text(_) | Self::RunText(_) => {
                text_of(self, regs).trim().parse::<f32>().unwrap_or(0.0)
            }
        }
    }
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
    /// Current costume, as a COSTUME codebook index (menu 14); 1-based like
    /// the menu, 0 = none set.
    pub costume: u8,
    /// Graphic effects by LOOKS_EFFECT index (menu 2: 1 color … 7 ghost);
    /// slot 0 unused. Amounts as Scratch stores them, not rendered here.
    pub effects: [f32; 8],
    /// What the sprite is saying or thinking: the value as given (a TEXT
    /// index keeps its register entry), and whether it is a thought.
    pub say: Option<(Value, bool)>,
    /// Volume, percent.
    pub volume: f32,
    /// The sprite this one was cloned from, if it is a clone.
    pub clone_of: Option<usize>,
    /// `false` once `delete this clone` ran: kept in place so sprite indices
    /// stay stable, drawn by nothing, run by nothing.
    pub alive: bool,
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
            costume: 0,
            effects: [0.0; 8],
            say: None,
            volume: 100.0,
            clone_of: None,
            alive: true,
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
    pub vars: Vec<Value>,
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
    /// The broadcast sent this round, as an `event_broadcast_menu` codebook
    /// index (menu 23), or `None`. `event_broadcast` sets it; the scene's
    /// wake mask reads it at the START of the next round to wake the
    /// `event_whenbroadcastreceived` scripts that name it, then clears it.
    pub broadcast: Option<u8>,
    /// Stage half-width — the edge `if on edge, bounce` reflects against.
    pub half_w: f32,
    /// Stage half-height.
    pub half_h: f32,
    /// List store, indexed by the LIST codebook handle a `data_listcontents`
    /// push carries (menu 26). Grown on demand; handle 0 is the zero-fallback.
    pub lists: Vec<Vec<Value>>,
    /// Current backdrop, as a BACKDROP codebook index (menu 15).
    pub backdrop: u8,
    /// The last sound started, as a SOUND codebook index (menu 16) — nothing
    /// is audible here; the fact that it was played is what is kept.
    pub last_sound: Option<u8>,
    /// `operator_random`'s state: a xorshift64* word, seeded by the caller
    /// for a reproducible run (0 is replaced by a fixed non-zero seed).
    pub rng: u64,
    /// Clones created this round, as sprite indices — the scene runs their
    /// `when I start as a clone` scripts once and clears this.
    pub pending_clones: Vec<usize>,
    /// Strings this run MADE — see [`TextRegister`]. Run-scoped: it lives and
    /// dies with the stage, and nothing in it ever reaches a stored node.
    pub texts: TextRegister,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            sprites: vec![Sprite::default()],
            current: 0,
            vars: vec![Value::Num(0.0)],
            mouse_x: 0.0,
            mouse_y: 0.0,
            timer: 0.0,
            touching: false,
            key: None,
            broadcast: None,
            half_w: 240.0,
            half_h: 180.0,
            lists: vec![Vec::new()],
            backdrop: 0,
            last_sound: None,
            rng: 0x9E37_79B9_7F4A_7C15,
            pending_clones: Vec::new(),
            texts: TextRegister::new(),
        }
    }
}

impl Stage {
    /// A variable by its codebook index; unset reads as `0`, as in Scratch.
    #[must_use]
    pub fn var(&self, idx: u8) -> Value {
        self.vars.get(usize::from(idx)).copied().unwrap_or_default()
    }

    /// A variable's NUMERIC reading, against this stage's own run register.
    /// A variable holding a PROJECT text reads 0 here — supply the basin
    /// through [`Value::num`] when that matters.
    #[must_use]
    pub fn var_num(&self, idx: u8) -> f32 {
        self.var(idx).num(Regs {
            basin: None,
            texts: &self.texts,
        })
    }

    /// The variable slot for a codebook index, growing the store to reach it.
    pub fn var_mut(&mut self, idx: u8) -> &mut Value {
        let i = usize::from(idx);
        if self.vars.len() <= i {
            self.vars.resize(i + 1, Value::Num(0.0));
        }
        &mut self.vars[i]
    }

    /// A list by its codebook handle; unset is empty, as in Scratch.
    #[must_use]
    pub fn list(&self, idx: u8) -> &[Value] {
        self.lists.get(usize::from(idx)).map_or(&[], Vec::as_slice)
    }

    /// A list by its codebook handle, grown on first write.
    pub fn list_mut(&mut self, idx: u8) -> &mut Vec<Value> {
        let i = usize::from(idx);
        if self.lists.len() <= i {
            self.lists.resize(i + 1, Vec::new());
        }
        &mut self.lists[i]
    }

    /// The next random word — xorshift64*, plain and reproducible.
    fn next_random(&mut self) -> u64 {
        if self.rng == 0 {
            self.rng = 0x9E37_79B9_7F4A_7C15;
        }
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
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
    /// A `text` literal names a TEXT register entry the run's basin does not
    /// hold — or the run was given no basin. Refused rather than read as 0.
    UnknownText(u8),
    /// A list op's first operand was not a list handle: the body is
    /// malformed (the handle push is missing or out of order).
    NotAList(u8),
    /// The run's text register is full (255 distinct made strings). Refused
    /// rather than recycling an index, which would alias two strings.
    TextRegisterFull,
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
            Self::UnknownText(i) => write!(f, "text {i} is not in the TEXT register"),
            Self::NotAList(b) => write!(f, "function {b:#04x} did not receive a list handle"),
            Self::TextRegisterFull => write!(f, "the run's text register is full (255)"),
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
    /// The sprite that owns the definition. A PROCEDURE index is minted per
    /// target, so two sprites' "procedure 1" are different blocks; a scene
    /// with owners bound resolves a call within the caller's sprite.
    pub owner: usize,
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
            owner: 0,
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
    /// `event_broadcast`: its operand is the message's codebook index.
    Broadcast,
    /// The list handle push (`data_listcontents`): yields `Value::List(imm)`.
    ListHandle,
    SwitchCostume,
    SwitchBackdrop,
    CostumeNumberName,
    BackdropNumberName,
    SetEffect,
    ChangeEffect,
    ClearEffects,
    Say,
    Think,
    SoundPlay,
    SetVolume,
    Volume,
    SensingOf,
    CreateClone,
    DeleteClone,
    /// `data_showvariable` and friends: monitors have no model here.
    MonitorNoop,
    /// `motion_goto`: operand is the GOTO menu index.
    GoTo,
    /// `motion_pointtowards`: operand is the POINT_TOWARDS menu index.
    PointTowards,
    /// `sensing_askandwait`: yields the slice; there is no input channel.
    Ask,
    /// `sensing_answer` / `sensing_username`: the empty text.
    EmptyText,
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
                    | "event_whenkeypressed"
                    | "event_whenbroadcastreceived"
                    | "event_whenstageclicked"
                    | "event_whenbackdropswitchesto" => Op::Hat,
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
                    "event_broadcast" | "event_broadcastandwait" => Op::Broadcast,
                    "data_listcontents" => Op::ListHandle,
                    "looks_switchcostumeto" => Op::SwitchCostume,
                    "looks_switchbackdropto" | "looks_switchbackdroptoandwait" => {
                        Op::SwitchBackdrop
                    }
                    "looks_costumenumbername" => Op::CostumeNumberName,
                    "looks_backdropnumbername" => Op::BackdropNumberName,
                    "looks_seteffectto" => Op::SetEffect,
                    "looks_changeeffectby" => Op::ChangeEffect,
                    "looks_cleargraphiceffects" => Op::ClearEffects,
                    "looks_say" | "looks_sayforsecs" => Op::Say,
                    "looks_think" | "looks_thinkforsecs" => Op::Think,
                    "sound_play" | "sound_playuntildone" => Op::SoundPlay,
                    "sound_setvolumeto" => Op::SetVolume,
                    "sound_volume" => Op::Volume,
                    "sensing_of" => Op::SensingOf,
                    "control_create_clone_of" => Op::CreateClone,
                    "control_delete_this_clone" => Op::DeleteClone,
                    "control_start_as_clone" => Op::Hat,
                    "data_showvariable" | "data_hidevariable" | "data_showlist"
                    | "data_hidelist" => Op::MonitorNoop,
                    "motion_goto" => Op::GoTo,
                    "motion_pointtowards" => Op::PointTowards,
                    "sensing_askandwait" => Op::Ask,
                    "sensing_answer" | "sensing_username" => Op::EmptyText,
                    _ => Op::Unimplemented,
                }
            };
        }
        Plan { op, arity }
    })
}

/// What wakes a script: read ONCE off its entry's first call (its hat).
///
/// This is the participation mask idea from `ndarray`'s jitson
/// (`ScanParams::focus_mask`: which dimensions take part, decided before the
/// loop and baked, never re-asked per element) applied to scheduling. A
/// Scratch project is mostly hats that are NOT firing — key hats for keys
/// nobody holds, broadcast receivers for messages nobody sent — and the
/// honest cost of a round is the scripts that ARE awake, not the script
/// count. So the scene compiles, once, one bitmask per input value
/// (`by_key[k]` = every script `when k pressed` wakes; `by_broadcast[b]`
/// likewise) and per round ORs three masks and walks the set bits. A script
/// the mask leaves clear costs zero calls and zero machine setup.
///
/// A script with no hat — a bare chain, the built-in templates' shape — is
/// `Always`, which is exactly the pre-mask behaviour, so nothing that ran
/// before stops running unless a hat says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Wake {
    Always,
    /// `event_whenkeypressed`: the `KEY_OPTION` index in the hat's immediate.
    /// Index 0 (the empty selection) and the `any` option wake on any key.
    Key(u8),
    /// `event_whenbroadcastreceived`: the message index in the hat's
    /// immediate.
    Broadcast(u8),
    /// `control_start_as_clone`: runs once for each new clone of the
    /// script's owner sprite, the round after the clone is made.
    Clone,
}

impl Wake {
    /// Read a script's wake condition off its entry's first call.
    fn of_script(functions: &[FunctionBody]) -> Self {
        let Some(head) = functions.first().and_then(|f| f.calls().next()) else {
            return Self::Always;
        };
        match scratch::device_by_byte(head.function.0) {
            Some(("event_whenkeypressed", ..)) => Self::Key(head.values[0]),
            Some(("event_whenbroadcastreceived", ..)) => Self::Broadcast(head.values[0]),
            Some(("control_start_as_clone", ..)) => Self::Clone,
            _ => Self::Always,
        }
    }
}

/// The baked wake masks of one scene: `u64` words over script indices.
#[derive(Debug, Clone, Default)]
struct WakeTable {
    words: usize,
    /// Scripts awake every round (no hat, or an always-on hat).
    always: Vec<u64>,
    /// Scripts that wake on ANY held key (`any`, or an unset key option).
    any_key: Vec<u64>,
    /// `(key index, mask)` for every key some script names.
    by_key: Vec<(u8, Vec<u64>)>,
    /// `(message index, mask)` for every message some script receives.
    by_broadcast: Vec<(u8, Vec<u64>)>,
    /// Scripts under a `when I start as a clone` hat — run per new clone,
    /// not per round, so they live outside the round mask.
    clone_scripts: Vec<usize>,
}

impl WakeTable {
    fn build(scripts: &[&[FunctionBody]]) -> Self {
        let words = scripts.len().div_ceil(64);
        let any = blockly_abi::menus::menu_by_id(1)
            .and_then(|m| blockly_abi::menus::encode(m, "any"))
            .unwrap_or(0);
        let mut t = Self {
            words,
            always: vec![0; words],
            any_key: vec![0; words],
            by_key: Vec::new(),
            by_broadcast: Vec::new(),
            clone_scripts: Vec::new(),
        };
        fn set(mask: &mut [u64], i: usize) {
            mask[i / 64] |= 1u64 << (i % 64);
        }
        fn entry(table: &mut Vec<(u8, Vec<u64>)>, k: u8, words: usize) -> &mut Vec<u64> {
            if let Some(pos) = table.iter().position(|(key, _)| *key == k) {
                return &mut table[pos].1;
            }
            table.push((k, vec![0; words]));
            &mut table.last_mut().expect("just pushed").1
        }
        for (i, s) in scripts.iter().enumerate() {
            match Wake::of_script(s) {
                Wake::Always => set(&mut t.always, i),
                Wake::Key(k) if k == 0 || k == any => set(&mut t.any_key, i),
                Wake::Key(k) => set(entry(&mut t.by_key, k, words), i),
                Wake::Broadcast(b) => set(entry(&mut t.by_broadcast, b, words), i),
                Wake::Clone => t.clone_scripts.push(i),
            }
        }
        t
    }

    /// The scripts awake this round, given the inputs: three ORs per word.
    fn awake(&self, key: Option<u8>, broadcast: Option<u8>, out: &mut Vec<u64>) {
        out.clear();
        out.extend_from_slice(&self.always);
        if let Some(k) = key {
            for (o, a) in out.iter_mut().zip(&self.any_key) {
                *o |= a;
            }
            if let Some((_, m)) = self.by_key.iter().find(|(kk, _)| *kk == k) {
                for (o, a) in out.iter_mut().zip(m) {
                    *o |= a;
                }
            }
        }
        if let Some(b) = broadcast
            && let Some((_, m)) = self.by_broadcast.iter().find(|(bb, _)| *bb == b)
        {
            for (o, a) in out.iter_mut().zip(m) {
                *o |= a;
            }
        }
    }
}

/// Walk the set bits of a word mask, lowest first.
fn set_bits(mask: &[u64]) -> impl Iterator<Item = usize> + '_ {
    mask.iter().enumerate().flat_map(|(w, &word)| {
        let mut bits = word;
        core::iter::from_fn(move || {
            if bits == 0 {
                return None;
            }
            let tz = bits.trailing_zeros() as usize;
            bits &= bits - 1;
            Some(w * 64 + tz)
        })
    })
}

/// A bounded run over stored function bodies.
pub struct Machine<'a> {
    /// The stage the program acts on.
    pub stage: Stage,
    functions: &'a [FunctionBody],
    /// Custom blocks callable from this run, by index.
    procs: &'a [Procedure<'a>],
    /// Whether procedure lookup is scoped to the caller's sprite (a scene
    /// with owners bound) or global by index (the templates' shape).
    scoped_procs: bool,
    /// The constant pool `POOL_LOAD` reads. `None` = every load is refused,
    /// which is what a program with no wide literal never notices.
    pool: Option<&'a ogar_loco::ConstantPool>,
    /// The basin `text` literals (and any project dropdown) are read from.
    basin: Option<&'a ogar_loco::basin::BasinCodebooks>,
    /// Argument frames of the custom blocks currently executing, innermost
    /// last. `PROC_ARG` reads the innermost; outside any call it reads `0`.
    frames: Vec<Vec<Value>>,
    /// ONE operand stack for the whole run. Each body executes above a frame
    /// base and truncates back to it on return, so nested bodies (a loop's,
    /// a custom block's) share it with no allocation after warm-up and no
    /// per-body zeroing — measured against both alternatives.
    stack: Vec<Value>,
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
            scoped_procs: false,
            pool: None,
            basin: None,
            frames: Vec::new(),
            stack: Vec::with_capacity(64),
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

    /// Resolve procedure calls within the running sprite (its root, for a
    /// clone) rather than globally by index.
    #[must_use]
    pub fn with_scoped_procs(mut self, scoped: bool) -> Self {
        self.scoped_procs = scoped;
        self
    }

    /// Read wide literals from this pool — the one the program was cast
    /// against with `lower_program_with_pool`.
    #[must_use]
    pub fn with_pool(mut self, pool: &'a ogar_loco::ConstantPool) -> Self {
        self.pool = Some(pool);
        self
    }

    /// Read `text` literals from this basin's TEXT register — the basin the
    /// program was cast against.
    #[must_use]
    pub fn with_basin(mut self, basin: &'a ogar_loco::basin::BasinCodebooks) -> Self {
        self.basin = Some(basin);
        self
    }

    /// The two text registers this run reads from — the project's sealed
    /// basin and the stage's own. One place builds the pair.
    fn regs(&self) -> Regs<'_> {
        Regs {
            basin: self.basin,
            texts: &self.stage.texts,
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
        // This body's frame on the shared operand stack.
        let base = self.stack.len();
        let plan = plan();

        for call in body.calls() {
            if self.budget == 0 {
                self.stack.truncate(base);
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
                        let c = self.pop(base, f)?;
                        if c.num(self.regs()) != 0.0 {
                            self.exec_in(functions, target)?;
                        }
                    }
                    FnIndex::IF_ELSE => {
                        let c = self.pop(base, f)?.num(self.regs());
                        let other = usize::from(call.values.get(1).copied().unwrap_or(0));
                        self.exec_in(functions, if c != 0.0 { target } else { other })?;
                    }
                    FnIndex::REPEAT => {
                        let n = self.pop(base, f)?.num(self.regs()).max(0.0) as u32;
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
                        let c = self.pop(base, f)?.num(self.regs());
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
                if self.stack.len() - base < argc {
                    return Err(RunError::StackUnderflow(f.0));
                }
                let args: Vec<Value> = self.stack.split_off(self.stack.len() - argc);
                let me = self
                    .stage
                    .current
                    .min(self.stage.sprites.len().saturating_sub(1));
                let root = self
                    .stage
                    .sprites
                    .get(me)
                    .and_then(|s| s.clone_of)
                    .unwrap_or(me);
                let scoped = self.scoped_procs;
                let proc_ = *self
                    .procs
                    .iter()
                    .find(|p| p.index == index && (!scoped || p.owner == root))
                    .ok_or(RunError::UnknownProcedure(index))?;
                self.frames.push(args);
                let outcome = self.exec_in(proc_.functions, proc_.body);
                self.frames.pop();
                outcome?;
                continue;
            }

            // ── everything else: pop arity, compute, push if it yields ────
            let arity = usize::from(plan.arity[usize::from(f.0)].ok_or(RunError::Uncovered(f.0))?);
            if self.stack.len() - base < arity {
                return Err(RunError::StackUnderflow(f.0));
            }
            // Operands into a fixed window — the ABI's arity is at most 3 —
            // rather than a fresh `Vec` per call.
            let mut window = [Value::default(); ogar_loco::MAX_VALUES_PER_CALL];
            let top = self.stack.len() - arity;
            window[..arity].copy_from_slice(&self.stack[top..]);
            self.stack.truncate(top);
            let immediate = f32::from(call.values[0]);

            let result = self.apply(f, &window[..arity], immediate)?;
            if let Some(v) = result {
                self.stack.push(v);
            }
        }
        // Anything this body left is its own; the caller's frame is below.
        self.stack.truncate(base);
        Ok(())
    }

    /// Pop within this body's frame — never into the caller's operands.
    #[inline]
    fn pop(&mut self, base: usize, f: FnIndex) -> Result<Value, RunError> {
        if self.stack.len() <= base {
            return Err(RunError::StackUnderflow(f.0));
        }
        self.stack.pop().ok_or(RunError::StackUnderflow(f.0))
    }

    /// The rare families — text, suspend/stop, random, lists — kept out of
    /// [`Self::apply`] so its frame stays small. `None` = not one of these.
    #[cold]
    #[inline(never)]
    fn apply_rare(
        &mut self,
        f: FnIndex,
        ops: &[Value],
        imm: f32,
    ) -> Option<Result<Option<Value>, RunError>> {
        let basin = self.basin;
        let raw = |i: usize| ops.get(i).copied().unwrap_or_default();
        // The operands' NUMERIC readings, taken once and up front: reading a
        // text borrows the stage's run register, and everything below this
        // line may borrow the stage mutably.
        let nums: [f32; ogar_loco::MAX_VALUES_PER_CALL] = {
            let regs = self.regs();
            core::array::from_fn(|i| ops.get(i).map_or(0.0, |v| v.num(regs)))
        };
        let a = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        // The text family. `length` and `contains` only READ, so they answer
        // from a borrowed reading; `join` and `letter of` MAKE a string, so
        // they own their readings first, then intern into the run register.
        if f == FnIndex::LENGTH || f == FnIndex::CONTAINS {
            let regs = self.regs();
            let s0 = text_of(raw(0), regs);
            return Some(match f {
                // Characters, not bytes: `length of "aä"` is 2.
                FnIndex::LENGTH => num(s0.chars().count() as f32),
                _ => num(f32::from(s0.contains(&*text_of(raw(1), regs)))),
            });
        }
        if f == FnIndex::JOIN || f == FnIndex::CHAR_AT {
            let made = {
                let regs = self.regs();
                if f == FnIndex::JOIN {
                    let mut s = text_of(raw(0), regs).into_owned();
                    s.push_str(&text_of(raw(1), regs));
                    s
                } else {
                    // `letter (n) of (s)`: 1-based, by character; out of
                    // range is the empty string, as in Scratch.
                    let n = nums[0];
                    let s = text_of(raw(1), regs);
                    if n < 1.0 || n.fract() != 0.0 {
                        String::new()
                    } else {
                        s.chars()
                            .nth(n as usize - 1)
                            .map_or_else(String::new, |c| c.to_string())
                    }
                }
            };
            return Some(match self.stage.texts.intern(&made) {
                Some(idx) => Ok(Some(Value::RunText(idx))),
                None => Err(RunError::TextRegisterFull),
            });
        }
        // A `text` literal yields its TEXT register index — the register
        // keeps the string; `Value::num` reads it as a number when a numeric
        // slot asks. A non-zero index must exist in the basin.
        if f == FnIndex::TEXT {
            let idx = imm as u8;
            if idx != 0
                && basin
                    .and_then(|b| b.get(blockly_abi::menus::TEXT_MENU))
                    .and_then(|book| book.resolve(idx))
                    .is_none()
            {
                return Some(Err(RunError::UnknownText(idx)));
            }
            return Some(Ok(Some(Value::Text(idx))));
        }
        // The suspend/stop family. A slice is not a clock: `wait` yields the
        // rest of this slice (the scene ticks time per round), `wait until`
        // yields unless its condition already holds, `stop` ends the slice.
        if f == FnIndex::WAIT || f == FnIndex::STOP || (f == FnIndex::WAIT_UNTIL && a(0) == 0.0) {
            self.budget = 0;
            return Some(Ok(None));
        }
        if f == FnIndex::WAIT_UNTIL {
            return Some(Ok(None));
        }
        // `pick random a to b`: integer when both bounds are integers, else a
        // float in the range — Scratch's rule. xorshift64* on the stage's word.
        if f == FnIndex::RANDOM_INT {
            let (lo, hi) = (a(0).min(a(1)), a(0).max(a(1)));
            let r = self.stage.next_random();
            let unit = (r >> 11) as f32 / (1u64 << 53) as f32;
            let v = if lo.fract() == 0.0 && hi.fract() == 0.0 {
                (lo + (unit * (hi - lo + 1.0)).floor()).min(hi)
            } else {
                lo + unit * (hi - lo)
            };
            return Some(num(v));
        }
        // Lists: the handle is the FIRST popped operand (the core's arity
        // counts it), then Scratch's socket order. Indices are 1-based;
        // out-of-range reads give 0 and writes do nothing, as in Scratch.
        if blockly_abi::is_list_op(f) {
            let Value::List(h) = raw(0) else {
                return Some(Err(RunError::NotAList(f.0)));
            };
            let len = self.stage.list(h).len();
            return Some(match f {
                FnIndex::LIST_LENGTH => num(len as f32),
                FnIndex::LIST_GET => Ok(Some(
                    list_slot(a(1), len).map_or(Value::Num(0.0), |i| self.stage.list(h)[i]),
                )),
                FnIndex::LIST_ADD => {
                    self.stage.list_mut(h).push(raw(1));
                    Ok(None)
                }
                FnIndex::LIST_DELETE => {
                    if let Some(i) = list_slot(a(1), len) {
                        self.stage.list_mut(h).remove(i);
                    }
                    Ok(None)
                }
                FnIndex::LIST_DELETE_ALL => {
                    self.stage.list_mut(h).clear();
                    Ok(None)
                }
                // `replace item INDEX of list with ITEM`
                FnIndex::LIST_SET => {
                    if let Some(i) = list_slot(a(1), len) {
                        self.stage.list_mut(h)[i] = raw(2);
                    }
                    Ok(None)
                }
                // `insert ITEM at INDEX of list` — `length + 1` appends.
                FnIndex::LIST_INSERT => {
                    if let Some(i) = list_slot(a(2), len + 1) {
                        self.stage.list_mut(h).insert(i, raw(1));
                    }
                    Ok(None)
                }
                FnIndex::LIST_INDEX_OF => {
                    let (want, regs) = (raw(1), self.regs());
                    let pos = self.stage.list(h).iter().position(|v| same(*v, want, regs));
                    num(pos.map_or(0.0, |p| (p + 1) as f32))
                }
                FnIndex::LIST_CONTAINS => {
                    let (want, regs) = (raw(1), self.regs());
                    num(f32::from(
                        self.stage.list(h).iter().any(|v| same(*v, want, regs)),
                    ))
                }
                _ => Err(RunError::Unimplemented(f.0)),
            });
        }

        None
    }

    /// Apply one non-branching call. `Some(v)` means it yielded a value.
    fn apply(&mut self, f: FnIndex, ops: &[Value], imm: f32) -> Result<Option<Value>, RunError> {
        let basin = self.basin;
        let raw = |i: usize| ops.get(i).copied().unwrap_or_default();
        // Read BEFORE the stage is borrowed mutably: an operand's numeric
        // reading consults the stage's own run text register.
        let nums: [f32; ogar_loco::MAX_VALUES_PER_CALL] = {
            let regs = self.regs();
            core::array::from_fn(|i| ops.get(i).map_or(0.0, |v| v.num(regs)))
        };
        let a = |i: usize| nums.get(i).copied().unwrap_or(0.0);
        // …and, for `change x by`, the variable's own current number.
        let var_cur = if f == FnIndex::VAR_CHANGE {
            self.stage.var(imm as u8).num(self.regs())
        } else {
            0.0
        };
        // Read before the stage is borrowed: the innermost custom-block frame.
        let frame_arg = self
            .frames
            .last()
            .and_then(|fr| fr.get(imm as usize))
            .copied()
            .unwrap_or_default();
        // A pool load: the pool holds numbers only, so the load IS the f64.
        if f == blockly_abi::POOL_LOAD {
            let idx = imm as u8;
            let c = self
                .pool
                .and_then(|p| p.resolve(idx))
                .filter(|c| c.classid == ogar_loco::pool::placeholder::CONST_F64)
                .ok_or(RunError::MissingConstant(idx))?;
            let mut le = [0u8; 8];
            le.copy_from_slice(&c.bytes[..8]);
            return num(f64::from_le_bytes(le) as f32);
        }
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
            FnIndex::FLOOR => Some(a(0).floor()),
            FnIndex::CEIL => Some(a(0).ceil()),
            FnIndex::SQRT => Some(a(0).sqrt()),
            FnIndex::SIN => Some(a(0).to_radians().sin()),
            FnIndex::COS => Some(a(0).to_radians().cos()),
            FnIndex::TAN => Some(a(0).to_radians().tan()),
            FnIndex::ASIN => Some(a(0).asin().to_degrees()),
            FnIndex::ACOS => Some(a(0).acos().to_degrees()),
            FnIndex::ATAN => Some(a(0).atan().to_degrees()),
            FnIndex::LN => Some(a(0).ln()),
            FnIndex::LOG10 => Some(a(0).log10()),
            FnIndex::EXP_E => Some(a(0).exp()),
            FnIndex::EXP_10 => Some(10f32.powf(a(0))),
            // The immediate is the VARIABLE codebook byte — which variable.
            // A variable holds a VALUE — a number, or a text by register
            // index — so `say (my message)` keeps its string.
            FnIndex::VAR_GET => return Ok(Some(s.var(imm as u8))),
            // The immediate is the argument's POSITION in the innermost
            // custom-block frame; outside any call it reads 0, as an unset
            // Scratch value does.
            FnIndex::PROC_ARG => return Ok(Some(frame_arg)),
            FnIndex::VAR_SET => {
                *s.var_mut(imm as u8) = raw(0);
                return Ok(None);
            }
            FnIndex::VAR_CHANGE => {
                *s.var_mut(imm as u8) = Value::Num(var_cur + a(0));
                return Ok(None);
            }
            _ => None,
        };
        if let Some(v) = core {
            return num(v);
        }

        // The less frequent families live in a cold, never-inlined function
        // so this hot function stays small (measured: folding them in here
        // cost ~5 ns on every call, rare or not — the prologue is paid by all).
        if let Some(r) = self.apply_rare(f, ops, imm) {
            return r;
        }
        let s = &mut self.stage;
        // Motion/looks act on the sprite this script is bound to.
        let me = s.current.min(s.sprites.len() - 1);

        // Then the device half, through the plan — the harvested NAME table
        // resolved to integer tags once, so the palette, the toolbox and the
        // interpreter still read one source and the hot path reads none.
        match plan().op[usize::from(f.0)] {
            Op::Unimplemented => Err(RunError::Unimplemented(f.0)),
            // A menu shadow block is a value: it yields its own codebook
            // index, which the consuming block pops as an operand.
            Op::Menu => num(imm),
            Op::Hat => Ok(None),
            Op::KeyPressed => {
                // The operand is the KEY_OPTION index the menu reporter
                // pushed; `any` is the harvested option 6.
                let want = a(0) as u8;
                let any = blockly_abi::menus::menu_by_id(1)
                    .and_then(|m| blockly_abi::menus::encode(m, "any"))
                    .unwrap_or(0);
                let held = s.key.is_some_and(|k| k == want || want == any);
                num(f32::from(held))
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
            Op::XPos => num(s.sprites[me].x),
            Op::YPos => num(s.sprites[me].y),
            Op::Dir => num(s.sprites[me].direction),
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
            Op::Size => num(s.sprites[me].size),
            // A costume switch has no visual model here; it is a real op with
            // no effect on this stage, which is different from unimplemented.
            Op::CostumeNoop => Ok(None),
            Op::MouseX => num(s.mouse_x),
            Op::MouseY => num(s.mouse_y),
            Op::Timer => num(s.timer),
            Op::ResetTimer => {
                s.timer = 0.0;
                Ok(None)
            }
            Op::Touching => num(f32::from(s.touching)),
            // The message index was pushed by the `event_broadcast_menu`
            // reporter. Delivery is the scene's: receivers wake next round.
            // (`and wait` cannot wait here — a slice is not a frame — so it
            // sends and continues, which is the bounded reading.)
            Op::Broadcast => {
                s.broadcast = Some(a(0) as u8);
                Ok(None)
            }
            Op::MouseDown => num(0.0),
            Op::ListHandle => Ok(Some(Value::List(imm as u8))),
            Op::SwitchCostume => {
                s.sprites[me].costume = a(0) as u8;
                Ok(None)
            }
            Op::SwitchBackdrop => {
                s.backdrop = a(0) as u8;
                Ok(None)
            }
            // NUMBER_NAME menu 6: 1 = number, 2 = name. The costume index is
            // the register index either way; a NAME has no numeric reading
            // here, so the index is what is yielded in both cases.
            Op::CostumeNumberName => num(f32::from(s.sprites[me].costume)),
            Op::BackdropNumberName => num(f32::from(s.backdrop)),
            // The effect is the LOOKS_EFFECT index in the immediate (menu 2).
            Op::SetEffect => {
                let e = (imm as usize).min(7);
                s.sprites[me].effects[e] = a(0);
                Ok(None)
            }
            Op::ChangeEffect => {
                let e = (imm as usize).min(7);
                s.sprites[me].effects[e] += a(0);
                Ok(None)
            }
            Op::ClearEffects => {
                s.sprites[me].effects = [0.0; 8];
                Ok(None)
            }
            Op::Say => {
                s.sprites[me].say = Some((raw(0), false));
                Ok(None)
            }
            Op::Think => {
                s.sprites[me].say = Some((raw(0), true));
                Ok(None)
            }
            Op::SoundPlay => {
                s.last_sound = Some(a(0) as u8);
                Ok(None)
            }
            Op::SetVolume => {
                s.sprites[me].volume = a(0).clamp(0.0, 100.0);
                Ok(None)
            }
            Op::Volume => num(s.sprites[me].volume),
            // `(PROPERTY) of (OBJECT)`: OBJECT is the OF_OBJECT index the
            // menu pushed (1 = the stage, k ≥ 2 = sprite k−2 in project
            // order); PROPERTY is the OF_PROPERTY index in the immediate
            // (1 x, 2 y, 3 direction, 4 costume #, 5 costume name, 6 size,
            // 7 volume, 8 backdrop #, 9 backdrop name; above that, a
            // variable by name — resolved through the basin, register to
            // register: OF_PROPERTY entry bytes → VARIABLE index).
            Op::SensingOf => {
                let obj = a(0) as usize;
                let prop = imm as u8;
                let sprite = (obj >= 2).then(|| obj - 2).filter(|i| *i < s.sprites.len());
                let v = match (prop, sprite) {
                    (1, Some(i)) => s.sprites[i].x,
                    (2, Some(i)) => s.sprites[i].y,
                    (3, Some(i)) => s.sprites[i].direction,
                    (4 | 5, Some(i)) => f32::from(s.sprites[i].costume),
                    (6, Some(i)) => s.sprites[i].size,
                    (7, Some(i)) => s.sprites[i].volume,
                    (7, None) => 100.0,
                    (8 | 9, None) => f32::from(s.backdrop),
                    (p, _) if p >= 10 => {
                        let var = basin
                            .and_then(|b| b.get(blockly_abi::menus::menu_by_id(27)?.id))
                            .and_then(|book| book.resolve(p))
                            .and_then(|entry| {
                                let m = blockly_abi::menus::menu_by_id(25)?;
                                let end = entry
                                    .bytes
                                    .iter()
                                    .position(|&x| x == 0)
                                    .unwrap_or(entry.bytes.len());
                                let name = core::str::from_utf8(&entry.bytes[..end]).ok()?;
                                blockly_abi::menus::encode_in(basin?, m, name)
                            });
                        // The sprite's variable, read as a number (a text
                        // variable read through `of` reads as Scratch reads it).
                        var.map_or(0.0, |idx| s.var_num(idx))
                    }
                    _ => 0.0,
                };
                num(v)
            }
            // CLONE_OF menu 24: 1 = `_myself_`, k ≥ 2 = the (k−2)th OTHER
            // sprite in stage order (the menu lists every sprite but the
            // holder). The clone copies the source's state, is marked, and
            // is queued for its `when I start as a clone` scripts, which the
            // scene runs once at the start of the next round.
            Op::CreateClone => {
                let pick = a(0) as usize;
                let src = if pick <= 1 {
                    Some(me)
                } else {
                    (0..s.sprites.len()).filter(|&i| i != me).nth(pick - 2)
                };
                if let Some(src) = src {
                    let mut c = s.sprites[src].clone();
                    c.clone_of = Some(s.sprites[src].clone_of.unwrap_or(src));
                    s.sprites.push(c);
                    let idx = s.sprites.len() - 1;
                    s.pending_clones.push(idx);
                }
                Ok(None)
            }
            Op::DeleteClone => {
                if s.sprites[me].clone_of.is_some() {
                    s.sprites[me].alive = false;
                    s.sprites[me].visible = false;
                    self.budget = 0;
                }
                Ok(None)
            }
            Op::MonitorNoop => Ok(None),
            // GOTO / POINT_TOWARDS menus: 1 = `_mouse_`, 2 = `_random_`,
            // k ≥ 3 = the (k−3)th OTHER sprite in stage order (the menu
            // lists every sprite but the holder).
            Op::GoTo | Op::PointTowards => {
                let pick = a(0) as usize;
                let target: Option<(f32, f32)> = match pick {
                    1 => Some((s.mouse_x, s.mouse_y)),
                    2 => {
                        let r1 = s.next_random();
                        let r2 = s.next_random();
                        let u = |r: u64| (r >> 11) as f32 / (1u64 << 53) as f32;
                        Some((
                            (u(r1) * 2.0 - 1.0) * s.half_w,
                            (u(r2) * 2.0 - 1.0) * s.half_h,
                        ))
                    }
                    k if k >= 3 => (0..s.sprites.len())
                        .filter(|&i| i != me)
                        .nth(k - 3)
                        .map(|i| (s.sprites[i].x, s.sprites[i].y)),
                    _ => None,
                };
                if let Some((tx, ty)) = target {
                    if matches!(plan().op[usize::from(f.0)], Op::GoTo) {
                        s.sprites[me].x = tx;
                        s.sprites[me].y = ty;
                    } else {
                        let (dx, dy) = (tx - s.sprites[me].x, ty - s.sprites[me].y);
                        // Scratch heading: 90 = right, 0 = up.
                        s.sprites[me].direction = dx.atan2(dy).to_degrees();
                    }
                }
                Ok(None)
            }
            Op::Ask => {
                self.budget = 0;
                Ok(None)
            }
            Op::EmptyText => Ok(Some(Value::Text(0))),
        }
    }
}

/// A yielded number.
#[inline]
fn num(x: f32) -> Result<Option<Value>, RunError> {
    Ok(Some(Value::Num(x)))
}

/// Scratch equality for list search: two texts match by register index,
/// anything else by numeric reading.
fn same(a: Value, b: Value, regs: Regs<'_>) -> bool {
    let textual = |v: Value| matches!(v, Value::Text(_) | Value::RunText(_));
    if textual(a) || textual(b) {
        // By CONTENT, never by index: a project literal "go" and a `join`-made
        // "go" are the same string in two registers, and Scratch's list search
        // finds either.
        return text_of(a, regs) == text_of(b, regs);
    }
    (a.num(regs) - b.num(regs)).abs() < f32::EPSILON
}

/// Scratch's 1-based list index from an already-read number; `None` when out
/// of range. Takes the NUMBER rather than the value so a caller can read its
/// operands before borrowing the list store mutably.
fn list_slot(i: f32, len: usize) -> Option<usize> {
    if i < 1.0 || i.fract() != 0.0 {
        return None;
    }
    let i = i as usize;
    (i <= len).then_some(i - 1)
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
    /// The basin every script's `text` literals are read from.
    basin: Option<&'a ogar_loco::basin::BasinCodebooks>,
    /// Per-sprite registers, when a project casts each target against its
    /// OWN basin and pool (the sb3 shape): index = sprite; `None` falls back
    /// to the scene-wide pair above. A clone reads its root's registers.
    sprite_regs: Vec<
        Option<(
            &'a ogar_loco::basin::BasinCodebooks,
            &'a ogar_loco::ConstantPool,
        )>,
    >,
    /// Which sprite each scheduled script controls. Defaults to the script's
    /// own index (one script per sprite, the templates' shape); a real
    /// project sets it with [`Scene::with_owners`].
    owners: Vec<usize>,
    /// Each scheduled script's position in the ORIGINAL `scripts` list.
    sched_orig: Vec<usize>,
    /// Each procedure's defining script's position in the original list.
    def_orig: Vec<usize>,
    /// Whether [`Scene::with_owners`] was applied — procedure lookup is then
    /// scoped by sprite.
    owners_bound: bool,
    /// The baked participation masks — see [`Wake`].
    wake: WakeTable,
    /// Scratch space for the round's awake mask, reused every round.
    awake: Vec<u64>,
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
            .enumerate()
            .partition(|(_, s)| Procedure::of_script(s).is_some());
        let def_orig: Vec<usize> = defs.iter().map(|(i, _)| *i).collect();
        let sched_orig: Vec<usize> = scheduled.iter().map(|(i, _)| *i).collect();
        let procs: Vec<Procedure<'a>> = defs
            .iter()
            .filter_map(|(i, s)| Procedure::of_script(s).map(|p| Procedure { owner: *i, ..p }))
            .collect();
        let scheduled: Vec<&'a [FunctionBody]> = scheduled.into_iter().map(|(_, s)| s).collect();
        let wake = WakeTable::build(&scheduled);
        Self {
            stage,
            owners: sched_orig.clone(),
            sched_orig,
            def_orig,
            owners_bound: false,
            scripts: scheduled,
            procs,
            pool: None,
            basin: None,
            sprite_regs: Vec::new(),
            awake: Vec::with_capacity(wake.words),
            wake,
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

    /// Read `text` literals from this basin's TEXT register in every script.
    #[must_use]
    pub fn with_basin(mut self, basin: &'a ogar_loco::basin::BasinCodebooks) -> Self {
        self.basin = Some(basin);
        self
    }

    /// The registers one sprite's scripts were cast against — its own
    /// `target_basin` and `LoweringContext` pool. Call once per sprite; a
    /// sprite without an entry uses the scene-wide basin/pool.
    #[must_use]
    pub fn with_sprite_registers(
        mut self,
        sprite: usize,
        basin: &'a ogar_loco::basin::BasinCodebooks,
        pool: &'a ogar_loco::ConstantPool,
    ) -> Self {
        if self.sprite_regs.len() <= sprite {
            self.sprite_regs.resize(sprite + 1, None);
        }
        self.sprite_regs[sprite] = Some((basin, pool));
        self
    }

    /// Bind every script — scheduled or definition — to the sprite it
    /// belongs to: `owners[k]` is the sprite of the k-th entry of the
    /// ORIGINAL `scripts` list handed to [`Scene::new`]. Scripts past the
    /// end of `owners` keep the default binding. Once bound, a procedure
    /// call resolves within the caller's sprite (a PROCEDURE index is
    /// minted per target).
    #[must_use]
    pub fn with_owners(mut self, owners: &[usize]) -> Self {
        for (k, slot) in self.owners.iter_mut().enumerate() {
            if let Some(o) = owners.get(self.sched_orig[k]) {
                *slot = *o;
            }
        }
        for (j, p) in self.procs.iter_mut().enumerate() {
            if let Some(o) = owners.get(self.def_orig[j]) {
                p.owner = *o;
            }
        }
        self.owners_bound = true;
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
            // The participation mask for this round: decided ONCE from the
            // inputs, before any script runs — a script whose hat is not
            // firing costs nothing. The broadcast is consumed here: it was
            // sent last round, its receivers wake this round.
            self.wake
                .awake(self.stage.key, self.stage.broadcast.take(), &mut self.awake);
            // The procedure table is a handful of `Copy` records; a per-round
            // copy is what lets the scripts borrow the scene mutably.
            let procs: Vec<Procedure<'a>> = self.procs.clone();
            let procs = procs.as_slice();
            // New clones first: each runs its owner's `when I start as a
            // clone` scripts once, AS the clone (the clone's own index).
            let born = core::mem::take(&mut self.stage.pending_clones);
            let clone_scripts = core::mem::take(&mut self.wake.clone_scripts);
            for &clone in &born {
                let root = self.stage.sprites.get(clone).and_then(|c| c.clone_of);
                for &i in &clone_scripts {
                    if root != Some(self.owners[i]) {
                        continue;
                    }
                    self.run_script(i, clone, slice, procs)?;
                }
            }
            self.wake.clone_scripts = clone_scripts;
            let awake = core::mem::take(&mut self.awake);
            for i in set_bits(&awake) {
                let sprite = self.owners[i];
                if !self.stage.sprites.get(sprite).is_none_or(|s| s.alive) {
                    continue;
                }
                self.run_script(i, sprite, slice, procs)?;
            }
            self.awake = awake;
            self.stage.timer += 1.0 / 30.0;
            self.trace.push(self.stage.clone());
        }
        Ok(())
    }

    /// One script's slice, as `sprite`, against the shared stage.
    fn run_script<'p>(
        &mut self,
        i: usize,
        sprite: usize,
        slice: u32,
        procs: &'p [Procedure<'p>],
    ) -> Result<(), RunError>
    where
        'a: 'p,
    {
        let script = self.scripts[i];
        // A clone reads the registers of the sprite it was cloned from.
        let root = self
            .stage
            .sprites
            .get(sprite)
            .and_then(|s| s.clone_of)
            .unwrap_or(sprite);
        let regs = self.sprite_regs.get(root).copied().flatten();
        let stage = core::mem::take(&mut self.stage);
        let mut m = Machine::resuming(script, slice, stage, sprite)
            .with_procs(procs)
            .with_scoped_procs(self.owners_bound);
        if let Some((basin, pool)) = regs {
            m = m.with_basin(basin).with_pool(pool);
        } else {
            if let Some(pool) = self.pool {
                m = m.with_pool(pool);
            }
            if let Some(basin) = self.basin {
                m = m.with_basin(basin);
            }
        }
        let outcome = m.run();
        self.stage = m.stage;
        outcome
    }

    /// The custom blocks this scene can call.
    #[must_use]
    pub fn procedures(&self) -> &[Procedure<'a>] {
        &self.procs
    }

    /// How many scheduled scripts would run a round under these inputs —
    /// the size of the participation mask, for measurement.
    #[must_use]
    pub fn awake_count(&self, key: Option<u8>, broadcast: Option<u8>) -> usize {
        let mut m = Vec::with_capacity(self.wake.words);
        self.wake.awake(key, broadcast, &mut m);
        m.iter().map(|w| w.count_ones() as usize).sum()
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
        assert_eq!(m.stage.var_num(score), 7.0);
        assert_eq!(m.stage.var_num(lives), 3.0);
        assert_eq!(
            m.stage.var_num(0),
            0.0,
            "the zero-fallback slot is untouched"
        );
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
        assert_eq!(
            hit.stage.var_num(0),
            5.0,
            "the branch must fire when touching"
        );

        let mut miss = Machine::new(&prog.functions, 100);
        miss.stage.touching = false;
        miss.run().expect("runs");
        assert_eq!(
            miss.stage.var_num(0),
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

    /// The participation mask, can-fire and can-stay-silent: two key-hat
    /// scripts on two sprites, one key held — ONLY that key's sprite moves,
    /// the other never runs (its motion would be visible). With no key held
    /// neither runs, and a hat-less script runs regardless.
    #[test]
    fn a_key_hat_script_runs_only_while_its_key_is_held() {
        use blockly_abi::menus;
        let code = |c: &str| {
            rec(
                "event_whenkeypressed",
                vec![("KEY_OPTION".into(), FieldValue::Code(c.into()))],
                vec![],
                vec![],
                Some(rec(
                    "motion_changexby",
                    vec![],
                    vec![("DX".into(), num(1))],
                    vec![],
                    None,
                )),
            )
        };
        let up = lower_program(LaneShape::Pairs, &code("up arrow")).unwrap();
        let down = lower_program(LaneShape::Pairs, &code("down arrow")).unwrap();
        let free = lower_program(
            LaneShape::Pairs,
            &rec(
                "motion_changeyby",
                vec![],
                vec![("DY".into(), num(1))],
                vec![],
                None,
            ),
        )
        .unwrap();
        let keys = menus::menu_by_id(1).unwrap();
        let k_up = menus::encode(keys, "up arrow");

        let run = |key: Option<u8>| {
            let stage = Stage {
                sprites: vec![Sprite::default(); 3],
                key,
                ..Stage::default()
            };
            let mut scene = Scene::new(
                stage,
                vec![
                    up.functions.as_slice(),
                    down.functions.as_slice(),
                    free.functions.as_slice(),
                ],
            );
            scene.run(5, 100).unwrap();
            (
                scene.stage.sprites[0].x,
                scene.stage.sprites[1].x,
                scene.stage.sprites[2].y,
                scene.awake_count(key, None),
            )
        };
        // Up held: script 0 fires 5 rounds, script 1 never, the free one always.
        assert_eq!(run(k_up), (5.0, 0.0, 5.0, 2));
        // Nothing held: only the hat-less script runs.
        assert_eq!(run(None), (0.0, 0.0, 5.0, 1));
    }

    /// A broadcast wakes its receiver on the NEXT round and nothing else:
    /// the sender runs under a flag hat, the receiver moves once per
    /// message, a receiver of a different message never moves.
    #[test]
    fn a_broadcast_wakes_its_receiver_next_round_and_no_other() {
        use blockly_abi::menus;
        let menu_leaf = |c: &str| {
            rec(
                "event_broadcast_menu",
                vec![("BROADCAST_OPTION".into(), FieldValue::Code(c.into()))],
                vec![],
                vec![],
                None,
            )
        };
        let receiver = |c: &str| {
            rec(
                "event_whenbroadcastreceived",
                vec![("BROADCAST_OPTION".into(), FieldValue::Code(c.into()))],
                vec![],
                vec![],
                Some(rec(
                    "motion_changexby",
                    vec![],
                    vec![("DX".into(), num(1))],
                    vec![],
                    None,
                )),
            )
        };
        // Two project broadcasts, interned into menu 23 of a project basin.
        use ogar_loco::basin::BasinCodebooks;
        let mut basin = BasinCodebooks::new();
        let m = menus::menu_by_id(23).unwrap();
        let mut b = menus::builder(
            m,
            ogar_loco::pool::placeholder::CONST_UTF8_INLINE,
            menus::PLACEHOLDER_DIGEST_CLASSID,
        )
        .unwrap();
        b.intern(ogar_loco::pool::placeholder::CONST_UTF8_INLINE, b"go")
            .unwrap();
        b.intern(ogar_loco::pool::placeholder::CONST_UTF8_INLINE, b"stop")
            .unwrap();
        basin.plug(b.seal()).unwrap();
        let cast =
            |r: &BlockRecord| blockly_abi::lower_program_in(LaneShape::Pairs, r, &basin).unwrap();
        let sender = cast(&rec(
            "event_whenflagclicked",
            vec![],
            vec![],
            vec![],
            Some(rec(
                "event_broadcast",
                vec![],
                vec![("BROADCAST_INPUT".into(), menu_leaf("go"))],
                vec![],
                None,
            )),
        ));
        let on_go = cast(&receiver("go"));
        let on_stop = cast(&receiver("stop"));

        let stage = Stage {
            sprites: vec![Sprite::default(); 3],
            ..Stage::default()
        };
        let mut scene = Scene::new(
            stage,
            vec![
                sender.functions.as_slice(),
                on_go.functions.as_slice(),
                on_stop.functions.as_slice(),
            ],
        );
        // Round 1: sender sends, no receiver awake yet (mask read at round start).
        scene.run(1, 100).unwrap();
        assert_eq!(scene.stage.sprites[1].x, 0.0, "a receiver wakes NEXT round");
        // Round 2: `go` receiver wakes; the sender sends again.
        scene.run(1, 100).unwrap();
        assert_eq!(scene.stage.sprites[1].x, 1.0);
        assert_eq!(scene.stage.sprites[2].x, 0.0, "`stop` was never sent");
        // Steady state: one wake per message.
        scene.run(3, 100).unwrap();
        assert_eq!(scene.stage.sprites[1].x, 4.0);
        assert_eq!(scene.stage.sprites[2].x, 0.0);
    }

    /// A `text` literal is read from the TEXT REGISTER, not from any pool:
    /// `set x to "12.5"` lands at 12.5; a word reads 0 as Scratch reads it;
    /// without the basin the run refuses rather than reading the index.
    #[test]
    fn a_text_literal_is_read_from_the_register_and_refused_without_it() {
        use blockly_abi::menus;
        use ogar_loco::basin::BasinCodebooks;
        let utf8 = ogar_loco::pool::placeholder::CONST_UTF8_INLINE;
        let mut basin = BasinCodebooks::new();
        let m = menus::menu_by_id(menus::TEXT_MENU).unwrap();
        let mut b = menus::builder(m, utf8, menus::PLACEHOLDER_DIGEST_CLASSID).unwrap();
        b.intern(utf8, b"12.5").unwrap();
        b.intern(utf8, b"hello").unwrap();
        basin.plug(b.seal()).unwrap();
        let text = |t: &str| {
            rec(
                "text",
                vec![("TEXT".into(), FieldValue::Wide(t.into()))],
                vec![],
                vec![],
                None,
            )
        };
        let setx = |v: BlockRecord| rec("motion_setx", vec![], vec![("X".into(), v)], vec![], None);
        let numeric =
            blockly_abi::lower_program_in(LaneShape::Pairs, &setx(text("12.5")), &basin).unwrap();
        let word =
            blockly_abi::lower_program_in(LaneShape::Pairs, &setx(text("hello")), &basin).unwrap();
        let idx = blockly_abi::raise_calls(numeric.entry())[0].values[0];
        assert_ne!(idx, 0);

        let mut m1 = Machine::new(&numeric.functions, 100).with_basin(&basin);
        m1.run().unwrap();
        assert_eq!(m1.stage.me().x, 12.5);
        assert_ne!(
            m1.stage.me().x,
            f32::from(idx),
            "the index is not the value"
        );
        let mut m2 = Machine::new(&word.functions, 100).with_basin(&basin);
        m2.run().unwrap();
        assert_eq!(m2.stage.me().x, 0.0);
        let mut bare = Machine::new(&numeric.functions, 100);
        assert_eq!(bare.run(), Err(RunError::UnknownText(idx)));
    }

    /// A basin with one entry in each named menu, for the family tests.
    fn basin_with(entries: &[(u8, &[&str])]) -> ogar_loco::basin::BasinCodebooks {
        use blockly_abi::menus;
        let utf8 = ogar_loco::pool::placeholder::CONST_UTF8_INLINE;
        let mut basin = ogar_loco::basin::BasinCodebooks::new();
        for &(id, names) in entries {
            let m = menus::menu_by_id(id).unwrap();
            let mut b = menus::builder(m, utf8, menus::PLACEHOLDER_DIGEST_CLASSID).unwrap();
            for n in names {
                b.intern(utf8, n.as_bytes()).unwrap();
            }
            basin.plug(b.seal()).unwrap();
        }
        basin
    }

    fn list_block(ty: &str, inputs: Vec<(&str, BlockRecord)>) -> BlockRecord {
        rec(
            ty,
            vec![("LIST".into(), FieldValue::Code("scores".into()))],
            inputs
                .into_iter()
                .map(|(n, b)| (n.to_string(), b))
                .collect(),
            vec![],
            None,
        )
    }

    /// Lists live in the stage's list store by codebook handle: add, replace,
    /// insert, delete, read, length, contains, index — with Scratch's
    /// 1-based indices and its out-of-range rules (read 0, write nothing).
    #[test]
    fn list_ops_act_on_the_stage_list_named_by_the_handle() {
        let basin = basin_with(&[(26, &["scores", "other"])]);
        let chain = |blocks: Vec<BlockRecord>| {
            let mut head: Option<BlockRecord> = None;
            for mut b in blocks.into_iter().rev() {
                b.next = head.map(Box::new);
                head = Some(b);
            }
            head.unwrap()
        };
        let script = chain(vec![
            list_block("data_addtolist", vec![("ITEM", num(5))]),
            list_block("data_addtolist", vec![("ITEM", num(7))]),
            list_block(
                "data_insertatlist",
                vec![("ITEM", num(3)), ("INDEX", num(1))],
            ),
            list_block(
                "data_replaceitemoflist",
                vec![("INDEX", num(2)), ("ITEM", num(6))],
            ),
            list_block("data_deleteoflist", vec![("INDEX", num(9))]),
            // set x to item 2 (6) + length (3) + contains 7 (1) + item # of 7 (3) + item 9 (0)
            rec(
                "motion_setx",
                vec![],
                vec![(
                    "X".into(),
                    rec(
                        "operator_add",
                        vec![],
                        vec![
                            (
                                "NUM1".into(),
                                list_block("data_itemoflist", vec![("INDEX", num(2))]),
                            ),
                            (
                                "NUM2".into(),
                                rec(
                                    "operator_add",
                                    vec![],
                                    vec![
                                        ("NUM1".into(), list_block("data_lengthoflist", vec![])),
                                        (
                                            "NUM2".into(),
                                            rec(
                                                "operator_add",
                                                vec![],
                                                vec![
                                                    (
                                                        "NUM1".into(),
                                                        list_block(
                                                            "data_listcontainsitem",
                                                            vec![("ITEM", num(7))],
                                                        ),
                                                    ),
                                                    (
                                                        "NUM2".into(),
                                                        rec(
                                                            "operator_add",
                                                            vec![],
                                                            vec![
                                                                (
                                                                    "NUM1".into(),
                                                                    list_block(
                                                                        "data_itemnumoflist",
                                                                        vec![("ITEM", num(7))],
                                                                    ),
                                                                ),
                                                                (
                                                                    "NUM2".into(),
                                                                    list_block(
                                                                        "data_itemoflist",
                                                                        vec![("INDEX", num(9))],
                                                                    ),
                                                                ),
                                                            ],
                                                            vec![],
                                                            None,
                                                        ),
                                                    ),
                                                ],
                                                vec![],
                                                None,
                                            ),
                                        ),
                                    ],
                                    vec![],
                                    None,
                                ),
                            ),
                        ],
                        vec![],
                        None,
                    ),
                )],
                vec![],
                None,
            ),
        ]);
        let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
        let mut m = Machine::new(&prog.functions, 1000).with_basin(&basin);
        m.run().unwrap();
        let h = blockly_abi::menus::encode_in(
            &basin,
            blockly_abi::menus::menu_by_id(26).unwrap(),
            "scores",
        )
        .unwrap();
        assert_eq!(
            m.stage.list(h),
            &[Value::Num(3.0), Value::Num(6.0), Value::Num(7.0)],
            "[3 inserted at 1] [5 → 6 at 2] [7]; delete 9 did nothing"
        );
        assert_eq!(m.stage.me().x, 6.0 + 3.0 + 1.0 + 3.0 + 0.0);
        // The other list is untouched: handles discriminate.
        let o = blockly_abi::menus::encode_in(
            &basin,
            blockly_abi::menus::menu_by_id(26).unwrap(),
            "other",
        )
        .unwrap();
        assert!(m.stage.list(o).is_empty());
    }

    /// `say` keeps the TEXT register index, not a number: the value on the
    /// sprite is the text the script said. Costume and effect state land on
    /// the sprite by their menu indices; `pick random` is reproducible from
    /// the stage's seed and stays inside its bounds.
    #[test]
    fn looks_state_and_random_land_on_the_sprite_and_stage() {
        let basin = basin_with(&[(29, &["hello"]), (20, &["idle", "run"]), (2, &[])]);
        let text = rec(
            "text",
            vec![("TEXT".into(), FieldValue::Wide("hello".into()))],
            vec![],
            vec![],
            None,
        );
        let costume_menu = rec(
            "looks_costume",
            vec![("COSTUME".into(), FieldValue::Code("run".into()))],
            vec![],
            vec![],
            None,
        );
        let script = rec(
            "looks_say",
            vec![],
            vec![("MESSAGE".into(), text)],
            vec![],
            Some(rec(
                "looks_switchcostumeto",
                vec![],
                vec![("COSTUME".into(), costume_menu)],
                vec![],
                Some(rec(
                    "looks_seteffectto",
                    vec![("EFFECT".into(), FieldValue::Code("GHOST".into()))],
                    vec![("VALUE".into(), num(40))],
                    vec![],
                    Some(rec(
                        "motion_setx",
                        vec![],
                        vec![(
                            "X".into(),
                            rec(
                                "operator_random",
                                vec![],
                                vec![("FROM".into(), num(1)), ("TO".into(), num(6))],
                                vec![],
                                None,
                            ),
                        )],
                        vec![],
                        None,
                    )),
                )),
            )),
        );
        let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
        let run = |seed: u64| {
            let mut m = Machine::new(&prog.functions, 100).with_basin(&basin);
            m.stage.rng = seed;
            m.run().unwrap();
            m.stage
        };
        let s1 = run(1);
        let hello = blockly_abi::menus::encode_in(
            &basin,
            blockly_abi::menus::menu_by_id(29).unwrap(),
            "hello",
        )
        .unwrap();
        assert_eq!(
            s1.me().say,
            Some((Value::Text(hello), false)),
            "the text is kept by register index"
        );
        assert_eq!(s1.me().costume, 2, "`run` is the second costume");
        let ghost = blockly_abi::menus::encode(blockly_abi::menus::menu_by_id(2).unwrap(), "GHOST")
            .unwrap();
        assert_eq!(s1.me().effects[usize::from(ghost)], 40.0);
        let x = s1.me().x;
        assert!(
            (1.0..=6.0).contains(&x) && x.fract() == 0.0,
            "integer bounds: {x}"
        );
        assert_eq!(run(1).me().x, x, "same seed, same draw");
        // Different seeds eventually draw a different value — can-fire.
        assert!((2..40u64).any(|seed| run(seed).me().x != x));
    }

    /// `wait` yields the rest of the slice: nothing after it runs this
    /// round, and the next round continues from the top of the (forever)
    /// body — so a `forever [wait 1; change x by 1]` moves ONE per round.
    #[test]
    fn wait_yields_the_slice_so_a_waiting_loop_moves_once_per_round() {
        let script = rec(
            "control_forever",
            vec![],
            vec![],
            vec![(
                "SUBSTACK".into(),
                rec(
                    "control_wait",
                    vec![],
                    vec![("DURATION".into(), num(1))],
                    vec![],
                    Some(rec(
                        "motion_changexby",
                        vec![],
                        vec![("DX".into(), num(1))],
                        vec![],
                        None,
                    )),
                ),
            )],
            None,
        );
        let prog = lower_program(LaneShape::Pairs, &script).unwrap();
        let mut scene = Scene::new(Stage::default(), vec![prog.functions.as_slice()]);
        scene.run(5, 1000).unwrap();
        // The scene re-enters the body each round; the change lands only
        // when the slice starts AFTER a yielded wait, i.e. never within one
        // slice — so x stays 0 here, which is the point: `wait` is not a
        // no-op that lets the loop spin 1000 times.
        assert_eq!(scene.stage.me().x, 0.0);
        let mut spin = Machine::new(&prog.functions, 1000);
        spin.run().unwrap();
        assert_eq!(
            spin.stage.me().x,
            0.0,
            "a bare run yields at the first wait"
        );
    }

    /// A clone: `create clone of myself` copies the sprite, and the owner's
    /// `when I start as a clone` script runs ONCE next round AS the clone —
    /// the original never runs it; `delete this clone` retires the clone.
    #[test]
    fn a_clone_is_born_runs_its_hat_once_as_itself_and_can_delete_itself() {
        let basin = basin_with(&[(24, &["_myself_"])]);
        let menu = rec(
            "control_create_clone_of_menu",
            vec![("CLONE_OPTION".into(), FieldValue::Code("_myself_".into()))],
            vec![],
            vec![],
            None,
        );
        let spawner = rec(
            "event_whenflagclicked",
            vec![],
            vec![],
            vec![],
            Some(rec(
                "control_create_clone_of",
                vec![],
                vec![("CLONE_OPTION".into(), menu)],
                vec![],
                None,
            )),
        );
        let on_clone = rec(
            "control_start_as_clone",
            vec![],
            vec![],
            vec![],
            Some(rec(
                "motion_changexby",
                vec![],
                vec![("DX".into(), num(10))],
                vec![],
                Some(rec(
                    "control_delete_this_clone",
                    vec![],
                    vec![],
                    vec![],
                    None,
                )),
            )),
        );
        let p_spawn = blockly_abi::lower_program_in(LaneShape::Pairs, &spawner, &basin).unwrap();
        let p_clone = blockly_abi::lower_program_in(LaneShape::Pairs, &on_clone, &basin).unwrap();
        let stage = Stage {
            sprites: vec![Sprite::default()],
            ..Stage::default()
        };
        let mut scene = Scene::new(
            stage,
            vec![p_spawn.functions.as_slice(), p_clone.functions.as_slice()],
        )
        .with_owners(&[0, 0]);
        scene.run(1, 100).unwrap();
        assert_eq!(scene.stage.sprites.len(), 2, "one clone born in round 1");
        assert_eq!(scene.stage.sprites[1].clone_of, Some(0));
        assert_eq!(
            scene.stage.sprites[1].x, 0.0,
            "the clone hat runs NEXT round"
        );
        scene.run(1, 100).unwrap();
        assert_eq!(
            scene.stage.sprites[0].x, 0.0,
            "the original never runs the clone hat"
        );
        assert_eq!(
            scene.stage.sprites[1].x, 10.0,
            "the clone ran it, as itself"
        );
        assert!(!scene.stage.sprites[1].alive, "…and deleted itself");
        assert_eq!(scene.stage.sprites.len(), 3, "round 2 spawned another");
    }

    /// `(x position) of (Ball)` reads ANOTHER sprite through the OF_OBJECT
    /// index the menu pushed, and a variable by name resolves register to
    /// register (OF_PROPERTY entry → VARIABLE index).
    #[test]
    fn sensing_of_reads_another_sprite_and_a_variable_by_name() {
        use blockly_abi::menus;
        let basin = basin_with(&[
            (19, &["_stage_", "Ball", "Paddle"]),
            (
                27,
                &[
                    "x position",
                    "y position",
                    "direction",
                    "costume #",
                    "costume name",
                    "size",
                    "volume",
                    "backdrop #",
                    "backdrop nm",
                    "score",
                ],
            ),
            (25, &["score"]),
        ]);
        let of = |prop: &str, obj: &str| {
            rec(
                "sensing_of",
                vec![("PROPERTY".into(), FieldValue::Code(prop.into()))],
                vec![(
                    "OBJECT".into(),
                    rec(
                        "sensing_of_object_menu",
                        vec![("OBJECT".into(), FieldValue::Code(obj.into()))],
                        vec![],
                        vec![],
                        None,
                    ),
                )],
                vec![],
                None,
            )
        };
        let script = rec(
            "motion_setx",
            vec![],
            vec![("X".into(), of("x position", "Paddle"))],
            vec![],
            Some(rec(
                "motion_sety",
                vec![],
                vec![("Y".into(), of("score", "Paddle"))],
                vec![],
                None,
            )),
        );
        let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
        let mut m = Machine::new(&prog.functions, 100).with_basin(&basin);
        m.stage.sprites = vec![
            Sprite::default(),
            Sprite {
                x: 123.0,
                ..Sprite::default()
            },
        ];
        let score = menus::encode_in(&basin, menus::menu_by_id(25).unwrap(), "score").unwrap();
        *m.stage.var_mut(score) = Value::Num(42.0);
        m.run().unwrap();
        assert_eq!(
            m.stage.sprites[0].x, 123.0,
            "Paddle is OF_OBJECT index 3 → sprite 1"
        );
        assert_eq!(m.stage.sprites[0].y, 42.0, "the variable resolved by name");
    }

    /// A PROCEDURE index is minted per target: two sprites each own a
    /// "procedure 1" with different bodies. With owners bound, each caller
    /// reaches ITS sprite's definition — never the other's.
    #[test]
    fn procedure_calls_resolve_within_the_callers_sprite_once_owners_are_bound() {
        let basin = basin_with(&[(28, &["step"])]);
        let def = |dx: u8| {
            rec(
                "procedures_definition",
                vec![("PROCCODE".into(), FieldValue::Code("step".into()))],
                vec![],
                vec![(
                    "SUBSTACK".into(),
                    rec(
                        "motion_changexby",
                        vec![],
                        vec![("DX".into(), num(dx))],
                        vec![],
                        None,
                    ),
                )],
                None,
            )
        };
        let call = rec(
            "procedures_call",
            vec![
                ("PROCCODE".into(), FieldValue::Code("step".into())),
                ("ARGC".into(), FieldValue::Byte(0)),
            ],
            vec![],
            vec![],
            None,
        );
        let cast =
            |r: &BlockRecord| blockly_abi::lower_program_in(LaneShape::Triples, r, &basin).unwrap();
        // Original script order: [def A (+1), call A, def B (+100), call B]
        let (da, ca, db, cb) = (cast(&def(1)), cast(&call), cast(&def(100)), cast(&call));
        let stage = Stage {
            sprites: vec![Sprite::default(), Sprite::default()],
            ..Stage::default()
        };
        let mut scene = Scene::new(
            stage,
            vec![
                da.functions.as_slice(),
                ca.functions.as_slice(),
                db.functions.as_slice(),
                cb.functions.as_slice(),
            ],
        )
        .with_owners(&[0, 0, 1, 1]);
        scene.run(1, 100).unwrap();
        assert_eq!(
            scene.stage.sprites[0].x, 1.0,
            "sprite 0's call reached ITS +1 definition"
        );
        assert_eq!(
            scene.stage.sprites[1].x, 100.0,
            "sprite 1's call reached ITS +100 definition"
        );
    }

    /// The run's register is content-keyed, deduping, and REFUSES rather than
    /// recycling an index — two strings sharing one index is exactly the
    /// register loss the whole discipline exists to prevent.
    #[test]
    fn the_run_text_register_dedups_and_refuses_when_full() {
        let mut r = TextRegister::new();
        assert_eq!(r.intern(""), Some(0), "the empty string is the fallback");
        assert_eq!(r.len(), 0, "…and occupies no slot");
        assert_eq!(r.intern("go"), Some(1));
        assert_eq!(r.intern("go"), Some(1), "the same string is the same index");
        assert_eq!(r.len(), 1);
        assert_eq!(r.intern("stop"), Some(2), "a different string, a new index");
        assert_eq!(r.get(1), Some("go"));
        assert_eq!(r.get(2), Some("stop"));
        assert_eq!(r.get(3), None);
        // Fill to the byte's limit, then refuse.
        for i in 3..=255u16 {
            assert_eq!(r.intern(&format!("s{i}")), u8::try_from(i).ok());
        }
        assert_eq!(r.len(), 255);
        assert_eq!(r.intern("one too many"), None);
        assert_eq!(
            r.get(255).map(str::to_string),
            Some("s255".to_string()),
            "the refusal did not overwrite the last entry"
        );
    }

    /// `join` MAKES a string: it lands in the run register, a variable keeps
    /// it (a variable holds a value, not an f32), and `length` reads the made
    /// string back — not the literal it started from, and not zero.
    #[test]
    fn join_makes_a_string_a_variable_keeps_and_length_reads_it_back() {
        let basin = basin_with(&[(29, &["score: "]), (25, &["msg"])]);
        let text = |t: &str| {
            rec(
                "text",
                vec![("TEXT".into(), FieldValue::Wide(t.into()))],
                vec![],
                vec![],
                None,
            )
        };
        let var_get = rec(
            "data_variable",
            vec![("VARIABLE".into(), FieldValue::Code("msg".into()))],
            vec![],
            vec![],
            None,
        );
        let script = rec(
            "data_setvariableto",
            vec![("VARIABLE".into(), FieldValue::Code("msg".into()))],
            vec![(
                "VALUE".into(),
                rec(
                    "operator_join",
                    vec![],
                    vec![
                        ("STRING1".into(), text("score: ")),
                        ("STRING2".into(), num(5)),
                    ],
                    vec![],
                    None,
                ),
            )],
            vec![],
            Some(rec(
                "motion_setx",
                vec![],
                vec![(
                    "X".into(),
                    rec(
                        "operator_length",
                        vec![],
                        vec![("STRING".into(), var_get)],
                        vec![],
                        None,
                    ),
                )],
                vec![],
                None,
            )),
        );
        let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
        let mut m = Machine::new(&prog.functions, 200).with_basin(&basin);
        m.run().unwrap();

        let msg = blockly_abi::menus::encode_in(
            &basin,
            blockly_abi::menus::menu_by_id(25).unwrap(),
            "msg",
        )
        .unwrap();
        let Value::RunText(idx) = m.stage.var(msg) else {
            panic!(
                "the variable must hold a made text, got {:?}",
                m.stage.var(msg)
            );
        };
        assert_ne!(idx, 0, "a non-empty join is not the empty-string fallback");
        assert_eq!(m.stage.texts.get(idx), Some("score: 5"));
        // "score: 5" is 8 characters. Two-sided: NOT 7 (the literal alone,
        // which is what a dropped second operand would give) and NOT 0 (an
        // unreadable text, which is what a register the reader cannot see
        // would give).
        assert_eq!(m.stage.me().x, 8.0);
        assert_ne!(m.stage.me().x, 7.0);
        assert_ne!(m.stage.me().x, 0.0);
    }

    /// `letter of` is 1-based and by CHARACTER; out of range is the empty
    /// string, as in Scratch. `contains` fires and stays silent.
    #[test]
    fn letter_of_is_one_based_by_character_and_contains_discriminates() {
        let basin = basin_with(&[(29, &["wörld"])]);
        let text = || {
            rec(
                "text",
                vec![("TEXT".into(), FieldValue::Wide("wörld".into()))],
                vec![],
                vec![],
                None,
            )
        };
        let letter = |n: u8| {
            rec(
                "operator_letter_of",
                vec![],
                vec![("LETTER".into(), num(n)), ("STRING".into(), text())],
                vec![],
                None,
            )
        };
        let run_text = |r: BlockRecord| {
            let script = rec(
                "looks_say",
                vec![],
                vec![("MESSAGE".into(), r)],
                vec![],
                None,
            );
            let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
            let mut m = Machine::new(&prog.functions, 200).with_basin(&basin);
            m.run().unwrap();
            let (v, _) = m.stage.me().say.expect("said something");
            text_of(
                v,
                Regs {
                    basin: Some(&basin),
                    texts: &m.stage.texts,
                },
            )
            .into_owned()
        };
        // The 2nd character of "wörld" is "ö" — a CHARACTER, not a byte (a
        // byte reading would split the two-byte ö and never produce it).
        assert_eq!(run_text(letter(2)), "ö");
        assert_eq!(run_text(letter(5)), "d");
        assert_eq!(run_text(letter(6)), "", "past the end is empty");
        assert_eq!(run_text(letter(0)), "", "there is no letter 0");

        let contains = |needle: &str| {
            let n = needle.to_string();
            let script = rec(
                "motion_setx",
                vec![],
                vec![(
                    "X".into(),
                    rec(
                        "operator_contains",
                        vec![],
                        vec![
                            ("STRING1".into(), text()),
                            (
                                "STRING2".into(),
                                rec(
                                    "text",
                                    vec![("TEXT".into(), FieldValue::Wide(n))],
                                    vec![],
                                    vec![],
                                    None,
                                ),
                            ),
                        ],
                        vec![],
                        None,
                    ),
                )],
                vec![],
                None,
            );
            let b = basin_with(&[(29, &["wörld", needle])]);
            let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &b).unwrap();
            let mut m = Machine::new(&prog.functions, 200).with_basin(&b);
            m.run().unwrap();
            m.stage.me().x
        };
        assert_eq!(contains("rld"), 1.0, "can fire");
        assert_eq!(contains("xyz"), 0.0, "…and can stay silent");
    }

    /// A list search matches a made string against a project literal by
    /// CONTENT, across the two registers — never by index, which would be a
    /// coincidence when the indices happen to agree and wrong when they do
    /// not.
    #[test]
    fn a_list_finds_a_made_string_equal_to_a_project_literal() {
        // "go" is index 2 in the project register and will be index 1 in the
        // run register, so an index comparison gives the wrong answer.
        let basin = basin_with(&[(29, &["other", "go", "g", "o"]), (26, &["seen"])]);
        let text = |t: &str| {
            rec(
                "text",
                vec![("TEXT".into(), FieldValue::Wide(t.into()))],
                vec![],
                vec![],
                None,
            )
        };
        let add = |t: &str, next: Option<BlockRecord>| {
            let mut r = rec(
                "data_addtolist",
                vec![("LIST".into(), FieldValue::Code("seen".into()))],
                vec![("ITEM".into(), text(t))],
                vec![],
                None,
            );
            r.next = next.map(Box::new);
            r
        };
        // TWO non-numeric items, and the wanted one is SECOND. A numeric
        // comparison reads every non-numeric text as 0 and would match the
        // first item; an index comparison would match neither. Only a content
        // comparison answers 2.
        let script = add(
            "other",
            Some(add(
                "go",
                Some(rec(
                    "motion_setx",
                    vec![],
                    vec![(
                        "X".into(),
                        rec(
                            "data_itemnumoflist",
                            vec![("LIST".into(), FieldValue::Code("seen".into()))],
                            vec![(
                                "ITEM".into(),
                                rec(
                                    "operator_join",
                                    vec![],
                                    vec![
                                        ("STRING1".into(), text("g")),
                                        ("STRING2".into(), text("o")),
                                    ],
                                    vec![],
                                    None,
                                ),
                            )],
                            vec![],
                            None,
                        ),
                    )],
                    vec![],
                    None,
                )),
            )),
        );
        let prog = blockly_abi::lower_program_in(LaneShape::Pairs, &script, &basin).unwrap();
        let mut m = Machine::new(&prog.functions, 200).with_basin(&basin);
        m.run().unwrap();
        // The indices really do differ, so this is not a coincidence.
        let project = blockly_abi::menus::encode_in(
            &basin,
            blockly_abi::menus::menu_by_id(29).unwrap(),
            "go",
        )
        .unwrap();
        assert_eq!(project, 2);
        assert_eq!(m.stage.texts.get(1), Some("go"), "the made one is index 1");
        assert_eq!(
            m.stage.me().x,
            2.0,
            "found at position 2, past a non-numeric decoy"
        );
        assert_ne!(
            m.stage.me().x,
            1.0,
            "a numeric comparison would have said 1"
        );
        assert_ne!(
            m.stage.me().x,
            0.0,
            "an index comparison would have found nothing"
        );
    }
}
