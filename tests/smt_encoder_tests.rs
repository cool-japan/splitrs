//! Integration tests for the QF_BV equivalence oracle (`smt` feature).
//!
//! Each test builds a pair of `syn::ItemFn` values and asserts the proven
//! [`Verdict`]. Soundness-critical cases (signed arithmetic shift, refutation
//! with a concrete counterexample) are covered explicitly.

#![cfg(feature = "smt")]

use splitrs::smt::{prove_equivalent, Verdict};
use syn::parse_quote;

/// Assert that two functions are proven equivalent.
fn assert_verified(a: syn::ItemFn, b: syn::ItemFn) {
    match prove_equivalent(&a, &b) {
        Verdict::Verified => {}
        other => panic!("expected Verified, got {other:?}"),
    }
}

/// Assert that two functions are reported Unsupported.
fn assert_unsupported(a: syn::ItemFn, b: syn::ItemFn) {
    match prove_equivalent(&a, &b) {
        Verdict::Unsupported { .. } => {}
        other => panic!("expected Unsupported, got {other:?}"),
    }
}

#[test]
fn add_is_commutative() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { a + b } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { b + a } };
    assert_verified(a, b);
}

#[test]
fn double_equals_self_add() {
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { 2 * x } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x + x } };
    assert_verified(a, b);
}

#[test]
fn add_is_associative() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32, c: u32) -> u32 { (a + b) + c } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32, c: u32) -> u32 { a + (b + c) } };
    assert_verified(a, b);
}

#[test]
fn mul_by_two_equals_shift_left_unsigned() {
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x * 2 } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x << 1 } };
    assert_verified(a, b);
}

#[test]
fn mul_by_three_is_not_identity() {
    // Regression guard for a counterexample-extraction soundness bug: binding a
    // fresh `out == body` variable before the disequality check made a genuine
    // `Sat` collapse to `Unsat` whenever the body contained a bit-vector multiply
    // (so `x * 3` was wrongly "proven" equal to `x`). The oracle now decides the
    // verdict from the bare disequality, so a non-power-of-two multiply must be
    // correctly REFUTED against the identity.
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x * 3 } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x } };
    match prove_equivalent(&a, &b) {
        Verdict::Refuted(cx) => {
            assert!(
                !cx.inputs.is_empty(),
                "a refutation must carry a concrete witness for x"
            );
        }
        other => panic!("`x * 3` must be Refuted against `x`, got {other:?}"),
    }
}

#[test]
fn mul_by_three_equals_repeated_addition() {
    // Companion to the regression guard: a genuine multiply identity must still
    // be Verified (the fix must not over-reject). `x * 3 == x + x + x`.
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x * 3 } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x + x + x } };
    assert_verified(a, b);
}

#[test]
fn mul_with_let_binding_is_not_identity() {
    // The same soundness guard through a `let`-bound multiply (the shape the
    // extraction transform produces): `let t = x * 3; t` must be Refuted vs `x`.
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { let t = x * 3; t } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x } };
    match prove_equivalent(&a, &b) {
        Verdict::Refuted(_) => {}
        other => panic!("`let t = x * 3; t` must be Refuted against `x`, got {other:?}"),
    }
}

#[test]
fn if_swap_with_negated_condition() {
    let a: syn::ItemFn = parse_quote! {
        fn f(a: i32, b: i32, c: bool) -> i32 { if c { a } else { b } }
    };
    let b: syn::ItemFn = parse_quote! {
        fn f(a: i32, b: i32, c: bool) -> i32 { if !c { b } else { a } }
    };
    assert_verified(a, b);
}

#[test]
fn de_morgan_law() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { !(a & b) } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { (!a) | (!b) } };
    assert_verified(a, b);
}

#[test]
fn cast_narrow_widen_equals_mask() {
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { (x as u8) as u32 } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x & 0xFF } };
    assert_verified(a, b);
}

