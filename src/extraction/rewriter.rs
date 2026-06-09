//! Rewriter: synthesise the extracted helper `ItemFn` and the rewritten
//! enclosing function that calls it.
//!
//! Given a [`Candidate`] (a qualified statement run with typed in/out sets) we
//! produce two `syn::ItemFn` values:
//!
//! - the **helper** `fn <orig>_extracted(<inputs>) -> <Out> { <run>; <out-expr> }`
//! - the **rewritten** enclosing function with the run replaced by a single
//!   `let`-binding call to the helper (or, for a tail-output run, the tail
//!   expression replaced by the call).
//!
//! Codegen mirrors the `ItemFn` synthesis idiom in `src/method_analyzer.rs`
//! (assemble a `syn::ItemFn` from a signature + block) and uses
//! `syn::parse_quote!` / `quote` for the fragments.

use quote::format_ident;

use super::detector::Candidate;

/// The output of rewriting: the helper plus the rewritten enclosing function.
pub(crate) struct Rewrite {
    /// The synthesised helper function (pure, in-fragment).
    pub(crate) helper: syn::ItemFn,
    /// The enclosing function with the run replaced by a call to the helper.
    pub(crate) rewritten_fn: syn::ItemFn,
    /// The chosen helper identifier (collision-safe).
    pub(crate) helper_ident: String,
}

