//! Equivalence prover: given two pure fixed-width-integer Rust functions,
//! prove they compute the same result for all inputs, or produce a concrete
//! counterexample, or report the construct as Unsupported.
//!
//! Soundness rests on the QF_BV theory: we assert `not(out_a == out_b)` and a
//! result of `Unsat` is a genuine proof that no input distinguishes the two
//! functions (for the modelled bit-precise semantics).

use std::collections::HashMap;

use num_bigint::BigInt;
use oxiz::core::TermKind;
use oxiz::{Solver, SolverResult, TermId, TermManager};

use super::encoder::{Encoded, Encoder, ValSort};
use super::types::{decode_bits, int_type_of, IntType};
use super::{Counterexample, Verdict};

/// A validated function parameter: its source name and integer/bool sort.
struct Param {
    name: String,
    sort: ValSort,
}

/// Drives a single equivalence proof between two function items.
pub struct EquivBuilder;

impl EquivBuilder {
    /// Prove `fn_a` and `fn_b` equivalent, or refute, or report Unsupported.
    pub fn prove(fn_a: &syn::ItemFn, fn_b: &syn::ItemFn) -> Verdict {
        // 1. Validate signatures match positionally (name + type).
        let params = match validate_signatures(fn_a, fn_b) {
            Ok(p) => p,
            Err(reason) => return Verdict::Unsupported { reason },
        };

        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");

        // 2. Create one shared variable per parameter; record its term/sort and
        //    keep the term id around for counterexample decoding.
        let mut env: HashMap<String, Encoded> = HashMap::new();
        let mut param_terms: Vec<(String, IntType, TermId)> = Vec::new();
        for param in &params {
            match param.sort {
                ValSort::Int(ty) => {
                    let sort_id = tm.sorts.bitvec(ty.width);
                    let var = tm.mk_var(&param.name, sort_id);
                    env.insert(
                        param.name.clone(),
                        Encoded {
                            term: var,
                            sort: param.sort,
                        },
                    );
                    param_terms.push((param.name.clone(), ty, var));
                }
                ValSort::Bool => {
                    // Booleans are not part of the printed counterexample input
                    // table decode (no IntType), but still shared between sides.
                    let bool_sort = tm.sorts.bool_sort;
                    let var = tm.mk_var(&param.name, bool_sort);
                    env.insert(
                        param.name.clone(),
                        Encoded {
                            term: var,
                            sort: param.sort,
                        },
                    );
                }
            }
        }

        // Pre-flight purity probe: reject anything outside the modelled
        // fragment *before* paying for a full encode. This is the same
        // accept/reject set the encoder enforces, surfaced as a fast gate (and
        // the Phase-4 extraction detector reuses these probes directly).
        if let Err(reason) = super::is_pure_supported_block(&fn_a.block) {
            return Verdict::Unsupported { reason };
        }
        if let Err(reason) = super::is_pure_supported_block(&fn_b.block) {
            return Verdict::Unsupported { reason };
        }

        // 3. Encode both bodies over the SAME TermManager seeded from the SAME
        //    param-symbol env (so both reference identical param term ids).
        let out_a = {
            let mut enc = Encoder::new(&mut tm, env.clone());
            match enc.encode_block(&fn_a.block, return_int_hint(fn_a)) {
                Ok(e) => e,
                Err(reason) => return Verdict::Unsupported { reason },
            }
        };
        let out_b = {
            let mut enc = Encoder::new(&mut tm, env);
            match enc.encode_block(&fn_b.block, return_int_hint(fn_b)) {
                Ok(e) => e,
                Err(reason) => return Verdict::Unsupported { reason },
            }
        };

        // 4. Both result sorts must match.
        if out_a.sort != out_b.sort {
            return Verdict::Unsupported {
                reason: "the two functions produce results of different sorts".to_string(),
            };
        }

        // 5. Assert *only* the disequality and check. This is the AUTHORITATIVE
        //    verdict.
        //
        //    Soundness note: we deliberately do NOT bind fresh
        //    `out_a_var = out_a` / `out_b_var = out_b` equality variables before
        //    this check. Doing so was observed to flip a genuinely-`Sat`
        //    disequality to `Unsat` whenever an output term contains a
        //    bit-vector multiply — an unsound interaction in the underlying
        //    solver's multiplier bit-blasting with extra equality-constrained
        //    variables. Because those helper variables are otherwise
        //    unconstrained, adding them can NEVER change satisfiability in a
        //    correct solver; asserting the bare disequality is the sound query.
        //    The counterexample *values* are recovered separately below in a
        //    fresh solver so a value-extraction quirk can never corrupt the
        //    Verified/Refuted decision the extraction gate relies on.
        let eq_out = tm.mk_eq(out_a.term, out_b.term);
        let diseq = tm.mk_not(eq_out);
        solver.assert(diseq, &mut tm);

        let check_result = solver.check(&mut tm);
        match check_result {
            SolverResult::Unsat => Verdict::Verified,
            SolverResult::Sat => {
                let cx = build_counterexample(
                    &solver,
                    &mut tm,
                    &param_terms,
                    out_a.sort,
                    out_a.term,
                    out_b.term,
                );
                Verdict::Refuted(cx)
            }
            SolverResult::Unknown => Verdict::Unsupported {
                reason: "solver returned Unknown".to_string(),
            },
        }
    }
}