#[test]
fn refute_add_vs_sub_with_counterexample() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { a + b } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { a - b } };
    match prove_equivalent(&a, &b) {
        Verdict::Refuted(cx) => {
            // The counterexample must populate both inputs.
            assert_eq!(cx.inputs.len(), 2, "expected two input bindings");
            let b_val = cx
                .inputs
                .iter()
                .find(|(name, _)| name == "b")
                .map(|(_, v)| v.clone())
                .expect("counterexample must bind `b`");
            // a + b and a - b differ exactly when b != 0.
            assert_ne!(
                b_val, "0",
                "counterexample must have b != 0, got b = {b_val}"
            );
        }
        other => panic!("expected Refuted, got {other:?}"),
    }
}

#[test]
fn unsupported_function_call() {
    let a: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { foo(x) } };
    let b: syn::ItemFn = parse_quote! { fn f(x: u32) -> u32 { x } };
    assert_unsupported(a, b);
}

#[test]
fn unsupported_float_param() {
    let a: syn::ItemFn = parse_quote! { fn f(x: f32) -> f32 { x } };
    let b: syn::ItemFn = parse_quote! { fn f(x: f32) -> f32 { x } };
    assert_unsupported(a, b);
}

#[test]
fn signed_shift_uses_arithmetic_shift() {
    // For a signed value, `x >> 1` is an *arithmetic* shift (sign-preserving).
    // The identity `(x >> 1) + (x >> 1)` differs from `x` for odd/negative x,
    // but we can pin the ashr vs lshr distinction with a sign-sensitive
    // identity: for i32, `(-2) >> 1 == -1`. We prove that an explicit
    // arithmetic-shift model agrees with Rust's `>>` on signed, by checking
    // that `x >> 1` equals itself encoded as signed (trivially Verified) AND
    // that the *unsigned* reinterpretation differs — captured below.
    //
    // Concretely: prove `x >> 1` (i32, arithmetic) is NOT equivalent to
    // `(x as u32) >> 1) as i32` (logical) — they disagree on negative x. A
    // Refuted verdict guards that signed `>>` lowered to ashr, not lshr.
    let arithmetic: syn::ItemFn = parse_quote! { fn f(x: i32) -> i32 { x >> 1 } };
    let logical: syn::ItemFn = parse_quote! { fn f(x: i32) -> i32 { ((x as u32) >> 1) as i32 } };
    match prove_equivalent(&arithmetic, &logical) {
        Verdict::Refuted(cx) => {
            // They must differ on some negative input (high bit set).
            let x_val = cx
                .inputs
                .iter()
                .find(|(name, _)| name == "x")
                .map(|(_, v)| v.clone())
                .expect("counterexample must bind `x`");
            assert!(
                x_val.starts_with('-'),
                "ashr vs lshr should diverge on a negative x, got x = {x_val}"
            );
        }
        other => {
            panic!("expected Refuted (proving signed >> is arithmetic, not logical), got {other:?}")
        }
    }
}

#[test]
fn signed_arithmetic_shift_self_identity() {
    // Sanity companion: signed `>>` against itself is Verified — confirms the
    // ashr path is internally consistent.
    let a: syn::ItemFn = parse_quote! { fn f(x: i32) -> i32 { x >> 2 } };
    let b: syn::ItemFn = parse_quote! { fn f(x: i32) -> i32 { (x >> 1) >> 1 } };
    assert_verified(a, b);
}

#[test]
fn let_binding_and_shadowing() {
    // `let` bindings (with SSA renaming for shadowing) must encode correctly.
    let a: syn::ItemFn = parse_quote! {
        fn f(a: u32, b: u32) -> u32 {
            let s = a + b;
            let s = s + s;
            s
        }
    };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { (a + b) * 2 } };
    assert_verified(a, b);
}

#[test]
fn trailing_return_is_allowed() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { return a + b; } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { b + a } };
    assert_verified(a, b);
}

#[test]
fn division_is_unsupported() {
    let a: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { a / b } };
    let b: syn::ItemFn = parse_quote! { fn f(a: u32, b: u32) -> u32 { a / b } };
    assert_unsupported(a, b);
}
