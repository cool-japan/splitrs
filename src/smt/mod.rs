//! SMT-backed semantic-equivalence oracle for verified refactoring.
//!
//! This module proves that two *pure, fixed-width-integer* Rust functions are
//! semantically equivalent for **all** inputs (or finds a concrete
//! counterexample, or reports the construct as unsupported). It lowers a small
//! supported `syn` fragment into the QF_BV theory of the OxiZ SMT solver and
//! asserts the disequality of the two functions' results: an `Unsat` answer is
//! a sound, bit-precise proof of equivalence.
//!
//! Gated behind the off-by-default `smt` cargo feature so a normal build never
//! pulls the solver.

mod encoder;
mod equiv;
mod types;

pub use equiv::EquivBuilder;
pub use types::{int_type_of, IntType};

/// Probe whether an expression lies entirely within the modelled pure,
/// fixed-width-integer fragment, **without** building any SMT terms.
///
/// Returns `Ok(())` if every construct is supported, or `Err(reason)` naming
/// the first unsupported construct. This is the purity gate the extraction
/// detector (Phase 4) uses to decide whether a candidate body is amenable to
/// equivalence proving before paying for a full encode.
pub fn is_pure_supported_expr(expr: &syn::Expr) -> Result<(), String> {
    encoder::probe_expr(expr)
}

/// Probe whether a block lies entirely within the modelled pure fragment,
/// without building SMT terms. See [`is_pure_supported_expr`].
pub fn is_pure_supported_block(block: &syn::Block) -> Result<(), String> {
    encoder::probe_block(block)
}

/// The result of an equivalence query between two functions.
#[derive(Debug, Clone)]
pub enum Verdict {
    /// Proven equivalent for all inputs (QF_BV `Unsat`).
    Verified,
    /// Refuted: a concrete input distinguishes the two functions.
    Refuted(Counterexample),
    /// The functions fall outside the modelled fragment; no proof attempted.
    Unsupported {
        /// Human-readable reason the proof could not be attempted.
        reason: String,
    },
}

/// A concrete counterexample distinguishing two functions.
#[derive(Debug, Clone)]
pub struct Counterexample {
    /// Each parameter name paired with its rendered decimal value.
    pub inputs: Vec<(String, String)>,
    /// The left function's output for these inputs (if recoverable).
    pub left_output: Option<String>,
    /// The right function's output for these inputs (if recoverable).
    pub right_output: Option<String>,
}

/// Prove two functions equivalent (delegates to [`EquivBuilder`]).
#[must_use]
pub fn prove_equivalent(fn_a: &syn::ItemFn, fn_b: &syn::ItemFn) -> Verdict {
    EquivBuilder::prove(fn_a, fn_b)
}

#[cfg(test)]
mod probe_tests {
    //! Exercises the non-mutating purity probe and the public type helpers.

    use super::{int_type_of, is_pure_supported_block, is_pure_supported_expr, IntType};

    #[test]
    fn accepts_supported_expr() {
        let expr: syn::Expr = syn::parse_str("a + b * 2").expect("parse");
        assert!(is_pure_supported_expr(&expr).is_ok());
    }

    #[test]
    fn rejects_call_expr() {
        let expr: syn::Expr = syn::parse_str("foo(a)").expect("parse");
        assert!(is_pure_supported_expr(&expr).is_err());
    }

    #[test]
    fn rejects_division_expr() {
        let expr: syn::Expr = syn::parse_str("a / b").expect("parse");
        let err = is_pure_supported_expr(&expr).expect_err("division must be rejected");
        assert!(err.contains("division"));
    }

    #[test]
    fn accepts_let_block() {
        let block: syn::Block = syn::parse_str("{ let s = a + b; s + s }").expect("parse block");
        assert!(is_pure_supported_block(&block).is_ok());
    }

    #[test]
    fn rejects_loop_block() {
        let block: syn::Block = syn::parse_str("{ while a < b { } a }").expect("parse block");
        assert!(is_pure_supported_block(&block).is_err());
    }

    #[test]
    fn int_type_helper_is_public() {
        let ty: syn::Type = syn::parse_str("u16").expect("parse type");
        assert_eq!(
            int_type_of(&ty),
            Some(IntType {
                width: 16,
                signed: false
            })
        );
    }
}

#[cfg(test)]
mod sanity {
    //! Step-0 soundness sanity check: confirms the QF_BV solver is sound by
    //! proving bvadd commutativity (Unsat under disequality) and refuting an
    //! add/sub confusion (Sat). If commutativity comes back Sat, the solver is
    //! UNSOUND and these tests fail loudly.

    use num_bigint::BigInt;
    use oxiz::core::TermKind;
    use oxiz::{Solver, SolverResult, TermManager};

