//! SMT-verified function-extraction transform.
//!
//! Extracts a pure fixed-width-integer sub-block of an over-budget free
//! function into a helper function — and COMMITS the extraction ONLY when the
//! OxiZ-backed equivalence oracle PROVES the rewrite preserves the enclosing
//! function's semantics. This realises the project's core proposal: *only
//! execute proven refactorings*.
//!
//! Pipeline (per over-budget free function):
//! 1. `detector::find_candidate` — locate a maximal contiguous pure-integer
//!    run with clean, integer-typed live-in/live-out sets.
//! 2. `rewriter::rewrite` — synthesise the helper `ItemFn` and the rewritten
//!    enclosing function that calls it.
//! 3. `gate::verify_and_decide` — prove the rewrite equivalent (whole-function
//!    inline-back proof, falling back to a componentwise block proof). Commit
//!    only on a `Verified`; a `Refuted` (a rewriter/detector bug) is a skip.
//!
//! On a committing verdict the input `syn::File` is mutated in place: the
//! original `Item::Fn` is replaced by the rewritten function and the helper is
//! inserted as a sibling `Item::Fn`. The committed helper is a normal free
//! function that later flows through `module_generator` unchanged.
//!
//! Scope of this cut: **free functions only**. Impl-method extraction
//! (`impl … { fn … }`) is out of scope here and is intentionally skipped.
//!
//! Gated behind the off-by-default `smt` cargo feature.

mod dataflow;
mod detector;
mod gate;
mod rewriter;

use gate::GateDecision;

/// The outcome of attempting to extract a pure block from one function.
#[derive(Debug)]
pub enum ExtractionOutcome {
    /// A proof passed and the file was mutated: the run was factored into
    /// `helper_ident`, called from the rewritten `fn_ident`.
    Committed {
        /// The enclosing function whose body was rewritten.
        fn_ident: String,
        /// The synthesised helper function inserted as a sibling.
        helper_ident: String,
    },
    /// The verification gate REFUTED the rewrite — a rewriter/detector bug was
    /// caught (semantic drift). The extraction was NOT applied. This is the
    /// safety net that proves the gate rejects rather than rubber-stamps.
    SkippedRefuted {
        /// The enclosing function the (rejected) extraction targeted.
        fn_ident: String,
        /// The concrete counterexample distinguishing the run from the helper.
        cx: crate::smt::Counterexample,
    },
    /// No extraction was committed because the candidate (or the function) lies
    /// outside the modelled fragment, or no qualifying candidate was found.
    SkippedUnsupported {
        /// The enclosing function considered.
        fn_ident: String,
        /// Why no proof-backed extraction was possible.
        reason: String,
    },
}

