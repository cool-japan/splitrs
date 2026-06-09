//! Live-in / live-out dataflow analysis over a contiguous statement run.
//!
//! For a run `[i..j)` of top-level statements inside a function body we compute:
//!
//! - `defs` — identifiers bound by a `let` *inside* the run.
//! - `inputs` — identifiers READ inside the run that resolve to a binding
//!   introduced BEFORE the run (a parameter or an earlier `let`). Each input
//!   carries its `syn::Type`.
//! - `outputs` — identifiers in `defs` that are READ in statements AFTER `j`
//!   (i.e. they are live-out of the run). Each output carries its `syn::Type`.
//!
//! Soundness note: the SMT-supported fragment has NO references, aliasing,
//! macros, closures, or pointer indirection. Every use of a value is therefore
//! a *syntactic* identifier read, and every binding is a `let`. A purely
//! syntactic read-after-run analysis is consequently SOUND for the fragment we
//! gate on — there is no hidden mutation or aliasing path by which a binding
//! could be live-out without appearing as a named read after the run.
//!
//! This mirrors the `FieldAccessVisitor` idiom in
//! `src/field_access_tracker.rs` (a `syn::visit::Visit` walker that collects
//! identifiers), specialised here to read-idents and `let`-bound idents.

use std::collections::{HashMap, HashSet};

use syn::visit::Visit;

/// The result of analysing a statement run for its data dependencies.
///
/// (No `Debug` derive: `syn::Type` only implements `Debug` under syn's
/// `extra-traits` feature, which we do not enable.)
#[derive(Clone, Default)]
pub(crate) struct RunDataflow {
    /// Identifiers bound by a `let` inside the run, in first-seen order.
    pub(crate) defs: Vec<String>,
    /// Live-in identifiers (read in the run, bound before it) with their types.
    pub(crate) inputs: Vec<(String, syn::Type)>,
    /// Live-out identifiers (defined in the run, read after it) with their types.
    pub(crate) outputs: Vec<(String, syn::Type)>,
}

/// A map from a binding name to the `syn::Type` we resolved for it.
type TypeEnv = HashMap<String, syn::Type>;

/// Compute the dataflow facts for the run `stmts[i..j)` inside `func`.
///
/// `func` is the enclosing function (we need its parameter signature to type
/// the inputs). `i`/`j` index `func.block.stmts`. Returns `None` when a
/// required type cannot be determined (an input or output whose type we cannot
/// resolve from a parameter, a `let` annotation, or a simple inference) — the
/// detector treats that as a disqualifying condition.
pub(crate) fn analyze_run(func: &syn::ItemFn, i: usize, j: usize) -> Option<RunDataflow> {
    let stmts = &func.block.stmts;
    if i >= j || j > stmts.len() {
        return None;
    }

    // 1. Build the type environment of everything bound BEFORE the run:
    //    parameters first, then `let`s in stmts[0..i]. Also record the full SET
    //    of pre-run binding NAMES (even ones whose type we could not resolve) so
    //    a read that resolves to an untyped pre-run binding rejects the run
    //    rather than silently becoming a dangling free variable in the helper.
    let mut pre_env: TypeEnv = HashMap::new();
    collect_param_types(&func.sig, &mut pre_env);
    collect_let_types(&stmts[0..i], &mut pre_env);
    let pre_names = collect_pre_binding_names(&func.sig, &stmts[0..i]);

    // 2. Collect the `let`-bound names introduced inside the run, plus their
    //    resolved types (annotation or inference against the running env).
    let mut run_env = pre_env.clone();
    let mut defs: Vec<String> = Vec::new();
    let mut def_types: TypeEnv = HashMap::new();
    for stmt in &stmts[i..j] {
        if let syn::Stmt::Local(local) = stmt {
            if let Some((name, ty)) = let_binding(local, &run_env) {
                if !defs.contains(&name) {
                    defs.push(name.clone());
                }
                if let Some(ty) = ty.clone() {
                    def_types.insert(name.clone(), ty.clone());
                    run_env.insert(name.clone(), ty);
                }
                // A `let` whose type we could not resolve is still recorded as a
                // def (so we know the name is bound here), but without a type;
                // if it later becomes an output the caller will reject the run.
                let _ = name;
            }
        }
    }

    // 3. Collect every identifier READ inside the run.
    let read_in_run = reads_in_stmts(&stmts[i..j]);

    // 4. Inputs = reads in the run that resolve to a binding BEFORE the run
    //    (present in pre_env) and are NOT shadowed by a def in the run before
    //    first use. Because the fragment is SSA-like (no reassignment) and we
    //    reject runs that re-`let` an input name, a name being both in pre_env
    //    and a def is treated conservatively as a fresh def (not an input).
    let defs_set: HashSet<&String> = defs.iter().collect();
    let mut inputs: Vec<(String, syn::Type)> = Vec::new();
    let mut seen_input: HashSet<String> = HashSet::new();
    for name in &read_in_run {
        if defs_set.contains(name) {
            continue; // bound inside the run -> not a live-in
        }
        if let Some(ty) = pre_env.get(name) {
            if seen_input.insert(name.clone()) {
                inputs.push((name.clone(), ty.clone()));
            }
        } else if pre_names.contains(name) {
            // The read resolves to a pre-run binding whose type we could NOT
            // resolve (e.g. it was initialised by a call). It would become an
            // untyped helper parameter / dangling reference — reject the run.
            return None;
        }
        // Otherwise it is a constant/global path we do not model as an input;
        // the detector's purity probe accepts only in-fragment statements, so a
        // genuine free variable cannot reach here for a qualifying run.
    }

    // 5. Outputs = defs read in stmts AFTER the run (or never, if not live-out).
    let read_after = reads_in_stmts(&stmts[j..]);
    let mut outputs: Vec<(String, syn::Type)> = Vec::new();
    let mut seen_output: HashSet<String> = HashSet::new();
    for name in &defs {
        if read_after.contains(name) {
            // A live-out binding MUST have a resolvable type or the run is
            // rejected (return None) — we cannot synthesise a typed return.
            let ty = def_types.get(name)?;
            if seen_output.insert(name.clone()) {
                outputs.push((name.clone(), ty.clone()));
            }
        }
    }

    // Stable ordering for deterministic codegen: inputs by first read, outputs
    // by definition order (already the case above).
    Some(RunDataflow {
        defs,
        inputs,
        outputs,
    })
}

