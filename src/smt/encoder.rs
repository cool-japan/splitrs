//! Recursive `syn` -> QF_BV term encoder for the verified-refactoring oracle.
//!
//! Only a deliberately small *pure* fixed-width-integer fragment is supported.
//! Every unsupported construct returns `Err(reason)` so the caller can surface
//! a precise "cannot prove" explanation rather than silently producing an
//! unsound encoding.  A companion non-mutating [`probe_expr`]/[`probe_block`]
//! pair walks the exact same accept/reject set without building terms, for a
//! future purity detector (Phase 4).

use std::collections::HashMap;
use std::str::FromStr;

use num_bigint::BigInt;
use oxiz::{TermId, TermManager};

use super::types::{int_type_of, sign_extend, zero_extend, IntType};

/// The sort of an encoded value: a fixed-width integer or a boolean.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ValSort {
    /// A bit-vector-sorted integer value of the given Rust integer type.
    Int(IntType),
    /// A boolean-sorted value.
    Bool,
}

/// An encoded value: its term id paired with the sort it carries.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Encoded {
    /// The OxiZ term.
    pub term: TermId,
    /// The value's sort.
    pub sort: ValSort,
}

/// Recursive encoder over a single `&mut TermManager`.
///
/// `env` maps a (possibly SSA-renamed) source variable name to its term and
/// sort.  Function parameters are seeded by the caller; `let` bindings add
/// fresh SSA entries keyed by source name so shadowing/re-let is handled
/// soundly without aliasing distinct bindings.
pub(crate) struct Encoder<'tm> {
    tm: &'tm mut TermManager,
    env: HashMap<String, Encoded>,
    fresh: usize,
}

impl<'tm> Encoder<'tm> {
    /// Create an encoder seeded with the given parameter environment.
    pub(crate) fn new(tm: &'tm mut TermManager, env: HashMap<String, Encoded>) -> Self {
        Self { tm, env, fresh: 0 }
    }

    /// Encode a function/block body that is a single expression.
    pub(crate) fn encode_expr(
        &mut self,
        expr: &syn::Expr,
        expected: Option<IntType>,
    ) -> Result<Encoded, String> {
        match expr {
            syn::Expr::Paren(p) => self.encode_expr(&p.expr, expected),
            syn::Expr::Group(g) => self.encode_expr(&g.expr, expected),
            syn::Expr::Lit(lit) => self.encode_lit(&lit.lit, expected),
            syn::Expr::Path(path) => self.encode_path(path),
            syn::Expr::Binary(bin) => self.encode_binary(bin, expected),
            syn::Expr::Unary(un) => self.encode_unary(un, expected),
            syn::Expr::If(if_expr) => self.encode_if(if_expr, expected),
            syn::Expr::Cast(cast) => self.encode_cast(cast),
            syn::Expr::Block(block) => self.encode_block(&block.block, expected),
            // Explicitly rejected constructs with tailored reasons.
            syn::Expr::Call(_) | syn::Expr::MethodCall(_) => {
                Err("function/method calls not modeled".to_string())
            }
            syn::Expr::Reference(_) => Err("references (&) not modeled".to_string()),
            syn::Expr::Assign(_) => Err("assignment/mutation not modeled".to_string()),
            syn::Expr::While(_) | syn::Expr::ForLoop(_) | syn::Expr::Loop(_) => {
                Err("loops not modeled (bounded unroll future)".to_string())
            }
            syn::Expr::Index(_) => Err("indexing not modeled".to_string()),
            syn::Expr::Struct(_) | syn::Expr::Field(_) => {
                Err("struct construction/field access not modeled".to_string())
            }
            syn::Expr::Match(_) => Err("match not modeled (future)".to_string()),
            syn::Expr::Closure(_) => Err("closures not modeled".to_string()),
            syn::Expr::Try(_) => Err("`?` operator not modeled".to_string()),
            syn::Expr::Async(_) | syn::Expr::Await(_) => Err("async not modeled".to_string()),
            syn::Expr::Return(_) => {
                Err("early return not modeled (only a trailing return is allowed)".to_string())
            }
            other => Err(format!("unsupported expression: {}", expr_kind(other))),
        }
    }