/// Extract a pure block from every over-budget free function in `file`,
/// committing each extraction only when proven equivalence-preserving.
///
/// `max_lines` is the trigger threshold: only functions whose rendered length
/// exceeds it are considered. The `file` is mutated in place for each committed
/// extraction (the rewritten function replaces the original; the helper is
/// inserted as a sibling immediately before it). Returns one outcome per
/// considered over-budget function.
pub fn extract_pure_blocks(file: &mut syn::File, max_lines: usize) -> Vec<ExtractionOutcome> {
    let mut outcomes = Vec::new();

    // We mutate `file.items` while iterating, so drive the loop by index and
    // recompute on each committed insertion (which shifts subsequent indices).
    let mut idx = 0usize;
    while idx < file.items.len() {
        // Only free functions are in scope for this cut.
        let func = match &file.items[idx] {
            syn::Item::Fn(f) => f.clone(),
            _ => {
                idx += 1;
                continue;
            }
        };

        // Trigger only on over-budget functions; otherwise advance silently.
        if detector::rendered_line_count(&func) <= max_lines {
            idx += 1;
            continue;
        }

        let fn_ident = func.sig.ident.to_string();

        // 1. Detect a candidate run.
        let candidate = match detector::find_candidate(&func, max_lines) {
            Some(c) => c,
            None => {
                outcomes.push(ExtractionOutcome::SkippedUnsupported {
                    fn_ident,
                    reason: "no qualifying pure-integer run found".to_string(),
                });
                idx += 1;
                continue;
            }
        };

        // 2. Synthesise the helper + rewritten function.
        let existing = existing_fn_idents(file);
        let rewrite = match rewriter::rewrite(&func, &candidate, &existing) {
            Some(r) => r,
            None => {
                outcomes.push(ExtractionOutcome::SkippedUnsupported {
                    fn_ident,
                    reason: "could not synthesise a helper for the candidate run".to_string(),
                });
                idx += 1;
                continue;
            }
        };

        // 3. Verify and decide.
        match gate::verify_and_decide(&func, &candidate, &rewrite) {
            GateDecision::Commit { block_verified } => {
                let helper_ident = rewrite.helper_ident.clone();
                // Record which proof path authorised the commit (whole-function
                // inline-back vs componentwise block proof) for traceability.
                let proof_path = if block_verified {
                    "block-verified"
                } else {
                    "whole-function-verified"
                };
                eprintln!(
                    "smt-extract: proof for `{}` -> `{}`: {proof_path}",
                    func.sig.ident, rewrite.helper_ident
                );
                // Mutate the file: replace the original fn, insert the helper as
                // a sibling immediately before the rewritten function.
                file.items[idx] = syn::Item::Fn(rewrite.rewritten_fn);
                file.items.insert(idx, syn::Item::Fn(rewrite.helper));
                outcomes.push(ExtractionOutcome::Committed {
                    fn_ident,
                    helper_ident,
                });
                // Skip past both the helper (now at idx) and the rewritten fn.
                idx += 2;
            }
            GateDecision::Refuted(cx) => {
                outcomes.push(ExtractionOutcome::SkippedRefuted { fn_ident, cx });
                idx += 1;
            }
            GateDecision::Skip { reason } => {
                outcomes.push(ExtractionOutcome::SkippedUnsupported { fn_ident, reason });
                idx += 1;
            }
        }
    }

    outcomes
}

/// A public, test-only view of the verification gate's decision.
///
/// Mirrors the internal gate outcome but with no `pub(crate)` types, so that the
/// `extraction_e2e_tests` integration test can feed the gate a deliberately
/// MIS-DERIVED rewrite and assert it is REFUTED (never committed). This is the
/// empirical proof that the SMT gate rejects semantic drift rather than
/// rubber-stamping it. `#[doc(hidden)]`: it is not part of the stable surface.
#[doc(hidden)]
#[derive(Debug)]
pub enum GateVerdictForTest {
    /// A `Verified` proof authorised a commit.
    WouldCommit,
    /// The gate REFUTED the rewrite (a rewriter/detector bug was caught).
    Refuted(crate::smt::Counterexample),
    /// No proof could be obtained (out of fragment / free variable).
    Skipped(String),
}

/// Test-only gate driver: detect the natural candidate in `original_src`, build
/// the natural rewrite, then OPTIONALLY override the synthesised helper with a
/// caller-supplied (possibly broken) `helper_override_src` before running the
/// verification gate. Returns the gate's verdict so a test can assert it.
///
/// When `helper_override_src` is `Some`, the override stands in for the helper
/// the rewriter would have produced — this is how the rewriter-bug guard injects
/// a helper that computes the wrong thing and proves the gate refuses to commit.
///
/// `#[doc(hidden)]`: a deliberate test surface, not stable API.
#[doc(hidden)]
pub fn verify_rewrite_for_test(
    original_src: &str,
    max_lines: usize,
    helper_override_src: Option<&str>,
) -> Option<GateVerdictForTest> {
    let func: syn::ItemFn = syn::parse_str(original_src).ok()?;
    let cand = detector::find_candidate(&func, max_lines)?;
    let mut rewrite = rewriter::rewrite(&func, &cand, &[func.sig.ident.to_string()])?;
    if let Some(src) = helper_override_src {
        rewrite.helper = syn::parse_str(src).ok()?;
    }
    let verdict = match gate::verify_and_decide(&func, &cand, &rewrite) {
        GateDecision::Commit { .. } => GateVerdictForTest::WouldCommit,
        GateDecision::Refuted(cx) => GateVerdictForTest::Refuted(cx),
        GateDecision::Skip { reason } => GateVerdictForTest::Skipped(reason),
    };
    Some(verdict)
}

