//! The **text projection** — `A = B + C` rendered from, and parsed back into,
//! the call stream.
//!
//! # Storage does not change (the W0 ruling)
//!
//! This is a third skin over the same rows, exactly like the block surface and
//! the PowerAutomate surface. Text is **never** the storage format: the ABI is
//! the call stream, the projection renders it, and parsing writes calls back.
//! Nothing here serializes.
//!
//! # Why this needs an arity table, and why the table refuses
//!
//! The call stream is post-order — `(NUMBER:5) (NUMBER:3) (ADD:0)`. Rendering
//! infix requires knowing that `ADD` consumes two operands and `NEG` consumes
//! one; the stream itself does not say. Arity is a codebook property, so
//! [`arity`] is a table, and it covers the expression core rather than all 95
//! palette slots.
//!
//! A function with no arity entry is **refused**, not guessed. A wrong arity
//! does not produce a wrong-looking render — it desynchronizes the whole
//! stack and silently reattributes every operand after it, which is a far
//! worse failure than a missing feature. Statement-level functions (`IF`,
//! `REPEAT`, `PROC_DEF`) are deliberately absent: they nest by reference, so
//! projecting them is the *statement* projection, a separate surface.
//!
//! # What round-trips, and what explicitly must NOT
//!
//! - **Round-trips:** the call stream, byte for byte. `body → text → body` is
//!   the identity on anything this table covers.
//! - **Must NOT round-trip:** whitespace, redundant parentheses, and every
//!   trace of geometry. Two texts differing only in spacing must produce the
//!   *same* body, and the render is canonical rather than reproducing what was
//!   typed. That is asserted directly — if a position or a spacing choice ever
//!   survived the trip, geometry would have leaked into the ABI, which is the
//!   thesis this whole arc rests on.
//!
//! Note the asymmetry that makes the second bullet a real test rather than a
//! platitude: parsing is many-to-one, so the round-trip that matters is
//! `body → text → body`, never `text → body → text`.

use ogar_loco::{Call, FnIndex, FunctionBody, LaneShape};

use crate::raise_calls;

/// How many operands a function consumes from the stack.
///
/// `None` means the shared core does not cover this function — it is refused
/// rather than assigned a plausible number, because a wrong arity
/// desynchronizes the stack and silently reattributes every later operand.
///
/// Post-flip this is a pure delegation: the table that used to live here
/// (and its control-flow half, which lived in the deleted local `flow`
/// module) is defined ONCE in [`ogar_loco::vocabulary::shared_core`], where
/// every sibling vocabulary reads the same bytes. Keeping a local copy would
/// be the drift the shared core exists to make impossible.
#[must_use]
pub fn arity(f: FnIndex) -> Option<u8> {
    ogar_loco::vocabulary::shared_core::stack_arity(f)
}

/// The infix spelling and binding power of a binary operator, if it has one.
///
/// Higher binds tighter. Functions absent from this table render in call form
/// (`sqrt(x)`), which is why [`arity`] and this table are separate: every
/// infix operator has an arity, but not every binary function is infix.
#[must_use]
fn infix(f: FnIndex) -> Option<(&'static str, u8)> {
    Some(match f {
        FnIndex::OR => ("or", 1),
        FnIndex::AND => ("and", 2),
        FnIndex::EQ => ("==", 3),
        FnIndex::NEQ => ("!=", 3),
        FnIndex::LT => ("<", 3),
        FnIndex::LTE => ("<=", 3),
        FnIndex::GT => (">", 3),
        FnIndex::GTE => (">=", 3),
        FnIndex::ADD => ("+", 4),
        FnIndex::SUB => ("-", 4),
        FnIndex::MUL => ("*", 5),
        FnIndex::DIV => ("/", 5),
        FnIndex::MOD => ("%", 5),
        FnIndex::POW => ("^", 6),
        _ => return None,
    })
}