    /// Encode an integer/boolean literal. Width for integers flows from
    /// `expected` or an explicit suffix (e.g. `5u32`).
    fn encode_lit(&mut self, lit: &syn::Lit, expected: Option<IntType>) -> Result<Encoded, String> {
        match lit {
            syn::Lit::Int(int_lit) => {
                // A literal suffix (`5u32`) pins the type regardless of context.
                let suffix = int_lit.suffix();
                let ty = if suffix.is_empty() {
                    expected.ok_or_else(|| {
                        "integer literal width is ambiguous (no suffix or context type)".to_string()
                    })?
                } else {
                    super::types::int_type_from_name(suffix)
                        .ok_or_else(|| format!("unsupported integer literal suffix `{suffix}`"))?
                };
                let digits = int_lit.base10_digits();
                let value = BigInt::from_str(digits)
                    .map_err(|_| format!("invalid integer literal `{digits}`"))?;
                let term = self.tm.mk_bitvec(value, ty.width);
                Ok(Encoded {
                    term,
                    sort: ValSort::Int(ty),
                })
            }
            syn::Lit::Bool(bool_lit) => {
                let term = self.tm.mk_bool(bool_lit.value);
                Ok(Encoded {
                    term,
                    sort: ValSort::Bool,
                })
            }
            _ => Err("only integer and boolean literals are modeled".to_string()),
        }
    }

    /// Encode a variable reference by looking it up in the environment.
    fn encode_path(&mut self, path: &syn::ExprPath) -> Result<Encoded, String> {
        let ident = path
            .path
            .get_ident()
            .ok_or_else(|| "only simple variable paths are modeled".to_string())?;
        let name = ident.to_string();
        self.env
            .get(&name)
            .copied()
            .ok_or_else(|| format!("unknown variable `{name}`"))
    }