/// Collect the identifiers of all free functions currently in `file` (used for
/// collision-free helper naming).
fn existing_fn_idents(file: &syn::File) -> Vec<String> {
    file.items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Fn(f) => Some(f.sig.ident.to_string()),
            _ => None,
        })
        .collect()
}

/// Print a per-outcome human-readable report to stdout.
pub fn print_report(outcomes: &[ExtractionOutcome]) {
    if outcomes.is_empty() {
        println!("smt-extract: no over-budget free functions considered.");
        return;
    }
    for outcome in outcomes {
        match outcome {
            ExtractionOutcome::Committed {
                fn_ident,
                helper_ident,
            } => {
                println!(
                    "smt-extract: COMMITTED  {fn_ident} -> extracted `{helper_ident}` (proof: Verified)"
                );
            }
            ExtractionOutcome::SkippedRefuted { fn_ident, cx } => {
                println!(
                    "smt-extract: SKIPPED    {fn_ident} -> REFUTED by the oracle (rewriter bug caught)"
                );
                let inputs = cx
                    .inputs
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                println!("             counterexample inputs: {inputs}");
                if let (Some(l), Some(r)) = (&cx.left_output, &cx.right_output) {
                    println!("             original={l}  helper={r}");
                }
            }
            ExtractionOutcome::SkippedUnsupported { fn_ident, reason } => {
                println!("smt-extract: SKIPPED    {fn_ident} -> {reason}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_file(src: &str) -> syn::File {
        syn::parse_str(src).expect("parse file")
    }

    #[test]
    fn commits_and_inserts_helper() {
        let mut file = parse_file(
            "fn calc(a: u32, b: u32) -> u32 {\n\
             let s = a + b;\n\
             let t = s * 2;\n\
             let u = t + a;\n\
             u\n\
             }",
        );
        let outcomes = extract_pure_blocks(&mut file, 1);
        assert!(
            outcomes
                .iter()
                .any(|o| matches!(o, ExtractionOutcome::Committed { .. })),
            "expected at least one Committed outcome, got {outcomes:?}"
        );
        // The file must now hold BOTH a helper and the rewritten caller.
        let rendered = prettyplease::unparse(&file);
        assert!(
            rendered.contains("calc_extracted"),
            "helper missing:\n{rendered}"
        );
        // Round-trip: the mutated file must still be valid Rust.
        syn::parse_file(&rendered).expect("mutated file must re-parse");
    }

    #[test]
    fn impure_call_is_not_committed() {
        // Every value in `withcall` flows through a function call, so there is
        // no pure-integer run to extract; the result type of `helper(a)` cannot
        // be modelled, so the tail `t + 1` reads an untyped pre-run binding and
        // the run is rejected. (The helper itself is declared #[inline] huge via
        // a body that is also call-bearing, so it offers no candidate either.)
        let mut file = parse_file(
            "fn withcall(a: u32) -> u32 {\n\
             let t = helper(a);\n\
             let u = helper(t);\n\
             u + 1\n\
             }\n\
             fn helper(x: u32) -> u32 { other(x) }",
        );
        let before = prettyplease::unparse(&file);
        let outcomes = extract_pure_blocks(&mut file, 1);
        assert!(
            !outcomes
                .iter()
                .any(|o| matches!(o, ExtractionOutcome::Committed { .. })),
            "must not commit an impure extraction, got {outcomes:?}"
        );
        // The file must be byte-for-byte unchanged when nothing commits.
        let after = prettyplease::unparse(&file);
        assert_eq!(before, after, "file must be unchanged when nothing commits");
    }
}