/// The call-form name of a function the projection covers.
#[must_use]
fn call_name(f: FnIndex) -> Option<&'static str> {
    Some(match f {
        FnIndex::NOT => "not",
        FnIndex::NEG => "neg",
        FnIndex::ABS => "abs",
        FnIndex::SQRT => "sqrt",
        FnIndex::LN => "ln",
        FnIndex::LOG10 => "log10",
        FnIndex::EXP_E => "exp",
        FnIndex::EXP_10 => "exp10",
        FnIndex::SIN => "sin",
        FnIndex::COS => "cos",
        FnIndex::TAN => "tan",
        FnIndex::ASIN => "asin",
        FnIndex::ACOS => "acos",
        FnIndex::ATAN => "atan",
        FnIndex::ROUND => "round",
        FnIndex::FLOOR => "floor",
        FnIndex::CEIL => "ceil",
        FnIndex::LENGTH => "len",
        FnIndex::JOIN => "join",
        FnIndex::TRUE => "true",
        FnIndex::FALSE => "false",
        FnIndex::NULL => "null",
        _ => return None,
    })
}

/// Why a call stream could not be projected, or a text could not be parsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// The function has no arity entry — the projection does not cover it.
    /// Refused rather than guessed: a wrong arity desynchronizes the stack.
    Uncovered {
        /// The palette byte.
        function: u8,
    },
    /// The call stream is not a single well-formed expression: a function
    /// wanted more operands than the stack held, or operands were left over.
    Unbalanced {
        /// How many values remained on the stack (1 is well-formed).
        remaining: usize,
    },
    /// The text could not be parsed at this byte offset.
    Syntax {
        /// Byte offset into the input.
        at: usize,
        /// What was expected.
        expected: &'static str,
    },
    /// A numeric literal outside the one-byte immediate range. Refused rather
    /// than truncated; the constant pool is the answer, not a smaller number.
    LiteralOutOfRange {
        /// The literal as written.
        text: String,
    },
    /// The parsed expression overran the shape's call budget.
    Overflow,
}

impl core::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ProjectionError::Uncovered { function } => write!(
                f,
                "function {function:#04x} has no arity entry; the text projection \
                 covers the expression core only"
            ),
            ProjectionError::Unbalanced { remaining } => write!(
                f,
                "the call stream is not one expression: {remaining} values left \
                 on the stack"
            ),
            ProjectionError::Syntax { at, expected } => {
                write!(f, "expected {expected} at byte {at}")
            }
            ProjectionError::LiteralOutOfRange { text } => write!(
                f,
                "literal `{text}` does not fit one immediate byte; that is what \
                 the constant pool is for"
            ),
            ProjectionError::Overflow => {
                write!(f, "the expression overran the shape's call budget")
            }
        }
    }
}

impl core::error::Error for ProjectionError {}

// ── Render: calls → text ────────────────────────────────────────────────────

/// Render a body as one canonical expression.
///
/// Canonical, not reproduced: the output is a function of the call stream
/// alone, so whatever spacing or redundant parentheses a user typed are gone.
/// Parentheses appear only where precedence requires them.
///
/// # Errors
///
/// [`ProjectionError::Uncovered`] for a function outside the expression core;
/// [`ProjectionError::Unbalanced`] if the stream is not one expression.
pub fn render_text(body: &FunctionBody) -> Result<String, ProjectionError> {
    // Each stack entry carries the binding power it was rendered at, so a
    // parent knows whether it must parenthesize. Rendering without that would
    // either drop needed parens or paren everything.
    let mut stack: Vec<(String, u8)> = Vec::new();

    for call in raise_calls(body) {
        let f = call.function;
        let n = arity(f).ok_or(ProjectionError::Uncovered { function: f.0 })?;
        if stack.len() < usize::from(n) {
            return Err(ProjectionError::Unbalanced {
                remaining: stack.len(),
            });
        }
        let args: Vec<(String, u8)> = stack.split_off(stack.len() - usize::from(n));

        let rendered = if let Some((op, power)) = infix(f) {
            let (lhs, lp) = &args[0];
            let (rhs, rp) = &args[1];
            // Left-associative: an equal-power right operand needs parens
            // (`a - (b - c)`), an equal-power left operand does not.
            let l = paren_if(lhs, *lp < power);
            let r = paren_if(rhs, *rp <= power);
            (format!("{l} {op} {r}"), power)
        } else if f == FnIndex::NUMBER {
            (call.values[0].to_string(), u8::MAX)
        } else if f == FnIndex::VAR_GET {
            (format!("var{}", call.values[0]), u8::MAX)
        } else if f == FnIndex::CONSTANT {
            (format!("const{}", call.values[0]), u8::MAX)
        } else if f == FnIndex::TEXT {
            (format!("text{}", call.values[0]), u8::MAX)
        } else {
            let name = call_name(f).ok_or(ProjectionError::Uncovered { function: f.0 })?;
            if n == 0 {
                (name.to_owned(), u8::MAX)
            } else {
                let inner: Vec<&str> = args.iter().map(|(s, _)| s.as_str()).collect();
                (format!("{name}({})", inner.join(", ")), u8::MAX)
            }
        };
        stack.push(rendered);
    }

    if stack.len() == 1 {
        Ok(stack.pop().expect("length checked").0)
    } else {
        Err(ProjectionError::Unbalanced {
            remaining: stack.len(),
        })
    }
}