/// Seed `env` with each typed parameter `name: Type` (plain-ident params only).
fn collect_param_types(sig: &syn::Signature, env: &mut TypeEnv) {
    for input in &sig.inputs {
        if let syn::FnArg::Typed(pat_type) = input {
            if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                if pat_ident.by_ref.is_none() && pat_ident.subpat.is_none() {
                    env.insert(pat_ident.ident.to_string(), (*pat_type.ty).clone());
                }
            }
        }
    }
}

/// Fold the `let` bindings in `stmts` into `env`, resolving each binding's type
/// from its annotation or by a simple inference against the current env.
fn collect_let_types(stmts: &[syn::Stmt], env: &mut TypeEnv) {
    for stmt in stmts {
        if let syn::Stmt::Local(local) = stmt {
            if let Some((name, Some(ty))) = let_binding(local, env) {
                env.insert(name, ty);
            }
        }
    }
}

/// Collect the names of every binding introduced BEFORE the run: each typed
/// parameter and each plain-ident `let` in the leading statements — regardless
/// of whether the binding's type could be resolved.
fn collect_pre_binding_names(sig: &syn::Signature, leading: &[syn::Stmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for input in &sig.inputs {
        if let syn::FnArg::Typed(pat_type) = input {
            if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                names.insert(pat_ident.ident.to_string());
            }
        }
    }
    for stmt in leading {
        if let syn::Stmt::Local(local) = stmt {
            match &local.pat {
                syn::Pat::Ident(pat_ident) => {
                    names.insert(pat_ident.ident.to_string());
                }
                syn::Pat::Type(pat_type) => {
                    if let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() {
                        names.insert(pat_ident.ident.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    names
}

/// Extract a `let` binding's plain-ident name and resolved type.
///
/// Resolution order: an explicit `: T` annotation wins; otherwise we attempt a
/// minimal type inference from the initializer against `env` (enough for the
/// straight-line integer fragment). Returns `(name, Some(type))` when a type is
/// known, `(name, None)` when the name is bound but its type is unknown.
/// Returns `None` only when the pattern is not a plain identifier.
fn let_binding(local: &syn::Local, env: &TypeEnv) -> Option<(String, Option<syn::Type>)> {
    // Plain `let x` or annotated `let x: T`.
    let (name, annotated) = match &local.pat {
        syn::Pat::Ident(pat_ident) => {
            if pat_ident.by_ref.is_some() || pat_ident.subpat.is_some() {
                return None;
            }
            (pat_ident.ident.to_string(), None)
        }
        syn::Pat::Type(pat_type) => {
            let syn::Pat::Ident(pat_ident) = pat_type.pat.as_ref() else {
                return None;
            };
            if pat_ident.by_ref.is_some() || pat_ident.subpat.is_some() {
                return None;
            }
            (pat_ident.ident.to_string(), Some((*pat_type.ty).clone()))
        }
        _ => return None,
    };

    if annotated.is_some() {
        return Some((name, annotated));
    }
    // No annotation: try to infer from the initializer.
    let inferred = local
        .init
        .as_ref()
        .and_then(|init| infer_expr_type(&init.expr, env));
    Some((name, inferred))
}

/// A minimal best-effort type inference for the pure-integer fragment.
///
/// This deliberately covers only the cases the SMT fragment models, so that the
/// type we attach to a binding is the same width the encoder will use:
/// - a literal with an integer suffix (`5u32`) -> that type;
/// - a bare identifier -> its env type;
/// - `(e)` / `{ e }` -> the inner type;
/// - a cast `e as T` -> `T`;
/// - a binary arithmetic/bitwise op -> the type of whichever operand resolves
///   (operands share a type in the fragment); comparisons -> `bool`;
/// - unary `-e` / `!e` -> the operand's type;
/// - an `if/else` -> the then-arm's type.
///
/// Anything else yields `None` (unknown), which only disqualifies the run if
/// the binding turns out to be a live-out output.
fn infer_expr_type(expr: &syn::Expr, env: &TypeEnv) -> Option<syn::Type> {
    match expr {
        syn::Expr::Paren(p) => infer_expr_type(&p.expr, env),
        syn::Expr::Group(g) => infer_expr_type(&g.expr, env),
        syn::Expr::Path(path) => {
            let ident = path.path.get_ident()?;
            env.get(&ident.to_string()).cloned()
        }
        syn::Expr::Lit(lit) => match &lit.lit {
            syn::Lit::Int(int_lit) => {
                let suffix = int_lit.suffix();
                if suffix.is_empty() {
                    None
                } else {
                    syn::parse_str::<syn::Type>(suffix).ok()
                }
            }
            syn::Lit::Bool(_) => syn::parse_str::<syn::Type>("bool").ok(),
            _ => None,
        },
        syn::Expr::Cast(cast) => Some((*cast.ty).clone()),
        syn::Expr::Unary(un) => match un.op {
            syn::UnOp::Not(_) | syn::UnOp::Neg(_) => infer_expr_type(&un.expr, env),
            _ => None,
        },
        syn::Expr::Binary(bin) => {
            use syn::BinOp;
            // Comparisons / logical -> bool.
            if matches!(
                bin.op,
                BinOp::Eq(_)
                    | BinOp::Ne(_)
                    | BinOp::Lt(_)
                    | BinOp::Le(_)
                    | BinOp::Gt(_)
                    | BinOp::Ge(_)
                    | BinOp::And(_)
                    | BinOp::Or(_)
            ) {
                return syn::parse_str::<syn::Type>("bool").ok();
            }
            // Arithmetic/bitwise: operands share a type; take whichever resolves.
            infer_expr_type(&bin.left, env).or_else(|| infer_expr_type(&bin.right, env))
        }
        syn::Expr::If(if_expr) => infer_block_tail_type(&if_expr.then_branch, env),
        syn::Expr::Block(block) => infer_block_tail_type(&block.block, env),
        _ => None,
    }
}

/// Infer the type of a block's tail expression (folding its leading `let`s into
/// a temporary env so the tail can reference them).
fn infer_block_tail_type(block: &syn::Block, env: &TypeEnv) -> Option<syn::Type> {
    let mut local_env = env.clone();
    let (last, leading) = block.stmts.split_last()?;
    collect_let_types(leading, &mut local_env);
    match last {
        syn::Stmt::Expr(expr, _) => infer_expr_type(expr, &local_env),
        _ => None,
    }
}

/// Collect, in first-seen order, every identifier READ across `stmts`.
///
/// "Read" means an identifier appearing in expression position (a `syn::Path`
/// that is a single ident). We explicitly do NOT count the binding occurrence
/// on the left of a `let` as a read (that is handled by visiting only the
/// initializer of a `Local`). For the pure fragment this captures exactly the
/// value uses.
fn reads_in_stmts(stmts: &[syn::Stmt]) -> Vec<String> {
    let mut visitor = ReadVisitor::default();
    for stmt in stmts {
        visitor.visit_stmt(stmt);
    }
    visitor.reads
}

/// A `syn::visit::Visit` walker collecting single-ident path reads in order,
/// deduplicated, and skipping the binding side of `let` patterns.
#[derive(Default)]
struct ReadVisitor {
    reads: Vec<String>,
    seen: HashSet<String>,
}

impl ReadVisitor {
    fn record(&mut self, name: String) {
        if self.seen.insert(name.clone()) {
            self.reads.push(name);
        }
    }
}

impl<'ast> Visit<'ast> for ReadVisitor {
    fn visit_local(&mut self, local: &'ast syn::Local) {
        // Visit ONLY the initializer expression — the pattern on the left binds
        // a name (a write), not a read. (`let ... else` divergent blocks are
        // outside the fragment, but visiting them is harmless.)
        if let Some(init) = &local.init {
            self.visit_expr(&init.expr);
            if let Some((_, _, diverge)) = init.diverge.as_ref().map(|d| ((), (), d.1.as_ref())) {
                self.visit_expr(diverge);
            }
        }
    }

    fn visit_expr_path(&mut self, path: &'ast syn::ExprPath) {
        if let Some(ident) = path.path.get_ident() {
            self.record(ident.to_string());
        }
        syn::visit::visit_expr_path(self, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a function and analyze the run `[i..j)`.
    fn analyze(src: &str, i: usize, j: usize) -> RunDataflow {
        let func: syn::ItemFn = syn::parse_str(src).expect("parse fn");
        analyze_run(&func, i, j).expect("dataflow should resolve")
    }

    fn names(pairs: &[(String, syn::Type)]) -> Vec<String> {
        pairs.iter().map(|(n, _)| n.clone()).collect()
    }

    #[test]
    fn inputs_are_pre_run_reads() {
        // params a,b live-in; the run [0..1) is `let s = a + b;`
        let df = analyze(
            "fn f(a: u32, b: u32) -> u32 { let s = a + b; let t = s * 2; t }",
            0,
            1,
        );
        assert_eq!(df.defs, vec!["s".to_string()]);
        let mut got = names(&df.inputs);
        got.sort();
        assert_eq!(got, vec!["a".to_string(), "b".to_string()]);
        // `s` is read after the run (in `let t = s * 2`) -> live-out.
        assert_eq!(names(&df.outputs), vec!["s".to_string()]);
    }

    #[test]
    fn multiple_outputs_tracked() {
        // run [0..2) defines p and q, both read in the tail -> two outputs.
        let df = analyze(
            "fn g(a: i32, b: i32) -> i32 { let p = a + b; let q = a - b; p * q }",
            0,
            2,
        );
        let mut outs = names(&df.outputs);
        outs.sort();
        assert_eq!(outs, vec!["p".to_string(), "q".to_string()]);
        // inputs are a and b.
        let mut ins = names(&df.inputs);
        ins.sort();
        assert_eq!(ins, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn dead_binding_is_not_an_output() {
        // `dead` is defined in the run but never read afterwards -> not output.
        let df = analyze(
            "fn h(a: u32) -> u32 { let dead = a + 1; let live = a * 3; live }",
            0,
            1,
        );
        // run [0..1) only defines `dead`, which is never read after -> no outputs.
        assert!(df.outputs.is_empty(), "dead binding must not be live-out");
        assert_eq!(df.defs, vec!["dead".to_string()]);
    }

    #[test]
    fn type_inference_resolves_let_output_type() {
        // `s` has no annotation; its type is inferred from `a + b` (u32).
        let df = analyze("fn f(a: u32, b: u32) -> u32 { let s = a + b; s + 1 }", 0, 1);
        assert_eq!(df.outputs.len(), 1);
        let (name, ty) = &df.outputs[0];
        assert_eq!(name, "s");
        let rendered = quote::quote!(#ty).to_string();
        assert_eq!(rendered, "u32");
    }
}
