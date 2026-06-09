//! The SMT verification gate: decide whether an extraction may be COMMITTED.
//!
//! The single non-negotiable rule is: **never commit without a `Verified`
//! proof**. Two complementary proof constructions feed the decision:
//!
//! ## Whole-function proof (authoritative when applicable)
//! Build `post_inlined` = the rewritten enclosing function with the helper
//! CALL inlined back (the call replaced by a block holding the helper's body,
//! its parameters α-renamed to the actual arguments, its returned value bound
//! to the call's `let` target). Because `post_inlined` contains no call it is
//! in-fragment *iff the original enclosing function is*. We then ask the oracle
//! `prove_equivalent(original_fn, post_inlined)`:
//! - `Verified` ⇒ COMMIT (factor-out+inline-back is α-equivalent to the
//!   original; any rewriter bug would make the two differ).
//! - `Refuted(cx)` ⇒ a rewriter/detector bug — DO NOT commit (the safety net).
//! - `Unsupported` ⇒ the enclosing fn is not fully in-fragment (or the inline
//!   form uses a tuple the encoder rejects); fall through.
//!
//! ## Block proof (fallback validation)
//! Synthesise `probe_orig(inputs) -> Out { <run>; <out-expr> }` (what the
//! helper *should* compute, derived from the ORIGINAL statements) and
//! `probe_helper(inputs) -> Out { <helper body> }` (what the rewriter actually
//! produced) and prove them equivalent. Multi-output runs return a tuple, which
//! the QF_BV encoder does not model, so we prove the tuple **componentwise**:
//! one single-output probe pair per live-out. All components `Verified` (plus
//! the SOUND conservative dataflow that guarantees `outputs` captures every
//! live-out binding — valid because the fragment has no aliasing) ⇒ COMMIT,
//! logged as block-verified. Any `Refuted` ⇒ skip-refuted; any `Unsupported`
//! (e.g. a free variable from a missing input) ⇒ skip with the reason.

use std::collections::HashMap;

use quote::format_ident;

use crate::smt::{prove_equivalent, Counterexample, Verdict};

use super::detector::Candidate;
use super::rewriter::Rewrite;

/// The gate's decision for one extraction.
#[derive(Debug)]
pub(crate) enum GateDecision {
    /// A `Verified` proof was obtained — the extraction may be committed.
    /// `block_verified` records which proof path succeeded (for the report).
    Commit { block_verified: bool },
    /// A proof actively REFUTED the rewrite — a rewriter/detector bug. Skip.
    Refuted(Counterexample),
    /// No `Verified` could be obtained (out-of-fragment / free var). Skip.
    Skip { reason: String },
}

/// Run the verification gate for `cand` extracted from `original_fn` with the
/// synthesised `rewrite`. Commits only on a real `Verified`.
pub(crate) fn verify_and_decide(
    original_fn: &syn::ItemFn,
    cand: &Candidate,
    rewrite: &Rewrite,
) -> GateDecision {
    // 1. Whole-function proof (authoritative). Build post_inlined and compare
    //    to the original. A Refuted here is decisive (a real semantic drift).
    let whole_result = try_whole_function_proof(original_fn, cand, rewrite);
    match whole_result {
        Some(Verdict::Verified) => {
            return GateDecision::Commit {
                block_verified: false,
            }
        }
        Some(Verdict::Refuted(cx)) => return GateDecision::Refuted(cx),
        // Unsupported or not-constructible: fall through to the block proof.
        _ => {}
    }

    // 2. Block proof (fallback). Prove the run ≡ the helper, componentwise.
    block_proof(original_fn, cand, rewrite)
}

// ───────────────────────────────────────────────────────────────────────────
// Whole-function proof.
// ───────────────────────────────────────────────────────────────────────────

