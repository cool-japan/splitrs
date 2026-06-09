//! Candidate detector: find a maximal contiguous pure-integer statement run
//! inside an over-budget function whose inputs/outputs are all integer-typed
//! and small enough to factor into a readable helper.
//!
//! A run qualifies when EVERY one of the following holds:
//! 1. every statement in the run is *in-fragment* per the SMT purity probe;
//! 2. every `let` in the run binds a plain `Pat::Ident` (no tuple/ref/`ref mut`);
//! 3. the dataflow `inputs` are all fixed-width-integer-typed;
//! 4. the dataflow `outputs` are all fixed-width-integer-typed and number ≤ 4
//!    (a cap that keeps the synthesised return tuple readable);
//! 5. no output binding is later re-bound (shadowed) — a conservative stand-in
//!    for "not reassigned", sound because the fragment has no `=` mutation.
//!
//! We greedily extend the longest qualifying run and emit a single best
//! [`Candidate`].  The detector only fires for functions whose rendered length
//! (via `prettyplease`) exceeds `max_lines`.

use crate::smt::int_type_of;

use super::dataflow::{analyze_run, RunDataflow};

/// A detected extraction candidate inside a single function.
///
/// (No `Debug` derive: `syn::Type` only implements `Debug` under syn's
/// `extra-traits` feature, which we do not enable.)
#[derive(Clone)]
pub(crate) struct Candidate {
    /// The enclosing function's identifier.
    pub(crate) fn_ident: String,
    /// The half-open statement index range `[start, end)` to extract.
    pub(crate) stmt_range: (usize, usize),
    /// Live-in identifiers with their integer types (helper parameters).
    pub(crate) inputs: Vec<(String, syn::Type)>,
    /// Live-out identifiers with their integer types (helper return values).
    pub(crate) outputs: Vec<(String, syn::Type)>,
    /// True when the run is the function tail and its value flows out as the
    /// function result (no named live-out binding; the tail expr is the output).
    pub(crate) tail_is_output: bool,
}

/// Count the rendered (prettyplease) line span of a function item.
pub(crate) fn rendered_line_count(func: &syn::ItemFn) -> usize {
    let file = syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![syn::Item::Fn(func.clone())],
    };
    prettyplease::unparse(&file).lines().count().max(1)
}

/// Find the best extraction candidate in `func`, or `None`.
///
/// `max_lines` is the trigger threshold: functions at or under it are skipped.
pub(crate) fn find_candidate(func: &syn::ItemFn, max_lines: usize) -> Option<Candidate> {
    if rendered_line_count(func) <= max_lines {
        return None;
    }
    let stmts = &func.block.stmts;
    let n = stmts.len();
    if n == 0 {
        return None;
    }

    // Precompute which statements are individually in-fragment + plain-let, so
    // run scanning is cheap. A run can only consist of such statements.
    let ok_stmt: Vec<bool> = stmts.iter().map(stmt_is_extractable).collect();

    let mut best: Option<Candidate> = None;
    let mut i = 0usize;
    while i < n {
        if !ok_stmt[i] {
            i += 1;
            continue;
        }
        // Extend a maximal block of individually-OK statements [i..k).
        let mut k = i;
        while k < n && ok_stmt[k] {
            k += 1;
        }
        // Within [i..k), greedily search for the longest qualifying sub-run.
        // Prefer the longest run anchored at i; if it fails the dataflow gate,
        // shrink from the end until a qualifying run is found.
        if let Some(cand) = best_run_in_window(func, i, k) {
            let len = cand.stmt_range.1 - cand.stmt_range.0;
            let better = match &best {
                None => true,
                Some(b) => len > (b.stmt_range.1 - b.stmt_range.0),
            };
            if better {
                best = Some(cand);
            }
        }
        i = k;
    }
    best
}