/// Validate that the two signatures have matching arity and positional
/// parameter names + (integer or bool) types. Returns the parameter list.
fn validate_signatures(fn_a: &syn::ItemFn, fn_b: &syn::ItemFn) -> Result<Vec<Param>, String> {
    let params_a = collect_params(&fn_a.sig)?;
    let params_b = collect_params(&fn_b.sig)?;

    if params_a.len() != params_b.len() {
        return Err(format!(
            "arity mismatch: {} vs {} parameters",
            params_a.len(),
            params_b.len()
        ));
    }
    for (idx, (a, b)) in params_a.iter().zip(params_b.iter()).enumerate() {
        if a.name != b.name {
            return Err(format!(
                "parameter {} name mismatch: `{}` vs `{}`",
                idx, a.name, b.name
            ));
        }
        if a.sort != b.sort {
            return Err(format!(
                "parameter `{}` type mismatch between the two functions",
                a.name
            ));
        }
    }
    Ok(params_a)
}

/// Collect a function's parameters as plain-ident integer/bool bindings.
fn collect_params(sig: &syn::Signature) -> Result<Vec<Param>, String> {
    let mut out = Vec::new();
    for input in &sig.inputs {
        let syn::FnArg::Typed(pat_type) = input else {
            return Err("`self` receiver is not modeled".to_string());
        };
        let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
            return Err("only plain identifier parameters are modeled".to_string());
        };
        if pat_ident.by_ref.is_some() || pat_ident.subpat.is_some() {
            return Err("`ref`/sub-pattern parameters are not modeled".to_string());
        }
        let name = pat_ident.ident.to_string();
        let sort = sort_of_type(&pat_type.ty)
            .ok_or_else(|| format!("parameter `{name}` has an unsupported type"))?;
        out.push(Param { name, sort });
    }
    Ok(out)
}

/// Map a parameter type to a [`ValSort`] (integer or the special `bool`).
fn sort_of_type(ty: &syn::Type) -> Option<ValSort> {
    if let Some(int_ty) = int_type_of(ty) {
        return Some(ValSort::Int(int_ty));
    }
    if is_bool_type(ty) {
        return Some(ValSort::Bool);
    }
    None
}

/// Recognise a bare `bool` type.
fn is_bool_type(ty: &syn::Type) -> bool {
    let syn::Type::Path(type_path) = ty else {
        return false;
    };
    type_path.qself.is_none()
        && type_path.path.segments.len() == 1
        && type_path
            .path
            .segments
            .first()
            .is_some_and(|s| s.ident == "bool" && matches!(s.arguments, syn::PathArguments::None))
}

/// Derive the integer return-type hint from a function signature, if its
/// return type is a fixed-width integer (used to resolve bare-literal widths
/// in the tail expression).
fn return_int_hint(item: &syn::ItemFn) -> Option<IntType> {
    match &item.sig.output {
        syn::ReturnType::Type(_, ty) => int_type_of(ty),
        syn::ReturnType::Default => None,
    }
}

