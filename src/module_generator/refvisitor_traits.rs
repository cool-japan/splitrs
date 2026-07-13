//! # `RefVisitor` - Trait Implementations
//!
//! This module contains trait implementations for `RefVisitor`.
//!
//! ## Implemented Traits
//!
//! - `Visit`
//!
//! 🤖 Generated with [SplitRS](https://github.com/cool-japan/splitrs)

use syn::visit::{self, Visit};

use super::types::RefVisitor;

impl<'ast> Visit<'ast> for RefVisitor {
    fn visit_path(&mut self, path: &'ast syn::Path) {
        if let Some(first) = path.segments.first() {
            self.path_roots.insert(first.ident.to_string());
        }
        visit::visit_path(self, path);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.method_calls.insert(call.method.to_string());
        visit::visit_expr_method_call(self, call);
    }
    fn visit_macro(&mut self, mac: &'ast syn::Macro) {
        if let Some(first) = mac.path.segments.first() {
            self.path_roots.insert(first.ident.to_string());
        }
        // `write!(dst, ...)` / `writeln!(dst, ...)` expand to a hidden
        // `dst.write_fmt(format_args!(...))` call -- there is no literal
        // `.write_fmt(` text in the source for `visit_expr_method_call` to
        // see, so without this the `std::fmt::Write` (or `std::io::Write`)
        // import providing that method silently fails `should_keep_trait_import`
        // and gets pruned, producing `error[E0599]: cannot write into
        // ...; the trait ... is implemented but not in scope`. Record the
        // method name these macros invoke so the existing
        // `known_trait_methods("Write")` / `should_keep_trait_import` path
        // (already used for ordinary `.write_fmt(...)` calls) keeps the
        // import alive here too.
        if let Some(last) = mac.path.segments.last() {
            if last.ident == "write" || last.ident == "writeln" {
                self.method_calls.insert("write_fmt".to_string());
            }
        }
        let tokens = mac.tokens.clone();
        if let Ok(exprs) = syn::parse::Parser::parse2(
            syn::punctuated::Punctuated::<syn::Expr, syn::Token![,]>::parse_terminated,
            tokens.clone(),
        ) {
            for expr in &exprs {
                self.visit_expr(expr);
            }
        } else if let Ok(types) = syn::parse::Parser::parse2(
            syn::punctuated::Punctuated::<syn::Type, syn::Token![,]>::parse_terminated,
            tokens.clone(),
        ) {
            for ty in &types {
                self.visit_type(ty);
            }
        } else if let Ok(expr) = syn::parse2::<syn::Expr>(tokens.clone()) {
            self.visit_expr(&expr);
        } else if let Ok(ty) = syn::parse2::<syn::Type>(tokens) {
            self.visit_type(&ty);
        }
        visit::visit_macro(self, mac);
    }
    fn visit_attribute(&mut self, attr: &'ast syn::Attribute) {
        if let Some(ident) = attr.path().get_ident() {
            self.attr_idents.insert(ident.to_string());
        } else if let Some(first) = attr.path().segments.first() {
            self.attr_idents.insert(first.ident.to_string());
        }
        if attr.path().is_ident("derive") {
            let _ = attr.parse_nested_meta(|meta| {
                if let Some(first) = meta.path.segments.first() {
                    self.attr_idents.insert(first.ident.to_string());
                }
                Ok(())
            });
        }
    }
}