/// Search the contiguous in-fragment window `[lo, hi)` for the longest sub-run
/// that passes the full dataflow/typing gate. Returns the best [`Candidate`].
fn best_run_in_window(func: &syn::ItemFn, lo: usize, hi: usize) -> Option<Candidate> {
    // Try every start `s` from lo; for each, the longest end first, shrinking.
    // We want the globally longest qualifying run within the window, so iterate
    // lengths from largest to smallest and return the first that qualifies.
    let window = hi - lo;
    for len in (1..=window).rev() {
        for s in lo..=(hi - len) {
            let e = s + len;
            if let Some(cand) = qualify_run(func, s, e) {
                return Some(cand);
            }
        }
    }
    None
}

/// Apply the full qualification gate to the run `[i..j)`. Returns a
/// [`Candidate`] iff it passes every rule.
fn qualify_run(func: &syn::ItemFn, i: usize, j: usize) -> Option<Candidate> {
    let stmts = &func.block.stmts;
    let df: RunDataflow = analyze_run(func, i, j)?;

    // Rule 3 + 4: inputs and outputs must be integer-typed; outputs ≤ 4.
    if !df.inputs.iter().all(|(_, ty)| int_type_of(ty).is_some()) {
        return None;
    }
    if !df.outputs.iter().all(|(_, ty)| int_type_of(ty).is_some()) {
        return None;
    }
    if df.outputs.len() > 4 {
        return None;
    }

    // Rule 5: no output is re-bound (shadowed) anywhere after the run — a
    // conservative "not reassigned" check. (Re-`let` of an output name later
    // would mean the extracted value is not the one observed downstream.)
    if output_is_reshadowed(stmts, j, &df.outputs) {
        return None;
    }

    // Soundness invariant: every named live-out MUST be one of the run's own
    // `let`-bound defs (a live-out cannot exist without being defined inside the
    // run). If this ever fails the dataflow is inconsistent — refuse the run.
    if !df
        .outputs
        .iter()
        .all(|(name, _)| df.defs.iter().any(|d| d == name))
    {
        return None;
    }

    // Decide tail_is_output: the run reaches the function tail AND contributes
    // the function's result. This is the case when j == stmts.len(), the last
    // statement is a tail expression (no trailing semicolon), and there are no
    // named live-out bindings (the value flows directly out).
    let tail_is_output = j == stmts.len()
        && df.outputs.is_empty()
        && matches!(stmts.last(), Some(syn::Stmt::Expr(_, None)));

    // A run with NO outputs and NOT a tail-output produces nothing observable —
    // extracting it would be a pure dead-code move; reject (nothing to verify).
    if df.outputs.is_empty() && !tail_is_output {
        return None;
    }

    // A run must have at least one input OR be a tail-output run; a run with no
    // inputs and named outputs computes only from constants — still valid, but
    // the block-proof needs the inputs to bind free vars; an empty input set is
    // fine (the helper is nullary). We keep it.

    Some(Candidate {
        fn_ident: func.sig.ident.to_string(),
        stmt_range: (i, j),
        inputs: df.inputs,
        outputs: df.outputs,
        tail_is_output,
    })
}

/// Is a single statement individually extractable: in-fragment per the SMT
/// purity probe AND (if a `let`) binding a plain `Pat::Ident`?
fn stmt_is_extractable(stmt: &syn::Stmt) -> bool {
    match stmt {
        syn::Stmt::Local(local) => {
            if !is_plain_ident_let(local) {
                return false;
            }
            // `let ... else` is out of fragment.
            let Some(init) = &local.init else {
                return false;
            };
            if init.diverge.is_some() {
                return false;
            }
            crate::smt::is_pure_supported_expr(&init.expr).is_ok()
        }
        syn::Stmt::Expr(expr, _semi) => {
            // A trailing tail expr (or an expr-statement) must be in-fragment.
            // An early `return` is rejected by the probe.
            crate::smt::is_pure_supported_expr(expr).is_ok()
        }
        // Items / macros are never in-fragment.
        syn::Stmt::Item(_) | syn::Stmt::Macro(_) => false,
    }
}

