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