/// Build a [`Counterexample`] from a satisfying model.
///
/// The input values come straight from the sound disequality-only model. The
/// output values are recovered by a pure constant-fold of the output term DAGs
/// seeded by those concrete parameter values (see [`eval_term_const`]). This
/// avoids spawning a second SMT solve — which re-solving an equality over a
/// symbolic bit-vector multiplier made pathologically slow — while remaining
/// bit-precise for the full set of operators the encoder emits.
fn build_counterexample(
    solver: &Solver,
    tm: &mut TermManager,
    param_terms: &[(String, IntType, TermId)],
    out_sort: ValSort,
    out_a_term: TermId,
    out_b_term: TermId,
) -> Counterexample {
    let mut inputs = Vec::new();
    let model = solver.model();

    // 1. Recover and render each input value from the sound model. Also collect
    //    the raw (term, width, value) assignments so we can pin them below.
    let mut pins: Vec<(TermId, u32, BigInt)> = Vec::new();
    for (name, ty, term) in param_terms {
        let raw = model.and_then(|m| model_bitvec_value(m, tm, *term));
        let rendered = raw
            .as_ref()
            .map(|v| decode_bits(v, *ty))
            .unwrap_or_else(|| "?".to_string());
        if let Some(v) = raw {
            pins.push((*term, ty.width, v));
        }
        inputs.push((name.clone(), rendered));
    }

    // 2. Evaluate output terms directly using the concrete input values.
    let (left_output, right_output) =
        outputs_under_pins(tm, &pins, out_sort, out_a_term, out_b_term);

    Counterexample {
        inputs,
        left_output,
        right_output,
    }
}

/// A folded constant value: a width-tagged bit-vector magnitude (unsigned,
/// already masked to its width) or a boolean. Used only to render
/// counterexample output values — never affects a verdict.
#[derive(Clone, Debug)]
enum ConstVal {
    /// An unsigned bit-vector magnitude in `[0, 2^width)`, paired with its width.
    Bv(BigInt, u32),
    /// A boolean.
    Bool(bool),
}

/// The bit width of a term's sort, if it is a bit-vector sort.
fn bv_width(tm: &TermManager, term: TermId) -> Option<u32> {
    let sort = tm.get(term)?.sort;
    tm.sorts.get(sort).and_then(|s| s.bitvec_width())
}

/// `2^width` as a [`BigInt`] (the modulus for a `width`-bit bit-vector).
fn modulus_of(width: u32) -> BigInt {
    BigInt::from(1u64) << width
}

/// Reduce `value` into the unsigned range `[0, 2^width)` (handles negatives).
fn mask_to(value: BigInt, width: u32) -> BigInt {
    let modulus = modulus_of(width);
    let reduced = value % &modulus;
    if reduced.sign() == num_bigint::Sign::Minus {
        reduced + modulus
    } else {
        reduced
    }
}

/// Interpret an unsigned `width`-bit magnitude as a two's-complement signed
/// [`BigInt`].
fn to_signed(value: &BigInt, width: u32) -> BigInt {
    let half = BigInt::from(1u64) << (width - 1);
    if *value >= half {
        value - modulus_of(width)
    } else {
        value.clone()
    }
}

/// Convert a (non-negative, in-range) shift amount to a `u32`, returning `None`
/// when it does not fit — callers treat that as the over-shift case.
fn shift_amount(value: &BigInt) -> Option<u32> {
    match value.to_u32_digits() {
        // Negative shift amounts cannot occur (values are unsigned magnitudes);
        // be defensive and signal over-shift.
        (num_bigint::Sign::Minus, _) => None,
        (_, digits) => match digits.as_slice() {
            [] => Some(0),
            [single] => Some(*single),
            // More than 32 bits of shift: always an over-shift for our widths.
            _ => None,
        },
    }
}