fn paren_if(s: &str, cond: bool) -> String {
    if cond { format!("({s})") } else { s.to_owned() }
}

// ── Parse: text → calls ─────────────────────────────────────────────────────

/// Parse an expression back into a call stream.
///
/// Emits post-order, which is the same stack discipline the block cast emits —
/// so a text edit and a block edit produce the same bytes.
///
/// # Errors
///
/// [`ProjectionError::Syntax`], [`ProjectionError::LiteralOutOfRange`], or
/// [`ProjectionError::Overflow`].
pub fn parse_text(text: &str, shape: LaneShape) -> Result<FunctionBody, ProjectionError> {
    let mut p = Parser {
        src: text.as_bytes(),
        at: 0,
        calls: Vec::new(),
    };
    p.expr(0)?;
    p.skip_ws();
    if p.at != p.src.len() {
        return Err(ProjectionError::Syntax {
            at: p.at,
            expected: "end of expression",
        });
    }
    FunctionBody::from_calls(shape, &p.calls).map_err(|_| ProjectionError::Overflow)
}

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
    calls: Vec<Call>,
}

impl Parser<'_> {
    fn skip_ws(&mut self) {
        while self.at < self.src.len() && self.src[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn peek_op(&mut self) -> Option<(FnIndex, &'static str, u8)> {
        self.skip_ws();
        let rest = &self.src[self.at..];
        // Longest match first, or `<=` would lex as `<` then fail on `=`.
        for f in [
            FnIndex::LTE,
            FnIndex::GTE,
            FnIndex::EQ,
            FnIndex::NEQ,
            FnIndex::AND,
            FnIndex::OR,
            FnIndex::LT,
            FnIndex::GT,
            FnIndex::ADD,
            FnIndex::SUB,
            FnIndex::MUL,
            FnIndex::DIV,
            FnIndex::MOD,
            FnIndex::POW,
        ] {
            let (op, power) = infix(f).expect("table entry");
            if rest.starts_with(op.as_bytes()) {
                return Some((f, op, power));
            }
        }
        None
    }

    /// Precedence climbing. `min_power` is the lowest binding power this call
    /// may consume.
    fn expr(&mut self, min_power: u8) -> Result<(), ProjectionError> {
        self.atom()?;
        while let Some((f, op, power)) = self.peek_op() {
            if power < min_power {
                break;
            }
            self.at += op.len();
            // Left-associative: the right side must bind strictly tighter.
            self.expr(power + 1)?;
            self.calls.push(Call::new(f));
        }
        Ok(())
    }

    fn atom(&mut self) -> Result<(), ProjectionError> {
        self.skip_ws();
        if self.at >= self.src.len() {
            return Err(ProjectionError::Syntax {
                at: self.at,
                expected: "an operand",
            });
        }
        if self.src[self.at] == b'(' {
            self.at += 1;
            self.expr(0)?;
            self.skip_ws();
            if self.at >= self.src.len() || self.src[self.at] != b')' {
                return Err(ProjectionError::Syntax {
                    at: self.at,
                    expected: "`)`",
                });
            }
            self.at += 1;
            return Ok(());
        }
        if self.src[self.at].is_ascii_digit() {
            return self.number();
        }
        self.name()
    }

    fn number(&mut self) -> Result<(), ProjectionError> {
        let start = self.at;
        while self.at < self.src.len() && self.src[self.at].is_ascii_digit() {
            self.at += 1;
        }
        let text = core::str::from_utf8(&self.src[start..self.at]).expect("ascii digits");
        let v: u8 = text
            .parse()
            .map_err(|_| ProjectionError::LiteralOutOfRange {
                text: text.to_owned(),
            })?;
        self.calls.push(Call::with_value(FnIndex::NUMBER, v));
        Ok(())
    }

    fn name(&mut self) -> Result<(), ProjectionError> {
        let start = self.at;
        while self.at < self.src.len()
            && (self.src[self.at].is_ascii_alphanumeric() || self.src[self.at] == b'_')
        {
            self.at += 1;
        }
        if start == self.at {
            return Err(ProjectionError::Syntax {
                at: self.at,
                expected: "an operand",
            });
        }
        let word = core::str::from_utf8(&self.src[start..self.at]).expect("ascii word");

        // `varN` / `constN` / `textN` — an indexed leaf.
        for (prefix, f) in [
            ("var", FnIndex::VAR_GET),
            ("const", FnIndex::CONSTANT),
            ("text", FnIndex::TEXT),
        ] {
            if let Some(idx) = word.strip_prefix(prefix)
                && !idx.is_empty()
                && idx.bytes().all(|b| b.is_ascii_digit())
            {
                let v: u8 = idx
                    .parse()
                    .map_err(|_| ProjectionError::LiteralOutOfRange {
                        text: word.to_owned(),
                    })?;
                self.calls.push(Call::with_value(f, v));
                return Ok(());
            }
        }

        let f = named(word).ok_or(ProjectionError::Syntax {
            at: start,
            expected: "a known operand or function",
        })?;
        let n = arity(f).ok_or(ProjectionError::Syntax {
            at: start,
            expected: "a covered function",
        })?;
        if n == 0 {
            self.calls.push(Call::new(f));
            return Ok(());
        }
        self.skip_ws();
        if self.at >= self.src.len() || self.src[self.at] != b'(' {
            return Err(ProjectionError::Syntax {
                at: self.at,
                expected: "`(` after a function name",
            });
        }
        self.at += 1;
        for i in 0..n {
            if i > 0 {
                self.skip_ws();
                if self.at >= self.src.len() || self.src[self.at] != b',' {
                    return Err(ProjectionError::Syntax {
                        at: self.at,
                        expected: "`,` between arguments",
                    });
                }
                self.at += 1;
            }
            self.expr(0)?;
        }
        self.skip_ws();
        if self.at >= self.src.len() || self.src[self.at] != b')' {
            return Err(ProjectionError::Syntax {
                at: self.at,
                expected: "`)` closing a call",
            });
        }
        self.at += 1;
        self.calls.push(Call::new(f));
        Ok(())
    }
}

/// The inverse of [`call_name`] — text word to function.
fn named(word: &str) -> Option<FnIndex> {
    Some(match word {
        "not" => FnIndex::NOT,
        "neg" => FnIndex::NEG,
        "abs" => FnIndex::ABS,
        "sqrt" => FnIndex::SQRT,
        "ln" => FnIndex::LN,
        "log10" => FnIndex::LOG10,
        "exp" => FnIndex::EXP_E,
        "exp10" => FnIndex::EXP_10,
        "sin" => FnIndex::SIN,
        "cos" => FnIndex::COS,
        "tan" => FnIndex::TAN,
        "asin" => FnIndex::ASIN,
        "acos" => FnIndex::ACOS,
        "atan" => FnIndex::ATAN,
        "round" => FnIndex::ROUND,
        "floor" => FnIndex::FLOOR,
        "ceil" => FnIndex::CEIL,
        "len" => FnIndex::LENGTH,
        "join" => FnIndex::JOIN,
        "true" => FnIndex::TRUE,
        "false" => FnIndex::FALSE,
        "null" => FnIndex::NULL,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BlockRecord, FieldValue, lower_script};

    const S: LaneShape = LaneShape::Pairs;

    #[test]
    fn the_block_cast_and_the_text_parser_agree_byte_for_byte() {
        // The claim that makes this a projection rather than a second format:
        // `5 + 3` typed as text and `5 + 3` built as blocks are the SAME bytes.
        let blocks = BlockRecord::leaf("math_arithmetic", "root")
            .with_field("OP", FieldValue::Code("ADD".into()))
            .with_input(
                "A",
                BlockRecord::leaf("math_number", "a").with_field("NUM", FieldValue::Byte(5)),
            )
            .with_input(
                "B",
                BlockRecord::leaf("math_number", "b").with_field("NUM", FieldValue::Byte(3)),
            );
        let from_blocks = lower_script(S, &blocks).unwrap();
        let from_text = parse_text("5 + 3", S).unwrap();
        assert_eq!(from_blocks.as_body_bytes(), from_text.as_body_bytes());
        // Anti-vacuity: a DIFFERENT program must not also match, or "the bytes
        // are equal" would be true of any two bodies.
        let other = parse_text("5 + 4", S).unwrap();
        assert_ne!(from_blocks.as_body_bytes(), other.as_body_bytes());
    }

    #[test]
    fn a_body_round_trips_through_text_byte_for_byte() {
        // body → text → body is the identity. Note the direction: parsing is
        // many-to-one, so `text → body → text` is NOT the round-trip that
        // matters and asserting it would be the weaker claim.
        for src in [
            "5 + 3",
            "1 + 2 * 3",
            "(1 + 2) * 3",
            "10 - 4 - 3",
            "10 - (4 - 3)",
            "2 ^ 3 + 1",
            "sqrt(16) + abs(3)",
            "join(text1, text2)",
            "var1 * var2 + 7",
            "1 < 2 and 3 >= 2",
            "not(1 == 2)",
            "true or false",
        ] {
            let body = parse_text(src, S).unwrap();
            let text = render_text(&body).unwrap();
            let again = parse_text(&text, S).unwrap();
            assert_eq!(
                body.as_body_bytes(),
                again.as_body_bytes(),
                "`{src}` did not survive the round trip (rendered `{text}`)"
            );
        }
    }

    #[test]
    fn whitespace_and_redundant_parens_explicitly_do_not_round_trip() {
        // The W5 falsifier's second half, and the one that would catch geometry
        // leaking into the ABI. Spacing is presentation; if it survived, the
        // ABI would be carrying a layout decision.
        let canonical = parse_text("1 + 2 * 3", S).unwrap();
        for noisy in [
            "1+2*3",
            "  1   +  2*3 ",
            "1 + (2 * 3)",
            "(1) + ((2) * (3))",
            "\t1\n+\t2 * 3\n",
        ] {
            let body = parse_text(noisy, S).unwrap();
            assert_eq!(
                canonical.as_body_bytes(),
                body.as_body_bytes(),
                "`{noisy}` produced different bytes — spacing reached the ABI"
            );
        }
        // …and the render is canonical rather than reproducing any of them.
        assert_eq!(render_text(&canonical).unwrap(), "1 + 2 * 3");
        // Two-sided: parens that are NOT redundant must survive, or "spacing
        // does not matter" would be indistinguishable from "parens are ignored".
        let grouped = parse_text("(1 + 2) * 3", S).unwrap();
        assert_ne!(canonical.as_body_bytes(), grouped.as_body_bytes());
        assert_eq!(render_text(&grouped).unwrap(), "(1 + 2) * 3");
    }

    #[test]
    fn precedence_and_associativity_are_real() {
        // Anti-vacuity for the round-trip test: a parser that ignored
        // precedence would still round-trip its own output. These pin the
        // actual stack order.
        let ops = |s: &str| {
            raise_calls(&parse_text(s, S).unwrap())
                .iter()
                .map(|c| c.function.0)
                .collect::<Vec<_>>()
        };
        // `1 + 2 * 3` must multiply FIRST — MUL before ADD in the stream.
        let mul_first = ops("1 + 2 * 3");
        let pos = |v: &[u8], b: u8| v.iter().position(|x| *x == b).unwrap();
        assert!(pos(&mul_first, FnIndex::MUL.0) < pos(&mul_first, FnIndex::ADD.0));
        // …and the grouped form must invert exactly that.
        let add_first = ops("(1 + 2) * 3");
        assert!(pos(&add_first, FnIndex::ADD.0) < pos(&add_first, FnIndex::MUL.0));

        // Left-associativity: `10 - 4 - 3` is `(10 - 4) - 3`, so the operand
        // order differs from the right-associated reading.
        assert_ne!(
            parse_text("10 - 4 - 3", S).unwrap().as_body_bytes(),
            parse_text("10 - (4 - 3)", S).unwrap().as_body_bytes()
        );
        assert_eq!(
            parse_text("10 - 4 - 3", S).unwrap().as_body_bytes(),
            parse_text("(10 - 4) - 3", S).unwrap().as_body_bytes()
        );
    }

    #[test]
    fn an_uncovered_function_is_refused_not_rendered_with_a_guessed_arity() {
        // A wrong arity does not produce a wrong-looking render — it
        // desynchronizes the stack and silently reattributes every later
        // operand. So the table refuses.
        // RE-PINNED. This used to name `IF`, which gained a stack arity when
        // control flow was covered (now in the shared core's tables) — so
        // asserting it is uncovered became wrong. The PROPERTY did not
        // change, so it is re-pinned against functions that are still
        // genuinely outside the table rather than weakened.
        assert_eq!(arity(FnIndex::PROC_DEF), None);
        assert_eq!(arity(FnIndex::WAIT), None);
        let body = FunctionBody::from_calls(S, &[Call::new(FnIndex::PROC_DEF)]).unwrap();
        assert_eq!(
            render_text(&body),
            Err(ProjectionError::Uncovered {
                function: FnIndex::PROC_DEF.0
            })
        );
        // …and `IF` really is covered now, so this test cannot quietly become
        // a claim about a function that no longer exists in the gap.
        assert_eq!(arity(FnIndex::IF), Some(1));
        // Two-sided: the covered core still renders, so the refusal is targeted
        // rather than a blanket failure.
        assert!(arity(FnIndex::ADD).is_some());
        assert!(render_text(&parse_text("1 + 1", S).unwrap()).is_ok());
    }

    #[test]
    fn an_unbalanced_stream_is_refused() {
        // `ADD` with nothing to add. Caught rather than rendered as something
        // plausible.
        let starved = FunctionBody::from_calls(S, &[Call::new(FnIndex::ADD)]).unwrap();
        assert_eq!(
            render_text(&starved),
            Err(ProjectionError::Unbalanced { remaining: 0 })
        );
        // Leftovers are equally wrong: two values, no operator.
        let extra = FunctionBody::from_calls(
            S,
            &[
                Call::with_value(FnIndex::NUMBER, 1),
                Call::with_value(FnIndex::NUMBER, 2),
            ],
        )
        .unwrap();
        assert_eq!(
            render_text(&extra),
            Err(ProjectionError::Unbalanced { remaining: 2 })
        );
    }

    #[test]
    fn a_literal_past_one_byte_is_refused_not_truncated() {
        // 255 fits, 256 does not. Truncating would silently change the program.
        assert!(parse_text("255", S).is_ok());
        assert_eq!(
            parse_text("256", S),
            Err(ProjectionError::LiteralOutOfRange {
                text: "256".to_owned()
            })
        );
    }

    #[test]
    fn syntax_errors_name_their_offset_and_do_not_half_parse() {
        for (src, _) in [("1 +", ""), ("(1 + 2", ""), ("1 $ 2", ""), ("sqrt 4", "")] {
            assert!(
                matches!(parse_text(src, S), Err(ProjectionError::Syntax { .. })),
                "`{src}` must be a syntax error"
            );
        }
        // Silence twin: the near-miss VALID forms of each must parse, or the
        // test above would pass for a parser that rejects everything.
        for src in ["1 + 2", "(1 + 2)", "1 * 2", "sqrt(4)"] {
            assert!(parse_text(src, S).is_ok(), "`{src}` must parse");
        }
    }

    #[test]
    fn an_expression_past_the_call_budget_overflows_rather_than_truncating() {
        // Same remedy as everywhere else in this ABI: split, never widen.
        let long = core::iter::repeat_n("1", LaneShape::Pairs.calls_per_function())
            .collect::<Vec<_>>()
            .join(" + ");
        assert_eq!(parse_text(&long, S), Err(ProjectionError::Overflow));
        // Two-sided: one term shorter than the limit still fits, so the bound
        // is the real budget and not an always-fail.
        let ok = core::iter::repeat_n("1", LaneShape::Pairs.calls_per_function() / 2)
            .collect::<Vec<_>>()
            .join(" + ");
        assert!(parse_text(&ok, S).is_ok());
    }
}