    /// Encode a binary operation.
    fn encode_binary(
        &mut self,
        bin: &syn::ExprBinary,
        expected: Option<IntType>,
    ) -> Result<Encoded, String> {
        use syn::BinOp;

        // Logical && / || operate on booleans.
        if matches!(bin.op, BinOp::And(_) | BinOp::Or(_)) {
            let lhs = self.encode_expr(&bin.left, None)?;
            let rhs = self.encode_expr(&bin.right, None)?;
            if lhs.sort != ValSort::Bool || rhs.sort != ValSort::Bool {
                return Err("&&/|| require boolean operands".to_string());
            }
            let term = match bin.op {
                BinOp::And(_) => self.tm.mk_and([lhs.term, rhs.term]),
                BinOp::Or(_) => self.tm.mk_or([lhs.term, rhs.term]),
                _ => unreachable!("guarded by matches! above"),
            };
            return Ok(Encoded {
                term,
                sort: ValSort::Bool,
            });
        }

        // Division/remainder carry panic side-conditions we do not model.
        if matches!(bin.op, BinOp::Div(_) | BinOp::Rem(_)) {
            return Err("division/remainder not modeled (panic side-conditions)".to_string());
        }

        // For arithmetic/bitwise/comparison ops, propagate the integer type
        // between the two operands. The expected type seeds whichever side is a
        // bare literal; an identifier or typed sub-expression resolves it.
        let left = self.encode_expr(&bin.left, expected)?;
        let left_ty = require_int(&left, "left operand of binary operator")?;
        // The right operand must share the left's integer type.
        let right = self.encode_expr(&bin.right, Some(left_ty))?;
        let right_ty = require_int(&right, "right operand of binary operator")?;
        if left_ty != right_ty {
            return Err(format!(
                "binary operator operand type mismatch: {}-bit {} vs {}-bit {}",
                left_ty.width,
                signedness(left_ty),
                right_ty.width,
                signedness(right_ty),
            ));
        }

        let signed = left_ty.signed;
        let (l, r) = (left.term, right.term);
        let int_result = ValSort::Int(left_ty);

        match bin.op {
            BinOp::Add(_) => {
                let term = self.tm.mk_bv_add(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::Sub(_) => {
                let term = self.tm.mk_bv_sub(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::Mul(_) => {
                let term = self.tm.mk_bv_mul(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::BitAnd(_) => {
                let term = self.tm.mk_bv_and(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::BitOr(_) => {
                let term = self.tm.mk_bv_or(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::BitXor(_) => {
                let term = self.tm.mk_bv_xor(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::Shl(_) => {
                let term = self.tm.mk_bv_shl(l, r);
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::Shr(_) => {
                // Signed → arithmetic shift (sign-replicating); unsigned →
                // logical shift. This choice is load-bearing for soundness.
                let term = if signed {
                    self.tm.mk_bv_ashr(l, r)
                } else {
                    self.tm.mk_bv_lshr(l, r)
                };
                Ok(Encoded {
                    term,
                    sort: int_result,
                })
            }
            BinOp::Eq(_) => {
                let t = self.tm.mk_eq(l, r);
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            BinOp::Ne(_) => {
                let eq = self.tm.mk_eq(l, r);
                let t = self.tm.mk_not(eq);
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            BinOp::Lt(_) => {
                let t = if signed {
                    self.tm.mk_bv_slt(l, r)
                } else {
                    self.tm.mk_bv_ult(l, r)
                };
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            BinOp::Le(_) => {
                let t = if signed {
                    self.tm.mk_bv_sle(l, r)
                } else {
                    self.tm.mk_bv_ule(l, r)
                };
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            BinOp::Gt(_) => {
                // a > b  ≡  b < a (OxiZ has no ugt/sgt; swap operands).
                let t = if signed {
                    self.tm.mk_bv_slt(r, l)
                } else {
                    self.tm.mk_bv_ult(r, l)
                };
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            BinOp::Ge(_) => {
                // a >= b  ≡  b <= a.
                let t = if signed {
                    self.tm.mk_bv_sle(r, l)
                } else {
                    self.tm.mk_bv_ule(r, l)
                };
                Ok(Encoded {
                    term: t,
                    sort: ValSort::Bool,
                })
            }
            _ => Err("unsupported binary operator".to_string()),
        }
    }

    /// Encode a unary operation (`-`, `!`).
    fn encode_unary(
        &mut self,
        un: &syn::ExprUnary,
        expected: Option<IntType>,
    ) -> Result<Encoded, String> {
        use syn::UnOp;
        match un.op {
            UnOp::Neg(_) => {
                let inner = self.encode_expr(&un.expr, expected)?;
                let ty = require_int(&inner, "operand of unary negation")?;
                if !ty.signed {
                    return Err("unary negation of an unsigned integer not modeled".to_string());
                }
                let term = self.tm.mk_bv_neg(inner.term);
                Ok(Encoded {
                    term,
                    sort: ValSort::Int(ty),
                })
            }
            UnOp::Not(_) => {
                let inner = self.encode_expr(&un.expr, expected)?;
                match inner.sort {
                    ValSort::Bool => {
                        let term = self.tm.mk_not(inner.term);
                        Ok(Encoded {
                            term,
                            sort: ValSort::Bool,
                        })
                    }
                    ValSort::Int(ty) => {
                        // Bitwise NOT on an integer.
                        let term = self.tm.mk_bv_not(inner.term);
                        Ok(Encoded {
                            term,
                            sort: ValSort::Int(ty),
                        })
                    }
                }
            }
            _ => Err("unsupported unary operator".to_string()),
        }
    }

    /// Encode an `if cond { a } else { b }` value. The `else` arm is mandatory
    /// (an `if` without `else` has no value to encode).
    fn encode_if(
        &mut self,
        if_expr: &syn::ExprIf,
        expected: Option<IntType>,
    ) -> Result<Encoded, String> {
        let cond = self.encode_expr(&if_expr.cond, None)?;
        if cond.sort != ValSort::Bool {
            return Err("`if` condition must be boolean".to_string());
        }
        let else_branch = if_expr
            .else_branch
            .as_ref()
            .ok_or_else(|| "`if` without `else` cannot be used as a value".to_string())?;

        let then_val = self.encode_block(&if_expr.then_branch, expected)?;
        // The `else` arm is either another `if` (else-if chain) or a block.
        let else_expr = &else_branch.1;
        let else_val = self.encode_expr(else_expr, expected.or(int_sort_of(&then_val)))?;

        if then_val.sort != else_val.sort {
            return Err("`if`/`else` arms have mismatched types".to_string());
        }
        let term = self.tm.mk_ite(cond.term, then_val.term, else_val.term);
        Ok(Encoded {
            term,
            sort: then_val.sort,
        })
    }

    /// Encode an `as` cast between integer types.
    fn encode_cast(&mut self, cast: &syn::ExprCast) -> Result<Encoded, String> {
        let target = int_type_of(&cast.ty)
            .ok_or_else(|| "cast target is not a fixed-width integer".to_string())?;
        let inner = self.encode_expr(&cast.expr, None)?;
        let source = require_int(&inner, "cast operand")?;

        if source.width == target.width {
            // Same-width reinterpretation (signed <-> unsigned): identical bits,
            // only the sort changes.
            return Ok(Encoded {
                term: inner.term,
                sort: ValSort::Int(target),
            });
        }
        if target.width > source.width {
            // Widening: zero-extend unsigned source, sign-extend signed source.
            let term = if source.signed {
                sign_extend(self.tm, inner.term, source.width, target.width)
            } else {
                zero_extend(self.tm, inner.term, source.width, target.width)
            };
            Ok(Encoded {
                term,
                sort: ValSort::Int(target),
            })
        } else {
            // Narrowing: truncate to the low `target.width` bits.
            let term = self.tm.mk_bv_extract(target.width - 1, 0, inner.term);
            Ok(Encoded {
                term,
                sort: ValSort::Int(target),
            })
        }
    }

    /// Encode a block: process `let` statements, then the tail expression.
    pub(crate) fn encode_block(
        &mut self,
        block: &syn::Block,
        expected: Option<IntType>,
    ) -> Result<Encoded, String> {
        let stmts = &block.stmts;
        if stmts.is_empty() {
            return Err("empty block has no value".to_string());
        }
        let (last, leading) = stmts
            .split_last()
            .ok_or_else(|| "empty block has no value".to_string())?;

        // Every statement before the last must be a `let` binding.
        for stmt in leading {
            self.encode_local_stmt(stmt)?;
        }

        // The final statement provides the block's value.
        match last {
            syn::Stmt::Expr(expr, _semi) => {
                // A trailing `return e;` is allowed and behaves like a tail expr.
                if let syn::Expr::Return(ret) = expr {
                    let inner = ret.expr.as_ref().ok_or_else(|| {
                        "`return;` without a value cannot produce a block value".to_string()
                    })?;
                    return self.encode_expr(inner, expected);
                }
                self.encode_expr(expr, expected)
            }
            syn::Stmt::Local(_) => {
                Err("block must end in an expression, not a `let` binding".to_string())
            }
            syn::Stmt::Item(_) => Err("item definitions inside a body are not modeled".to_string()),
            syn::Stmt::Macro(_) => Err("macro invocations are not modeled".to_string()),
        }
    }

    /// Process a single `let` statement, inserting a fresh SSA binding.
    fn encode_local_stmt(&mut self, stmt: &syn::Stmt) -> Result<(), String> {
        let syn::Stmt::Local(local) = stmt else {
            // Earlier `return` shows up as an Expr statement here.
            if let syn::Stmt::Expr(syn::Expr::Return(_), _) = stmt {
                return Err("early return not modeled".to_string());
            }
            return Err("only `let` bindings are allowed before the block tail".to_string());
        };

        // The pattern must be a plain identifier (optionally with a type
        // annotation). Reject tuples, refs, and `mut`-through-ref patterns.
        let (name, annotated) = plain_let_target(local)?;
        let init = local
            .init
            .as_ref()
            .ok_or_else(|| format!("`let {name}` without an initializer is not modeled"))?;
        if init.diverge.is_some() {
            return Err("`let ... else` is not modeled".to_string());
        }

        let value = self.encode_expr(&init.expr, annotated)?;
        // If the binding was annotated with an integer type, enforce it.
        if let Some(ann) = annotated {
            match value.sort {
                ValSort::Int(actual) if actual == ann => {}
                ValSort::Int(actual) => {
                    return Err(format!(
                        "`let {name}: {}{}` initializer has type {}{}",
                        signedness(ann),
                        ann.width,
                        signedness(actual),
                        actual.width
                    ));
                }
                ValSort::Bool => {
                    return Err(format!(
                        "`let {name}: {}{}` initialized with a boolean",
                        signedness(ann),
                        ann.width
                    ));
                }
            }
        }

        // Insert under a fresh SSA key so a later re-`let`/shadow of the same
        // source name does not alias the earlier binding.
        self.fresh += 1;
        self.env.insert(name, value);
        Ok(())
    }
}

/// Extract the bound name and optional integer annotation from a `let`,
/// rejecting any non-trivial pattern.
fn plain_let_target(local: &syn::Local) -> Result<(String, Option<IntType>), String> {
    match &local.pat {
        syn::Pat::Ident(pat_ident) => {
            // `ref`/sub-patterns are not modeled; a plain `mut x` binding is
            // fine (the value is still SSA — we never mutate it).
            if pat_ident.by_ref.is_some() || pat_ident.subpat.is_some() {
                return Err("`ref`/sub-pattern `let` bindings are not modeled".to_string());
            }
            Ok((pat_ident.ident.to_string(), None))
        }
        syn::Pat::Type(pat_type) => {
            let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
                return Err("only plain identifier `let` patterns are modeled".to_string());
            };
            if pat_ident.by_ref.is_some() || pat_ident.subpat.is_some() {
                return Err("`ref`/sub-pattern `let` bindings are not modeled".to_string());
            }
            let ty = int_type_of(&pat_type.ty)
                .ok_or_else(|| "`let` type annotation is not a fixed-width integer".to_string())?;
            Ok((pat_ident.ident.to_string(), Some(ty)))
        }
        _ => Err("only plain identifier `let` patterns are modeled".to_string()),
    }
}

/// Require that an encoded value is an integer, returning its [`IntType`].
fn require_int(enc: &Encoded, ctx: &str) -> Result<IntType, String> {
    match enc.sort {
        ValSort::Int(ty) => Ok(ty),
        ValSort::Bool => Err(format!("{ctx} must be an integer, found a boolean")),
    }
}

/// Return the integer type of an encoded value, if it is an integer.
fn int_sort_of(enc: &Encoded) -> Option<IntType> {
    match enc.sort {
        ValSort::Int(ty) => Some(ty),
        ValSort::Bool => None,
    }
}

/// Human-readable signedness word.
fn signedness(ty: IntType) -> &'static str {
    if ty.signed {
        "i"
    } else {
        "u"
    }
}

/// A short label for an unexpected expression kind (for error messages).
fn expr_kind(expr: &syn::Expr) -> &'static str {
    match expr {
        syn::Expr::Array(_) => "array literal",
        syn::Expr::Range(_) => "range",
        syn::Expr::Repeat(_) => "array repeat",
        syn::Expr::Tuple(_) => "tuple",
        syn::Expr::Macro(_) => "macro invocation",
        syn::Expr::Let(_) => "let-expression",
        _ => "expression",
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Non-mutating purity probe (Phase 4 reuse).
//
// Mirrors the encoder's accept/reject set exactly, but without a TermManager,
// returning Ok(()) when the fragment is fully supported and Err(reason)
// otherwise.  Used to test whether an extracted body is a pure modellable
// function before attempting an equivalence proof.
// ───────────────────────────────────────────────────────────────────────────

/// Probe an expression for membership in the supported pure fragment.
pub(crate) fn probe_expr(expr: &syn::Expr) -> Result<(), String> {
    match expr {
        syn::Expr::Paren(p) => probe_expr(&p.expr),
        syn::Expr::Group(g) => probe_expr(&g.expr),
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Int(_) | syn::Lit::Bool(_) => Ok(()),
            _ => Err("only integer and boolean literals are modeled".to_string()),
        },
        syn::Expr::Path(path) => {
            if path.path.get_ident().is_some() {
                Ok(())
            } else {
                Err("only simple variable paths are modeled".to_string())
            }
        }
        syn::Expr::Binary(bin) => {
            use syn::BinOp;
            if matches!(bin.op, BinOp::Div(_) | BinOp::Rem(_)) {
                return Err("division/remainder not modeled (panic side-conditions)".to_string());
            }
            probe_expr(&bin.left)?;
            probe_expr(&bin.right)
        }
        syn::Expr::Unary(un) => match un.op {
            syn::UnOp::Neg(_) | syn::UnOp::Not(_) => probe_expr(&un.expr),
            _ => Err("unsupported unary operator".to_string()),
        },
        syn::Expr::If(if_expr) => {
            probe_expr(&if_expr.cond)?;
            let else_branch = if_expr
                .else_branch
                .as_ref()
                .ok_or_else(|| "`if` without `else` cannot be used as a value".to_string())?;
            probe_block(&if_expr.then_branch)?;
            probe_expr(&else_branch.1)
        }
        syn::Expr::Cast(cast) => {
            if int_type_of(&cast.ty).is_none() {
                return Err("cast target is not a fixed-width integer".to_string());
            }
            probe_expr(&cast.expr)
        }
        syn::Expr::Block(block) => probe_block(&block.block),
        syn::Expr::Return(ret) => match ret.expr.as_ref() {
            Some(inner) => probe_expr(inner),
            None => Err("`return;` without a value is not modeled".to_string()),
        },
        syn::Expr::Call(_) | syn::Expr::MethodCall(_) => {
            Err("function/method calls not modeled".to_string())
        }
        syn::Expr::Reference(_) => Err("references (&) not modeled".to_string()),
        syn::Expr::Assign(_) => Err("assignment/mutation not modeled".to_string()),
        syn::Expr::While(_) | syn::Expr::ForLoop(_) | syn::Expr::Loop(_) => {
            Err("loops not modeled (bounded unroll future)".to_string())
        }
        syn::Expr::Index(_) => Err("indexing not modeled".to_string()),
        syn::Expr::Struct(_) | syn::Expr::Field(_) => {
            Err("struct construction/field access not modeled".to_string())
        }
        syn::Expr::Match(_) => Err("match not modeled (future)".to_string()),
        syn::Expr::Closure(_) => Err("closures not modeled".to_string()),
        syn::Expr::Try(_) => Err("`?` operator not modeled".to_string()),
        syn::Expr::Async(_) | syn::Expr::Await(_) => Err("async not modeled".to_string()),
        other => Err(format!("unsupported expression: {}", expr_kind(other))),
    }
}

/// Probe a block for membership in the supported pure fragment.
pub(crate) fn probe_block(block: &syn::Block) -> Result<(), String> {
    let (last, leading) = block
        .stmts
        .split_last()
        .ok_or_else(|| "empty block has no value".to_string())?;
    for stmt in leading {
        match stmt {
            syn::Stmt::Local(local) => {
                let (_name, _ann) = plain_let_target(local)?;
                let init = local
                    .init
                    .as_ref()
                    .ok_or_else(|| "`let` without an initializer is not modeled".to_string())?;
                if init.diverge.is_some() {
                    return Err("`let ... else` is not modeled".to_string());
                }
                probe_expr(&init.expr)?;
            }
            syn::Stmt::Expr(syn::Expr::Return(_), _) => {
                return Err("early return not modeled".to_string());
            }
            _ => {
                return Err("only `let` bindings are allowed before the block tail".to_string());
            }
        }
    }
    match last {
        syn::Stmt::Expr(expr, _) => {
            if let syn::Expr::Return(ret) = expr {
                return match ret.expr.as_ref() {
                    Some(inner) => probe_expr(inner),
                    None => Err("`return;` without a value is not modeled".to_string()),
                };
            }
            probe_expr(expr)
        }
        syn::Stmt::Local(_) => {
            Err("block must end in an expression, not a `let` binding".to_string())
        }
        syn::Stmt::Item(_) => Err("item definitions inside a body are not modeled".to_string()),
        syn::Stmt::Macro(_) => Err("macro invocations are not modeled".to_string()),
    }
}