    /// Guards the bit-blasting of `bvshl` with a constant shift: `x << 1` must
    /// be provably equal to `x * 2` (Unsat under disequality). Before the OxiZ
    /// shift-wiring fix this returned a spurious Sat.
    #[test]
    fn shl_one_proves_equal_to_mul_two() {
        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");
        let bv32 = tm.sorts.bitvec(32);
        let x = tm.mk_var("x", bv32);
        let one = tm.mk_bitvec(1i64, 32);
        let two = tm.mk_bitvec(2i64, 32);
        let shl = tm.mk_bv_shl(x, one);
        let mul = tm.mk_bv_mul(x, two);
        let eq = tm.mk_eq(shl, mul);
        let diseq = tm.mk_not(eq);
        solver.assert(diseq, &mut tm);
        assert_eq!(solver.check(&mut tm), SolverResult::Unsat);
    }

    /// Guards the counterexample model: `a + b` vs `a - b` is Sat, and the
    /// extracted model must be a genuine witness (`b != 0`), not a degenerate
    /// all-zero assignment. Before the model-snapshot + precedence fixes this
    /// returned `a = b = 0`, which is NOT a real counterexample.
    #[test]
    fn refute_model_is_a_genuine_witness() {
        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");
        let bv32 = tm.sorts.bitvec(32);
        let a = tm.mk_var("a", bv32);
        let b = tm.mk_var("b", bv32);
        let add = tm.mk_bv_add(a, b);
        let sub = tm.mk_bv_sub(a, b);
        let eq = tm.mk_eq(add, sub);
        let diseq = tm.mk_not(eq);
        solver.assert(diseq, &mut tm);
        assert_eq!(solver.check(&mut tm), SolverResult::Sat);
        let model = solver.model().expect("sat model");
        let b_term = model.get(b).expect("b must be in the model");
        match tm.get(b_term).map(|t| t.kind.clone()) {
            Some(TermKind::BitVecConst { value, .. }) => {
                assert_ne!(value, BigInt::from(0), "witness must have b != 0");
            }
            other => panic!("expected a BitVecConst for b, got {other:?}"),
        }
    }

    /// Guards the arithmetic-vs-logical shift distinction AND leaf-operand model
    /// tracking: signed `x >> 1` (ashr) differs from logical `>> 1` (lshr) for
    /// negative `x`, and the counterexample must expose a concrete value for the
    /// shifted leaf variable `x` (it must appear in the model).
    #[test]
    fn ashr_differs_from_lshr_with_x_in_model() {
        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");
        let bv32 = tm.sorts.bitvec(32);
        let x = tm.mk_var("x", bv32);
        let one = tm.mk_bitvec(1i64, 32);
        let ashr = tm.mk_bv_ashr(x, one);
        let lshr = tm.mk_bv_lshr(x, one);
        let eq = tm.mk_eq(ashr, lshr);
        let diseq = tm.mk_not(eq);
        solver.assert(diseq, &mut tm);
        assert_eq!(solver.check(&mut tm), SolverResult::Sat);
        let model = solver.model().expect("sat model");
        let x_term = model.get(x).expect("x must be in the model");
        assert!(
            matches!(
                tm.get(x_term).map(|t| t.kind.clone()),
                Some(TermKind::BitVecConst { .. })
            ),
            "x must decode to a concrete bitvector value"
        );
    }

    #[test]
    fn bvadd_is_commutative_unsat() {
        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");

        let bv32 = tm.sorts.bitvec(32);
        let x = tm.mk_var("x", bv32);
        let y = tm.mk_var("y", bv32);

        let xy = tm.mk_bv_add(x, y);
        let yx = tm.mk_bv_add(y, x);
        let eq = tm.mk_eq(xy, yx);
        let diseq = tm.mk_not(eq);
        solver.assert(diseq, &mut tm);

        assert_eq!(
            solver.check(&mut tm),
            SolverResult::Unsat,
            "bvadd commutativity must be Unsat under disequality — the solver is UNSOUND otherwise"
        );
    }

    #[test]
    fn bvadd_vs_bvsub_sat() {
        let mut tm = TermManager::new();
        let mut solver = Solver::new();
        solver.set_logic("QF_BV");

        let bv32 = tm.sorts.bitvec(32);
        let x = tm.mk_var("x", bv32);
        let y = tm.mk_var("y", bv32);

        let xy = tm.mk_bv_add(x, y);
        let xy_sub = tm.mk_bv_sub(x, y);
        let eq = tm.mk_eq(xy, xy_sub);
        let diseq = tm.mk_not(eq);
        solver.assert(diseq, &mut tm);

        assert_eq!(
            solver.check(&mut tm),
            SolverResult::Sat,
            "bvadd vs bvsub must be Sat under disequality (they differ when y != 0)"
        );
    }
}