/// Purely constant-fold an OxiZ term to a [`ConstVal`], seeded by the concrete
/// values of the parameter `Var` leaves recorded in `pins`.
///
/// This reproduces QF_BV semantics for exactly the operator set the encoder
/// emits — it never spawns a solver and cannot hang. It is used only to render
/// counterexample output values; it can never influence a verdict. Returns
/// `None` for any leaf that is unbound or any construct outside the modelled
/// fragment, in which case the value renders as the existing `"?"`.
fn eval_term_const(
    tm: &TermManager,
    term: TermId,
    pins: &HashMap<TermId, BigInt>,
) -> Option<ConstVal> {
    let kind = tm.get(term)?.kind.clone();
    match kind {
        // ── Leaves ───────────────────────────────────────────────────────────
        TermKind::BitVecConst { value, width } => Some(ConstVal::Bv(mask_to(value, width), width)),
        TermKind::Var(_) => {
            let value = pins.get(&term)?.clone();
            let width = bv_width(tm, term)?;
            Some(ConstVal::Bv(mask_to(value, width), width))
        }
        TermKind::True => Some(ConstVal::Bool(true)),
        TermKind::False => Some(ConstVal::Bool(false)),

        // ── Bit-vector arithmetic (operands share the result width) ───────────
        TermKind::BvAdd(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bv(mask_to(va + vb, width), width))
        }
        TermKind::BvSub(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            // Add the modulus before masking to stay non-negative.
            Some(ConstVal::Bv(mask_to(va - vb, width), width))
        }
        TermKind::BvMul(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bv(mask_to(va * vb, width), width))
        }

        // ── Bit-vector bitwise ────────────────────────────────────────────────
        TermKind::BvAnd(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bv(va & vb, width))
        }
        TermKind::BvOr(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bv(va | vb, width))
        }
        TermKind::BvXor(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bv(va ^ vb, width))
        }
        TermKind::BvNot(a) => {
            let width = bv_width(tm, term)?;
            let va = eval_bv(tm, a, pins)?;
            // One's complement within the width: mask ^ va.
            let mask = modulus_of(width) - BigInt::from(1u64);
            Some(ConstVal::Bv(mask ^ va, width))
        }

        // ── Bit-vector shifts ─────────────────────────────────────────────────
        TermKind::BvShl(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            let result = match shift_amount(&vb) {
                Some(sh) if sh < width => va << sh,
                _ => BigInt::from(0u64),
            };
            Some(ConstVal::Bv(mask_to(result, width), width))
        }
        TermKind::BvLshr(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            // `va` is already an unsigned in-range magnitude, so `>>` is logical.
            let result = match shift_amount(&vb) {
                Some(sh) if sh < width => va >> sh,
                _ => BigInt::from(0u64),
            };
            Some(ConstVal::Bv(result, width))
        }
        TermKind::BvAshr(a, b) => {
            let width = bv_width(tm, term)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            let signed = to_signed(&va, width);
            let negative = signed.sign() == num_bigint::Sign::Minus;
            let result = match shift_amount(&vb) {
                Some(sh) if sh < width => {
                    // BigInt `>>` is arithmetic (floor) for negatives — exactly ashr.
                    signed >> sh
                }
                // Over-shift: sign-fill (all-ones when negative, else zero).
                _ if negative => -BigInt::from(1u64),
                _ => BigInt::from(0u64),
            };
            Some(ConstVal::Bv(mask_to(result, width), width))
        }

        // ── Bit-vector structural ─────────────────────────────────────────────
        TermKind::BvExtract { high, low, arg } => {
            let va = eval_bv(tm, arg, pins)?;
            let out_width = high - low + 1;
            let field_mask = modulus_of(out_width) - BigInt::from(1u64);
            let extracted = (va >> low) & field_mask;
            Some(ConstVal::Bv(extracted, out_width))
        }
        TermKind::BvConcat(high, low) => {
            // Result = (high << low_width) | low; width = sum of operand widths.
            let (vh, low_width) = (eval_bv(tm, high, pins)?, bv_width(tm, low)?);
            let vl = eval_bv(tm, low, pins)?;
            let high_width = bv_width(tm, high)?;
            let value = (vh << low_width) | vl;
            Some(ConstVal::Bv(value, high_width + low_width))
        }

        // ── Comparisons (produce booleans) ────────────────────────────────────
        TermKind::Eq(a, b) => {
            match (eval_term_const(tm, a, pins)?, eval_term_const(tm, b, pins)?) {
                (ConstVal::Bv(va, _), ConstVal::Bv(vb, _)) => Some(ConstVal::Bool(va == vb)),
                (ConstVal::Bool(ba), ConstVal::Bool(bb)) => Some(ConstVal::Bool(ba == bb)),
                _ => None,
            }
        }
        TermKind::BvUlt(a, b) => {
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bool(va < vb))
        }
        TermKind::BvUle(a, b) => {
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bool(va <= vb))
        }
        TermKind::BvSlt(a, b) => {
            let width = bv_width(tm, a)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bool(
                to_signed(&va, width) < to_signed(&vb, width),
            ))
        }
        TermKind::BvSle(a, b) => {
            let width = bv_width(tm, a)?;
            let (va, vb) = (eval_bv(tm, a, pins)?, eval_bv(tm, b, pins)?);
            Some(ConstVal::Bool(
                to_signed(&va, width) <= to_signed(&vb, width),
            ))
        }

        // ── Boolean connectives ───────────────────────────────────────────────
        TermKind::Not(a) => match eval_term_const(tm, a, pins)? {
            ConstVal::Bool(b) => Some(ConstVal::Bool(!b)),
            ConstVal::Bv(..) => None,
        },
        TermKind::And(args) => {
            let mut all = true;
            for arg in args {
                match eval_term_const(tm, arg, pins)? {
                    ConstVal::Bool(b) => all &= b,
                    ConstVal::Bv(..) => return None,
                }
            }
            Some(ConstVal::Bool(all))
        }
        TermKind::Or(args) => {
            let mut any = false;
            for arg in args {
                match eval_term_const(tm, arg, pins)? {
                    ConstVal::Bool(b) => any |= b,
                    ConstVal::Bv(..) => return None,
                }
            }
            Some(ConstVal::Bool(any))
        }

        // ── Choice ────────────────────────────────────────────────────────────
        TermKind::Ite(c, t, e) => match eval_term_const(tm, c, pins)? {
            ConstVal::Bool(true) => eval_term_const(tm, t, pins),
            ConstVal::Bool(false) => eval_term_const(tm, e, pins),
            ConstVal::Bv(..) => None,
        },

        // Any construct the encoder never emits renders as "?".
        _ => None,
    }
}