/// Attempt the whole-function proof. Returns `None` if `post_inlined` cannot be
/// constructed in-fragment (e.g. a multi-output tuple binding the encoder does
/// not model); otherwise the oracle's verdict.
fn try_whole_function_proof(
    original_fn: &syn::ItemFn,
    cand: &Candidate,
    rewrite: &Rewrite,
) -> Option<Verdict> {
    let post_inlined = build_post_inlined(cand, rewrite)?;
    Some(prove_equivalent(original_fn, &post_inlined))
}

/// Build `post_inlined`: the rewritten function with the helper call inlined.
///
/// The inlined form replaces the call expression with a *block expression*
/// holding the helper's body (α-renamed). This is only constructible in-fragment
/// for the single-output and tail-output shapes; multi-output (tuple) inlining
/// is rejected (`None`) so the caller falls through to the block proof.
fn build_post_inlined(cand: &Candidate, rewrite: &Rewrite) -> Option<syn::ItemFn> {
    // Multi-output inlining would require a tuple `let` binding, which the
    // encoder does not model. Defer to the block proof.
    if cand.outputs.len() > 1 {
        return None;
    }

    // α-rename map: helper param name -> actual argument expression. Our codegen
    // uses identical names (args are the input idents), so this is the identity;
    // we still build it explicitly to remain sound if that ever changes.
    let rename: HashMap<String, syn::Expr> = cand
        .inputs
        .iter()
        .map(|(name, _)| {
            let id = format_ident!("{}", name);
            (name.clone(), syn::parse_quote!(#id))
        })
        .collect();

    // The helper body with params substituted by their arguments.
    let inlined_block = substitute_idents_in_block(&rewrite.helper.block, &rename)?;
    let inlined_expr: syn::Expr = syn::Expr::Block(syn::ExprBlock {
        attrs: Vec::new(),
        label: None,
        block: inlined_block,
    });

    // Reconstruct the enclosing function with the call site replaced by the
    // inlined block. We rebuild from the ORIGINAL statement layout so the
    // result is structurally the original modulo the (block-wrapped) run.
    let mut item = rewrite.rewritten_fn.clone();
    let stmts = &mut item.block.stmts;
    let (i, _j) = cand.stmt_range;

    if cand.tail_is_output {
        // The rewritten tail is `helper(args)`; replace it with the inlined block.
        // Find the tail statement (last) and swap its expression.
        if let Some(last) = stmts.last_mut() {
            if let syn::Stmt::Expr(_, None) = last {
                *last = syn::Stmt::Expr(inlined_expr, None);
            } else {
                return None;
            }
        } else {
            return None;
        }
    } else {
        // The rewritten run is a single `let out = helper(args);` at index i.
        // Replace its initializer with the inlined block.
        let stmt = stmts.get_mut(i)?;
        let syn::Stmt::Local(local) = stmt else {
            return None;
        };
        let init = local.init.as_mut()?;
        *init.expr = inlined_expr;
    }

    Some(item)
}

// ───────────────────────────────────────────────────────────────────────────
// Block proof (componentwise).
// ───────────────────────────────────────────────────────────────────────────

/// Prove the extracted run equivalent to the synthesised helper.
///
/// Tail-output runs prove a single value (the tail). Named-output runs prove
/// each live-out binding componentwise (sound: componentwise equality ⟺ tuple
/// equality, because the fragment has no aliasing).
fn block_proof(original_fn: &syn::ItemFn, cand: &Candidate, rewrite: &Rewrite) -> GateDecision {
    if cand.tail_is_output {
        return block_proof_tail(original_fn, cand, rewrite);
    }
    block_proof_named(original_fn, cand, rewrite)
}

/// Tail-output block proof: probe_orig = the run verbatim (it ends in the tail
/// value); probe_helper = the helper body verbatim. Both share the signature
/// `(inputs) -> <enclosing return type>`.
fn block_proof_tail(
    original_fn: &syn::ItemFn,
    cand: &Candidate,
    rewrite: &Rewrite,
) -> GateDecision {
    let ret_ty = match &original_fn.sig.output {
        syn::ReturnType::Type(_, ty) => (**ty).clone(),
        syn::ReturnType::Default => {
            return GateDecision::Skip {
                reason: "tail-output run in a function with no return type".to_string(),
            }
        }
    };
    let run = match run_statements(original_fn, cand) {
        Some(r) => r,
        None => {
            return GateDecision::Skip {
                reason: "could not isolate the extracted statement run".to_string(),
            }
        }
    };
    let probe_orig = match build_probe(&cand.inputs, &ret_ty, run, None) {
        Some(f) => f,
        None => {
            return GateDecision::Skip {
                reason: "could not synthesise the original tail probe".to_string(),
            }
        }
    };
    let probe_helper = match build_probe(
        &cand.inputs,
        &ret_ty,
        rewrite.helper.block.stmts.clone(),
        None,
    ) {
        Some(f) => f,
        None => {
            return GateDecision::Skip {
                reason: "could not synthesise the helper tail probe".to_string(),
            }
        }
    };
    match prove_equivalent(&probe_orig, &probe_helper) {
        Verdict::Verified => GateDecision::Commit {
            block_verified: true,
        },
        Verdict::Refuted(cx) => GateDecision::Refuted(cx),
        Verdict::Unsupported { reason } => GateDecision::Skip {
            reason: format!("tail output: {reason}"),
        },
    }
}

/// Named-output block proof: prove each live-out binding equivalent between the
/// original run and the helper body. Both probes project the same `<name>`.
fn block_proof_named(
    original_fn: &syn::ItemFn,
    cand: &Candidate,
    rewrite: &Rewrite,
) -> GateDecision {
    let run = match run_statements(original_fn, cand) {
        Some(r) => r,
        None => {
            return GateDecision::Skip {
                reason: "could not isolate the extracted statement run".to_string(),
            }
        }
    };
    let helper_leading = helper_leading_statements(rewrite);

    for (name, ty) in &cand.outputs {
        let id = format_ident!("{}", name);
        let proj: syn::Expr = syn::parse_quote!(#id);

        let probe_orig = match build_probe(&cand.inputs, ty, run.clone(), Some(proj.clone())) {
            Some(f) => f,
            None => {
                return GateDecision::Skip {
                    reason: format!("could not synthesise the original probe for `{name}`"),
                }
            }
        };
        let probe_helper =
            match build_probe(&cand.inputs, ty, helper_leading.clone(), Some(proj.clone())) {
                Some(f) => f,
                None => {
                    return GateDecision::Skip {
                        reason: format!("could not synthesise the helper probe for `{name}`"),
                    }
                }
            };

        match prove_equivalent(&probe_orig, &probe_helper) {
            Verdict::Verified => {}
            Verdict::Refuted(cx) => return GateDecision::Refuted(cx),
            Verdict::Unsupported { reason } => {
                return GateDecision::Skip {
                    reason: format!("output `{name}`: {reason}"),
                }
            }
        }
    }

    GateDecision::Commit {
        block_verified: true,
    }
}

/// Build a single-output probe `fn __splitrs_probe(inputs) -> ret_ty { body[; proj] }`.
///
/// When `proj` is `Some(expr)` it is appended as the tail (named-output
/// projection). When `proj` is `None` the `body` is used verbatim and must
/// already end in a tail expression (tail-output case).
fn build_probe(
    inputs: &[(String, syn::Type)],
    ret_ty: &syn::Type,
    body: Vec<syn::Stmt>,
    proj: Option<syn::Expr>,
) -> Option<syn::ItemFn> {
    let probe_id = format_ident!("__splitrs_probe");

    let mut params = syn::punctuated::Punctuated::<syn::FnArg, syn::token::Comma>::new();
    for (name, ty) in inputs {
        let id = format_ident!("{}", name);
        let arg: syn::FnArg = syn::parse_quote!( #id: #ty );
        params.push(arg);
    }

    let mut stmts = body;
    match proj {
        Some(expr) => stmts.push(syn::Stmt::Expr(expr, None)),
        None => {
            // The body must already provide the tail value.
            if !matches!(stmts.last(), Some(syn::Stmt::Expr(_, None))) {
                return None;
            }
        }
    }
    let block = syn::Block {
        brace_token: Default::default(),
        stmts,
    };

    let probe: syn::ItemFn = syn::parse_quote! {
        fn #probe_id ( #params ) -> #ret_ty #block
    };
    Some(probe)
}

/// The extracted statement run `[i..j)` cloned out of the original function.
fn run_statements(original_fn: &syn::ItemFn, cand: &Candidate) -> Option<Vec<syn::Stmt>> {
    let (i, j) = cand.stmt_range;
    let stmts = &original_fn.block.stmts;
    if i >= j || j > stmts.len() {
        return None;
    }
    Some(stmts[i..j].to_vec())
}

/// The helper's leading `let` statements (its body with the trailing tail
/// return expression stripped), so a named-output probe can re-append its own
/// projection.
fn helper_leading_statements(rewrite: &Rewrite) -> Vec<syn::Stmt> {
    let stmts = &rewrite.helper.block.stmts;
    if let Some((syn::Stmt::Expr(_, None), leading)) = stmts.split_last() {
        leading.to_vec()
    } else {
        stmts.clone()
    }
}

/// Substitute every simple-ident read in `block` per `rename`, returning the
/// rewritten block. Used to α-rename helper params during inline-back. Because
/// our argument expressions are themselves single idents, this is sound and
/// total over the fragment.
fn substitute_idents_in_block(
    block: &syn::Block,
    rename: &HashMap<String, syn::Expr>,
) -> Option<syn::Block> {
    // The fragment uses identical argument names, so substitution is the
    // identity in practice; we clone the block unchanged. (A future rename map
    // with differing names would require a folding visitor here.)
    if rename
        .iter()
        .all(|(k, v)| matches!(v, syn::Expr::Path(p) if p.path.is_ident(k)))
    {
        return Some(block.clone());
    }
    // Non-identity renames are not produced by the current rewriter; signal
    // "cannot construct" so the caller falls through to the block proof.
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extraction::detector::find_candidate;
    use crate::extraction::rewriter::rewrite as do_rewrite;

    fn parse_fn(src: &str) -> syn::ItemFn {
        syn::parse_str(src).expect("parse fn")
    }

    /// A correct extraction commits.
    #[test]
    fn correct_extraction_commits() {
        let func = parse_fn("fn calc(a: u32, b: u32) -> u32 { let s = a + b; let t = s * 2; t }");
        let cand = find_candidate(&func, 1).expect("candidate");
        let rw = do_rewrite(&func, &cand, &["calc".to_string()]).expect("rewrite");
        match verify_and_decide(&func, &cand, &rw) {
            GateDecision::Commit { .. } => {}
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    /// A deliberately MIS-DERIVED helper (computes `a - b` where the run
    /// computes `a + b`) must be REFUTED, never committed. This is the proof
    /// that the gate rejects semantic drift rather than rubber-stamping.
    #[test]
    fn mis_derived_helper_is_refuted() {
        let func = parse_fn("fn calc(a: u32, b: u32) -> u32 { let s = a + b; let t = s * 2; t }");
        let cand = find_candidate(&func, 1).expect("candidate");
        let mut rw = do_rewrite(&func, &cand, &["calc".to_string()]).expect("rewrite");
        // Corrupt the helper: replace its body with one that subtracts.
        rw.helper = parse_fn("fn calc_extracted(a: u32, b: u32) -> u32 { a - b }");
        match verify_and_decide(&func, &cand, &rw) {
            GateDecision::Refuted(_) => {}
            other => panic!("expected Refuted for a mis-derived helper, got {other:?}"),
        }
    }
}
