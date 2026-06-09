//! End-to-end tests for the SMT-verified function-extraction transform
//! (`smt` feature).
//!
//! These exercise the public `extraction` API:
//!   1. **Happy path** — an over-budget pure-integer function gets a sub-run
//!      factored into a helper, the gate PROVES equivalence, the file is mutated
//!      to hold both the helper and the rewritten caller, and the result is
//!      valid Rust that round-trips through `prettyplease`/`syn`.
//!   2. **Impure block not extracted** — a function whose computation flows
//!      through a function call yields NO `Committed` outcome and leaves the
//!      file unchanged.
//!   3. **Rewriter-bug guard** — feeding the verification gate a deliberately
//!      MIS-DERIVED helper (computes the wrong arithmetic) is REFUTED, never
//!      committed. This empirically proves the gate rejects semantic drift.
//!
//! Temp files (where used) go through the OS temp dir per project policy.

#![cfg(feature = "smt")]

use std::fs;

use splitrs::extraction::{
    extract_pure_blocks, verify_rewrite_for_test, ExtractionOutcome, GateVerdictForTest,
};

/// A pure-integer function long enough to trip a small `max_lines` budget, with
/// an extractable straight-line sub-run feeding a later use.
const PURE_FN: &str = "\
fn mix(a: u32, b: u32) -> u32 {
    let s = a + b;
    let t = s * 3;
    let u = t ^ a;
    let v = u + b;
    v
}
";

#[test]
fn happy_path_commits_and_round_trips() {
    let mut file: syn::File = syn::parse_str(PURE_FN).expect("fixture parses");

    // Force the trigger with a tiny budget so the function is "over budget".
    let outcomes = extract_pure_blocks(&mut file, 1);

    // At least one extraction must have been COMMITTED (proof passed).
    let committed: Vec<_> = outcomes
        .iter()
        .filter_map(|o| match o {
            ExtractionOutcome::Committed {
                fn_ident,
                helper_ident,
            } => Some((fn_ident.clone(), helper_ident.clone())),
            _ => None,
        })
        .collect();
    assert!(
        !committed.is_empty(),
        "expected at least one Committed outcome, got {outcomes:?}"
    );
    let (fn_ident, helper_ident) = &committed[0];
    assert_eq!(fn_ident, "mix");
    assert!(
        helper_ident.starts_with("mix_extracted"),
        "helper should be named after the origin, got {helper_ident}"
    );

    // The mutated file must contain BOTH the helper and the rewritten caller
    // (with a call to the helper).
    let rendered = prettyplease::unparse(&file);
    assert!(
        rendered.contains(helper_ident.as_str()),
        "rendered file must contain the helper `{helper_ident}`:\n{rendered}"
    );
    assert!(
        rendered.contains(&format!("{helper_ident}(")),
        "rendered file must contain a call to `{helper_ident}`:\n{rendered}"
    );

    // The mutated output must be valid Rust (round-trips through syn).
    let reparsed = syn::parse_file(&rendered).expect("mutated file must re-parse as valid Rust");
    // Sanity: there are now at least two free functions (helper + caller).
    let fn_count = reparsed
        .items
        .iter()
        .filter(|i| matches!(i, syn::Item::Fn(_)))
        .count();
    assert!(
        fn_count >= 2,
        "expected helper + caller (>=2 fns), found {fn_count}"
    );
}

#[test]
fn happy_path_via_tempfile_round_trips() {
    // Same happy path, but written through a temp file (policy: OS temp dir).
    let dir = std::env::temp_dir().join(format!("splitrs_extraction_{}", std::process::id()));
    fs::create_dir_all(&dir).expect("create temp dir");
    let src_path = dir.join("input.rs");
    fs::write(&src_path, PURE_FN).expect("write fixture");

    let source = fs::read_to_string(&src_path).expect("read fixture");
    let mut file: syn::File = syn::parse_str(&source).expect("parse fixture");
    let outcomes = extract_pure_blocks(&mut file, 1);

    assert!(
        outcomes
            .iter()
            .any(|o| matches!(o, ExtractionOutcome::Committed { .. })),
        "expected a Committed outcome, got {outcomes:?}"
    );

    let rendered = prettyplease::unparse(&file);
    let out_path = dir.join("output.rs");
    fs::write(&out_path, &rendered).expect("write output");
    let written = fs::read_to_string(&out_path).expect("read output");
    syn::parse_file(&written).expect("written output must be valid Rust");

    // Clean up the temp directory.
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn impure_call_block_is_not_extracted() {
    // Every value flows through a function call whose result type cannot be
    // modelled, so no pure-integer run qualifies -> nothing is committed and the
    // file is left byte-for-byte unchanged.
    let src = "\
fn pipeline(a: u32) -> u32 {
    let t = stage_one(a);
    let u = stage_two(t);
    u + 1
}
fn stage_one(x: u32) -> u32 { passthrough(x) }
";
    let mut file: syn::File = syn::parse_str(src).expect("fixture parses");
    let before = prettyplease::unparse(&file);

    let outcomes = extract_pure_blocks(&mut file, 1);
    assert!(
        !outcomes
            .iter()
            .any(|o| matches!(o, ExtractionOutcome::Committed { .. })),
        "an impure (call-bearing) region must never be committed, got {outcomes:?}"
    );

    let after = prettyplease::unparse(&file);
    assert_eq!(before, after, "file must be unchanged when nothing commits");
}

#[test]
fn rewriter_bug_is_refuted_not_committed() {
    // CRITICAL: drive the gate with a MIS-DERIVED helper. The natural extraction
    // of `mix` factors a sub-run that computes (among other things) `s = a + b`;
    // we override the synthesised helper with one that SUBTRACTS instead of
    // adds. The whole-function inline-back proof must detect the divergence and
    // REFUTE the rewrite. A gate that rubber-stamped would (wrongly) commit.
    let broken_helper = "fn mix_extracted(a: u32, b: u32) -> u32 { a - b }";
    let verdict = verify_rewrite_for_test(PURE_FN, 1, Some(broken_helper))
        .expect("a candidate must be found in the pure fixture");

    match verdict {
        GateVerdictForTest::Refuted(cx) => {
            // The counterexample must name the inputs that distinguish them.
            assert!(
                !cx.inputs.is_empty(),
                "a refutation must carry a concrete counterexample"
            );
        }
        other => {
            panic!("the SMT gate MUST refute a mis-derived helper (semantic drift), got {other:?}")
        }
    }
}

#[test]
fn correct_rewrite_would_commit() {
    // The companion to the rewriter-bug guard: with NO override (the rewriter's
    // own faithful helper), the gate PROVES equivalence and would commit. This
    // confirms the gate is not vacuously refusing everything.
    let verdict = verify_rewrite_for_test(PURE_FN, 1, None).expect("a candidate must be found");
    assert!(
        matches!(verdict, GateVerdictForTest::WouldCommit),
        "a faithful rewrite must be proven and committable, got {verdict:?}"
    );
}