/// Fold a term that is expected to be bit-vector-sorted to its magnitude.
fn eval_bv(tm: &TermManager, term: TermId, pins: &HashMap<TermId, BigInt>) -> Option<BigInt> {
    match eval_term_const(tm, term, pins)? {
        ConstVal::Bv(value, _width) => Some(value),
        ConstVal::Bool(_) => None,
    }
}

/// Render a folded [`ConstVal`] using the same sort-matching logic
/// [`build_counterexample`] uses for input values (sharing [`decode_bits`]).
fn render_const(value: ConstVal, out_sort: ValSort) -> Option<String> {
    match (value, out_sort) {
        (ConstVal::Bv(v, _w), ValSort::Int(ty)) => Some(decode_bits(&v, ty)),
        (ConstVal::Bool(b), ValSort::Bool) => Some(b.to_string()),
        (ConstVal::Bv(v, _w), ValSort::Bool) => Some(
            if v == BigInt::from(0u64) {
                "false"
            } else {
                "true"
            }
            .to_string(),
        ),
        _ => None,
    }
}

/// Evaluate both output terms under the pinned input values and render them.
/// Returns `(left, right)` rendered strings (or `None` each if a value cannot
/// be recovered). This is purely for display; it never affects the verdict.
///
/// Uses a pure constant-fold over the BV/Bool term DAG (seeded by the concrete
/// model input values) rather than spawning a second solver, avoiding the
/// pathological re-solve-over-a-symbolic-multiplier slowdown that previously
/// made compound-expression counterexamples hang.
fn outputs_under_pins(
    tm: &TermManager,
    pins: &[(TermId, u32, BigInt)],
    out_sort: ValSort,
    out_a_term: TermId,
    out_b_term: TermId,
) -> (Option<String>, Option<String>) {
    // Concrete unsigned value per parameter term id.
    let pin_map: HashMap<TermId, BigInt> = pins.iter().map(|(t, _w, v)| (*t, v.clone())).collect();

    let left = eval_term_const(tm, out_a_term, &pin_map).and_then(|v| render_const(v, out_sort));
    let right = eval_term_const(tm, out_b_term, &pin_map).and_then(|v| render_const(v, out_sort));
    (left, right)
}