/// Build the helper + rewritten function for `cand` extracted from `func`.
///
/// `existing_idents` is the set of free-function names already present in the
/// file so we can pick a collision-free helper name. Returns `None` only if a
/// fragment fails to construct (defensive — inputs are pre-validated).
pub(crate) fn rewrite(
    func: &syn::ItemFn,
    cand: &Candidate,
    existing_idents: &[String],
) -> Option<Rewrite> {
    let (i, j) = cand.stmt_range;
    let stmts = &func.block.stmts;
    if i >= j || j > stmts.len() {
        return None;
    }

    let helper_ident = fresh_helper_ident(&cand.fn_ident, existing_idents);
    let helper_id = format_ident!("{}", helper_ident);

    // --- Helper signature pieces. ---
    let params = build_params(&cand.inputs);

    // --- Helper return type, return tail expr, and body statements. ---
    let (ret_ty, tail_expr, body_stmts) = build_helper_body(func, cand, &stmts[i..j])?;
    let helper_block = make_block(body_stmts, tail_expr);

    // Assemble the helper item via parse_quote (idiomatic, avoids hand-filling
    // every `syn::Signature` field).
    let helper: syn::ItemFn = syn::parse_quote! {
        fn #helper_id ( #params ) -> #ret_ty #helper_block
    };

    // --- Rewritten enclosing function. ---
    let call_args = build_call_args(&cand.inputs);
    let call_expr: syn::Expr = syn::parse_quote!( #helper_id ( #(#call_args),* ) );

    let mut new_stmts: Vec<syn::Stmt> = Vec::with_capacity(stmts.len());
    new_stmts.extend_from_slice(&stmts[0..i]);

    if cand.tail_is_output {
        // Replace the entire run with a single tail expression: the call.
        // (The run reached the function tail; its value is the function result.)
        new_stmts.push(syn::Stmt::Expr(call_expr, None));
    } else {
        // Bind the outputs from the call, then keep the post-run statements.
        let bind = build_output_binding(&cand.outputs, &call_expr)?;
        new_stmts.push(bind);
        new_stmts.extend_from_slice(&stmts[j..]);
    }

    let mut rewritten_fn = func.clone();
    rewritten_fn.block = Box::new(syn::Block {
        brace_token: func.block.brace_token,
        stmts: new_stmts,
    });

    Some(Rewrite {
        helper,
        rewritten_fn,
        helper_ident,
    })
}

/// Compute the helper's return type, return tail expression, and the body
/// statements (the run, minus any trailing tail expr that becomes the return).
///
/// Returns `(ret_ty, tail_expr, body_stmts)`.
fn build_helper_body(
    func: &syn::ItemFn,
    cand: &Candidate,
    run: &[syn::Stmt],
) -> Option<(syn::Type, syn::Expr, Vec<syn::Stmt>)> {
    if cand.tail_is_output {
        // The run already ends in a tail expression that IS the value. The
        // helper's return type is the enclosing function's return type.
        let ret_ty = match &func.sig.output {
            syn::ReturnType::Type(_, ty) => (**ty).clone(),
            syn::ReturnType::Default => return None,
        };
        // Split the run: everything but the last stmt is the body; the last
        // stmt (a tail expr) becomes the helper's tail expr.
        let (last, leading) = run.split_last()?;
        let tail_expr = match last {
            syn::Stmt::Expr(expr, None) => expr.clone(),
            _ => return None,
        };
        Some((ret_ty, tail_expr, leading.to_vec()))
    } else {
        // Named live-out outputs: the body is the whole run; the tail expr
        // returns the output(s); the return type is the single type or a tuple.
        let (ret_ty, tail_expr) = build_output_return(&cand.outputs)?;
        Some((ret_ty, tail_expr, run.to_vec()))
    }
}

/// For named outputs, build the helper return type and the tail return expr.
///
/// One output -> `T` and `out`. Many -> `(T1, T2, ...)` and `(out1, out2, ...)`.
fn build_output_return(outputs: &[(String, syn::Type)]) -> Option<(syn::Type, syn::Expr)> {
    match outputs.len() {
        0 => None, // a no-output non-tail run is rejected by the detector.
        1 => {
            let (name, ty) = &outputs[0];
            let id = format_ident!("{}", name);
            let expr: syn::Expr = syn::parse_quote!(#id);
            Some((ty.clone(), expr))
        }
        _ => {
            let types: Vec<&syn::Type> = outputs.iter().map(|(_, t)| t).collect();
            let names: Vec<syn::Ident> = outputs
                .iter()
                .map(|(n, _)| format_ident!("{}", n))
                .collect();
            let ret_ty: syn::Type = syn::parse_quote!( ( #(#types),* ) );
            let tail: syn::Expr = syn::parse_quote!( ( #(#names),* ) );
            Some((ret_ty, tail))
        }
    }
}

/// Build the `let (outputs) = call;` statement that replaces the run.
fn build_output_binding(
    outputs: &[(String, syn::Type)],
    call_expr: &syn::Expr,
) -> Option<syn::Stmt> {
    match outputs.len() {
        0 => None,
        1 => {
            let id = format_ident!("{}", outputs[0].0);
            Some(syn::parse_quote!( let #id = #call_expr; ))
        }
        _ => {
            let names: Vec<syn::Ident> = outputs
                .iter()
                .map(|(n, _)| format_ident!("{}", n))
                .collect();
            Some(syn::parse_quote!( let ( #(#names),* ) = #call_expr; ))
        }
    }
}

/// Build the helper's typed parameter list from the inputs.
fn build_params(
    inputs: &[(String, syn::Type)],
) -> syn::punctuated::Punctuated<syn::FnArg, syn::token::Comma> {
    let mut params = syn::punctuated::Punctuated::new();
    for (name, ty) in inputs {
        let id = format_ident!("{}", name);
        let arg: syn::FnArg = syn::parse_quote!( #id: #ty );
        params.push(arg);
    }
    params
}

/// Build the call-site argument expressions from the inputs (by name).
fn build_call_args(inputs: &[(String, syn::Type)]) -> Vec<syn::Expr> {
    inputs
        .iter()
        .map(|(name, _)| {
            let id = format_ident!("{}", name);
            syn::parse_quote!(#id)
        })
        .collect()
}

/// Assemble a `syn::Block` from leading statements plus a tail expression.
fn make_block(mut stmts: Vec<syn::Stmt>, tail: syn::Expr) -> syn::Block {
    stmts.push(syn::Stmt::Expr(tail, None));
    syn::Block {
        brace_token: Default::default(),
        stmts,
    }
}

/// Pick a collision-free helper name: `<orig>_extracted`, then `_2`, `_3`, …
fn fresh_helper_ident(orig: &str, existing: &[String]) -> String {
    let base = format!("{orig}_extracted");
    if !existing.iter().any(|e| e == &base) {
        return base;
    }
    let mut n = 2usize;
    loop {
        let candidate = format!("{base}_{n}");
        if !existing.iter().any(|e| e == &candidate) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_fn(src: &str) -> syn::ItemFn {
        syn::parse_str(src).expect("parse fn")
    }

    fn render(item: &syn::ItemFn) -> String {
        prettyplease::unparse(&syn::File {
            shebang: None,
            attrs: Vec::new(),
            items: vec![syn::Item::Fn(item.clone())],
        })
    }

    #[test]
    fn fresh_ident_avoids_collisions() {
        let existing = vec!["f_extracted".to_string()];
        assert_eq!(fresh_helper_ident("f", &existing), "f_extracted_2");
        assert_eq!(fresh_helper_ident("g", &existing), "g_extracted");
    }

    #[test]
    fn rewrites_single_output_run() {
        let func = parse_fn("fn calc(a: u32, b: u32) -> u32 { let s = a + b; let t = s * 2; t }");
        let cand = Candidate {
            fn_ident: "calc".to_string(),
            stmt_range: (0, 1),
            inputs: vec![
                ("a".to_string(), syn::parse_str("u32").expect("ty")),
                ("b".to_string(), syn::parse_str("u32").expect("ty")),
            ],
            outputs: vec![("s".to_string(), syn::parse_str("u32").expect("ty"))],
            tail_is_output: false,
        };
        let rw = rewrite(&func, &cand, &["calc".to_string()]).expect("rewrite");
        let helper_src = render(&rw.helper);
        assert!(helper_src.contains("fn calc_extracted"));
        assert!(helper_src.contains("-> u32"));
        let caller_src = render(&rw.rewritten_fn);
        assert!(caller_src.contains("calc_extracted(a, b)"));
    }

    #[test]
    fn rewrites_two_output_tuple() {
        let func = parse_fn("fn g(a: i32, b: i32) -> i32 { let p = a + b; let q = a - b; p * q }");
        let cand = Candidate {
            fn_ident: "g".to_string(),
            stmt_range: (0, 2),
            inputs: vec![
                ("a".to_string(), syn::parse_str("i32").expect("ty")),
                ("b".to_string(), syn::parse_str("i32").expect("ty")),
            ],
            outputs: vec![
                ("p".to_string(), syn::parse_str("i32").expect("ty")),
                ("q".to_string(), syn::parse_str("i32").expect("ty")),
            ],
            tail_is_output: false,
        };
        let rw = rewrite(&func, &cand, &["g".to_string()]).expect("rewrite");
        let helper_src = render(&rw.helper);
        assert!(helper_src.contains("-> (i32, i32)"));
        let caller_src = render(&rw.rewritten_fn);
        assert!(caller_src.contains("let (p, q) = g_extracted(a, b)"));
    }
}