/// True iff `local` binds a plain identifier (`let x` or `let x: T`), rejecting
/// tuples, refs, `ref mut`, and sub-patterns. A plain `mut x` is allowed (the
/// fragment never mutates it, so SSA semantics hold).
fn is_plain_ident_let(local: &syn::Local) -> bool {
    match &local.pat {
        syn::Pat::Ident(pat_ident) => pat_ident.by_ref.is_none() && pat_ident.subpat.is_none(),
        syn::Pat::Type(pat_type) => match pat_type.pat.as_ref() {
            syn::Pat::Ident(pat_ident) => pat_ident.by_ref.is_none() && pat_ident.subpat.is_none(),
            _ => false,
        },
        _ => false,
    }
}

/// Returns true if any output name is re-bound by a `let` in `stmts[after..]`.
fn output_is_reshadowed(
    stmts: &[syn::Stmt],
    after: usize,
    outputs: &[(String, syn::Type)],
) -> bool {
    if outputs.is_empty() || after >= stmts.len() {
        return false;
    }
    let out_names: Vec<&String> = outputs.iter().map(|(n, _)| n).collect();
    for stmt in &stmts[after..] {
        if let syn::Stmt::Local(local) = stmt {
            if let Some(name) = let_bound_name(local) {
                if out_names.iter().any(|n| **n == name) {
                    return true;
                }
            }
        }
    }
    false
}

/// The plain-ident name bound by a `let`, if any.
fn let_bound_name(local: &syn::Local) -> Option<String> {
    match &local.pat {
        syn::Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
        syn::Pat::Type(pat_type) => match pat_type.pat.as_ref() {
            syn::Pat::Ident(pat_ident) => Some(pat_ident.ident.to_string()),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn(src: &str) -> syn::ItemFn {
        syn::parse_str(src).expect("parse fn")
    }

    #[test]
    fn under_budget_is_skipped() {
        let func = parse_fn("fn small(a: u32) -> u32 { a + 1 }");
        // A huge max_lines means never trigger.
        assert!(find_candidate(&func, 1000).is_none());
    }

    #[test]
    fn finds_straight_line_run() {
        let func = parse_fn(
            "fn calc(a: u32, b: u32) -> u32 {\n\
             let s = a + b;\n\
             let t = s * 2;\n\
             let u = t + a;\n\
             u\n\
             }",
        );
        // Force trigger with a tiny budget.
        let cand = find_candidate(&func, 1).expect("should find a candidate");
        // The whole body is a tail-output run; the longest run that qualifies
        // includes the tail expr.
        assert_eq!(cand.fn_ident, "calc");
        // Inputs must include a and b.
        let in_names: Vec<&String> = cand.inputs.iter().map(|(n, _)| n).collect();
        assert!(in_names.contains(&&"a".to_string()));
        assert!(in_names.contains(&&"b".to_string()));
    }

    #[test]
    fn never_extracts_across_a_call() {
        // The only computation flows through a function call, whose result type
        // cannot be resolved, so no qualifying pure run exists. (A pure prefix
        // like `let s = a + 1` would itself be extractable — here there is none.)
        let func = parse_fn(
            "fn withcall(a: u32) -> u32 {\n\
             let t = helper(a);\n\
             t + 1\n\
             }",
        );
        assert!(
            find_candidate(&func, 1).is_none(),
            "no pure run should qualify when every value flows through a call"
        );
    }

    #[test]
    fn candidate_run_never_spans_a_call() {
        // A pure prefix exists (`let s = a + 1`) but the call statement must
        // never be inside the extracted run.
        let func = parse_fn(
            "fn mixed(a: u32) -> u32 {\n\
             let s = a + 1;\n\
             let t = helper(s);\n\
             t\n\
             }",
        );
        if let Some(cand) = find_candidate(&func, 1) {
            let (i, j) = cand.stmt_range;
            // The call is statement index 1; the run must not include it.
            assert!(!(i <= 1 && 1 < j), "run must not span the call statement");
        }
    }
}