/// Read a term's raw unsigned bit-vector value from a model, if present.
fn model_bitvec_value(
    model: &oxiz::solver::Model,
    tm: &mut TermManager,
    term: TermId,
) -> Option<BigInt> {
    let value_term = match model.get(term) {
        Some(v) => v,
        None => model.eval(term, tm),
    };
    match tm.get(value_term).map(|t| t.kind.clone())? {
        TermKind::BitVecConst { value, .. } => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod eval_tests {
    //! Unit tests for the pure counterexample-output evaluator and the full
    //! refute-with-outputs path. These guard that rendering counterexample
    //! output values is (a) bit-precise and (b) solver-free (and therefore
    //! fast — the regression these replace took 98s in a re-solve).

    use super::*;

    /// A 32-bit variable plus a `(term, value)` pin for it.
    fn var_u32(tm: &mut TermManager, name: &str, value: u64) -> (TermId, (TermId, BigInt)) {
        let bv32 = tm.sorts.bitvec(32);
        let v = tm.mk_var(name, bv32);
        (v, (v, BigInt::from(value)))
    }

    /// `(a + b) * 2` and `a - b` fold to concrete masked magnitudes.
    #[test]
    fn folds_add_mul_and_sub() {
        let mut tm = TermManager::new();
        let (a, pin_a) = var_u32(&mut tm, "a", 5);
        let (b, pin_b) = var_u32(&mut tm, "b", 3);
        let two = tm.mk_bitvec(2i64, 32);
        let sum = tm.mk_bv_add(a, b);
        let prod = tm.mk_bv_mul(sum, two);
        let diff = tm.mk_bv_sub(a, b);

        let pins: HashMap<TermId, BigInt> = [pin_a, pin_b].into_iter().collect();

        match eval_term_const(&tm, prod, &pins).expect("prod folds") {
            ConstVal::Bv(v, w) => {
                assert_eq!(v, BigInt::from(16));
                assert_eq!(w, 32);
            }
            other => panic!("expected Bv, got {other:?}"),
        }
        match eval_term_const(&tm, diff, &pins).expect("diff folds") {
            ConstVal::Bv(v, w) => {
                assert_eq!(v, BigInt::from(2));
                assert_eq!(w, 32);
            }
            other => panic!("expected Bv, got {other:?}"),
        }
    }

    /// Subtraction wraps around modulo `2^width` (here `3 - 5` in `u32`).
    #[test]
    fn sub_wraps_around() {
        let mut tm = TermManager::new();
        let (a, pin_a) = var_u32(&mut tm, "a", 3);
        let (b, pin_b) = var_u32(&mut tm, "b", 5);
        let diff = tm.mk_bv_sub(a, b);
        let pins: HashMap<TermId, BigInt> = [pin_a, pin_b].into_iter().collect();
        // 3 - 5 = -2 ≡ 2^32 - 2 = 4294967294.
        match eval_term_const(&tm, diff, &pins).expect("diff folds") {
            ConstVal::Bv(v, _) => assert_eq!(v, BigInt::from(4_294_967_294u64)),
            other => panic!("expected Bv, got {other:?}"),
        }
    }

    /// A left shift `x << 1` folds (and masks within width).
    #[test]
    fn folds_left_shift() {
        let mut tm = TermManager::new();
        let (x, pin_x) = var_u32(&mut tm, "x", 0x4000_0001);
        let one = tm.mk_bitvec(1i64, 32);
        let shl = tm.mk_bv_shl(x, one);
        let pins: HashMap<TermId, BigInt> = [pin_x].into_iter().collect();
        // 0x40000001 << 1 = 0x80000002 (no overflow past 32 bits).
        match eval_term_const(&tm, shl, &pins).expect("shl folds") {
            ConstVal::Bv(v, _) => assert_eq!(v, BigInt::from(0x8000_0002u64)),
            other => panic!("expected Bv, got {other:?}"),
        }
    }

    /// Arithmetic shift right of a negative value sign-fills (signed `>> 1`).
    #[test]
    fn folds_arithmetic_shift_right_negative() {
        let mut tm = TermManager::new();
        // 0xFFFFFFF0 is -16 as i32; ashr by 1 → -8 = 0xFFFFFFF8.
        let (x, pin_x) = var_u32(&mut tm, "x", 0xFFFF_FFF0);
        let one = tm.mk_bitvec(1i64, 32);
        let ashr = tm.mk_bv_ashr(x, one);
        let pins: HashMap<TermId, BigInt> = [pin_x].into_iter().collect();
        match eval_term_const(&tm, ashr, &pins).expect("ashr folds") {
            ConstVal::Bv(v, _) => assert_eq!(v, BigInt::from(0xFFFF_FFF8u64)),
            other => panic!("expected Bv, got {other:?}"),
        }
    }

    /// Signed less-than honours two's-complement: `-1 < 1` is true even though
    /// the unsigned magnitudes are `0xFFFFFFFF` and `1`.
    #[test]
    fn folds_signed_less_than() {
        let mut tm = TermManager::new();
        let (a, pin_a) = var_u32(&mut tm, "a", 0xFFFF_FFFF); // -1
        let (b, pin_b) = var_u32(&mut tm, "b", 1); // 1
        let slt = tm.mk_bv_slt(a, b);
        let ult = tm.mk_bv_ult(a, b);
        let pins: HashMap<TermId, BigInt> = [pin_a, pin_b].into_iter().collect();
        match eval_term_const(&tm, slt, &pins).expect("slt folds") {
            ConstVal::Bool(b) => assert!(b, "-1 <_s 1 must be true"),
            other => panic!("expected Bool, got {other:?}"),
        }
        match eval_term_const(&tm, ult, &pins).expect("ult folds") {
            ConstVal::Bool(b) => assert!(!b, "0xFFFFFFFF <_u 1 must be false"),
            other => panic!("expected Bool, got {other:?}"),
        }
    }

    /// `ite(a == b, a, b)` selects the taken branch after folding the condition.
    #[test]
    fn folds_ite_and_eq() {
        let mut tm = TermManager::new();
        let (a, pin_a) = var_u32(&mut tm, "a", 7);
        let (b, pin_b) = var_u32(&mut tm, "b", 9);
        let eq = tm.mk_eq(a, b);
        let ite = tm.mk_ite(eq, a, b);
        let pins: HashMap<TermId, BigInt> = [pin_a, pin_b].into_iter().collect();
        // a != b, so the else branch (b = 9) is taken.
        match eval_term_const(&tm, ite, &pins).expect("ite folds") {
            ConstVal::Bv(v, _) => assert_eq!(v, BigInt::from(9)),
            other => panic!("expected Bv, got {other:?}"),
        }
    }

    /// An unbound variable yields `None` (renders as `"?"`), never a wrong value.
    #[test]
    fn unbound_var_is_none() {
        let mut tm = TermManager::new();
        let bv32 = tm.sorts.bitvec(32);
        let a = tm.mk_var("a", bv32);
        let pins: HashMap<TermId, BigInt> = HashMap::new();
        assert!(eval_term_const(&tm, a, &pins).is_none());
    }

    fn parse_fn(src: &str) -> syn::ItemFn {
        syn::parse_str(src).expect("fn should parse")
    }

    /// End-to-end: a corrupted helper (`a - b`) is REFUTED against the run
    /// (`(a + b) * 2`), AND the counterexample carries concrete, bit-precise
    /// output values for both sides (never `None`/`"?"`). This is the full fix:
    /// the output rendering completes in milliseconds via the pure evaluator.
    #[test]
    fn refute_populates_concrete_outputs() {
        let original =
            parse_fn("fn calc(a: u32, b: u32) -> u32 { let s = a + b; let t = s * 2; t }");
        let corrupted = parse_fn("fn calc(a: u32, b: u32) -> u32 { a - b }");

        let verdict = EquivBuilder::prove(&original, &corrupted);
        let cx = match verdict {
            Verdict::Refuted(cx) => cx,
            other => panic!("expected Refuted, got {other:?}"),
        };

        // Both outputs must be populated (the whole point of the fix).
        let left = cx.left_output.expect("left output must be concrete");
        let right = cx.right_output.expect("right output must be concrete");

        // Recompute the expected rendered values from the witness inputs and
        // confirm the evaluator agrees with a direct two's-complement decode.
        let u32t = IntType {
            width: 32,
            signed: false,
        };
        let lookup = |name: &str| -> BigInt {
            let raw = cx
                .inputs
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.clone())
                .unwrap_or_else(|| panic!("input `{name}` missing"));
            BigInt::parse_bytes(raw.as_bytes(), 10).expect("decimal input")
        };
        let av = lookup("a");
        let bv = lookup("b");
        let mask = (BigInt::from(1u64) << 32) - BigInt::from(1u64);
        let expected_left = decode_bits(&(((&av + &bv) * BigInt::from(2)) & &mask), u32t);
        let modulus = BigInt::from(1u64) << 32;
        let expected_right = decode_bits(&(((&av - &bv) % &modulus + &modulus) % &modulus), u32t);

        assert_eq!(left, expected_left, "left output must equal (a+b)*2");
        assert_eq!(
            right, expected_right,
            "right output must equal a-b (wrapping)"
        );
        // The witness must actually distinguish the two sides.
        assert_ne!(left, right, "a genuine counterexample must differ");
    }
}
