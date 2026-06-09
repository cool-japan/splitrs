//! Integration tests for cross-module visibility elevation
//! (v0.3.3 — fix for the picking.rs E0425 regression).
//!
//! When splitrs buckets items into sibling generated modules, references
//! between those buckets must continue to resolve:
//!
//! 1. **Visibility elevation** — private items moved to a sibling module
//!    that are referenced from another generated module must be promoted
//!    from `fn`/`const`/... to `pub(super) fn`/... so the sibling can see
//!    them through the module wall.
//!
//! 2. **Use-statement injection** — the module doing the referencing must
//!    gain a `use super::<other_mod>::<name>;` (or grouped equivalent).
//!
//! Pre-0.3.3, `compute_cross_module_visibility` only iterated
//! `module.standalone_items` when collecting referenced helper names, so
//! methods bundled inside `module.types[*].impls` (the picking.rs case —
//! `impl PickingRay { fn distance_to_point(&self) { cross(...); ... } }`)
//! never registered their calls into private free functions like `cross`,
//! and the helpers stayed private. The generated `cargo build` would
//! then fail with `error[E0425]: cannot find function 'cross'`.
//!
//! These tests exercise that exact pattern, plus several edge cases.

use splitrs::file_analyzer::FileAnalyzer;
use splitrs::module_generator::generate_tests_rs_with_uses;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a fixture and analyze it with the given target_modules rules.
fn analyze(code: &str) -> (syn::File, FileAnalyzer) {
    let file = syn::parse_file(code).expect("test fixture failed to parse as Rust");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.analyze(&file);
    (file, analyzer)
}

/// Compute the rendered text of every module, keyed by module name.
fn render_modules(file: &syn::File, analyzer: &FileAnalyzer) -> HashMap<String, String> {
    let modules = analyzer.group_by_module(500);

    // Build type_to_module map exactly like main.rs does.
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for m in &modules {
        for t in m.get_exported_types() {
            type_to_module.insert(t, m.name.clone());
        }
    }

    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    modules
        .iter()
        .map(|m| {
            let content = m.generate_content(
                file,
                &analyzer.use_statements,
                &type_to_module,
                &needs_pub_super,
                cross_module_imports.get(&m.name),
                &fields_need_pub_super,
                Some(&analyzer.trait_tracker),
            );
            (m.name.clone(), content)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Simple case: impl method calls private helper in sibling module
// ---------------------------------------------------------------------------
//
// Reproduces the picking.rs failure: `impl Foo { fn uses_helper(&self) { helper(); } }`
// lives in `types.rs` while `fn helper()` lives in `functions.rs`. Both must
// resolve: helper goes pub(super), and types.rs imports it.

#[test]
fn impl_method_to_sibling_helper_elevates_and_imports() {
    let code = r#"
        pub struct Foo {
            pub value: i32,
        }

        impl Foo {
            pub fn use_helper(&self) -> i32 {
                helper(self.value)
            }
        }

        fn helper(x: i32) -> i32 {
            x * 2
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");
    let types_rs = rendered.get("types").expect("expected `types` module");

    assert!(
        functions_rs.contains("pub(super) fn helper"),
        "helper must be elevated to pub(super); got:\n{}",
        functions_rs
    );
    assert!(
        types_rs.contains("use super::functions::helper"),
        "types.rs must import `helper`; got:\n{}",
        types_rs
    );
}

// ---------------------------------------------------------------------------
// 2. Already-pub helper: visibility must NOT be downgraded
// ---------------------------------------------------------------------------

#[test]
fn already_pub_helper_stays_pub() {
    let code = r#"
        pub struct Foo;

        impl Foo {
            pub fn use_helper(&self) -> i32 {
                helper()
            }
        }

        pub fn helper() -> i32 {
            42
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");

    // Must remain `pub fn helper`, not `pub(super) fn helper` (which would
    // narrow the helper's visibility and is the wrong correction).
    assert!(
        functions_rs.contains("pub fn helper") && !functions_rs.contains("pub(super) fn helper"),
        "already-pub helper must not be downgraded; got:\n{}",
        functions_rs
    );
}

// ---------------------------------------------------------------------------
// 3. No cross-module refs: no spurious `use super::*` injected
// ---------------------------------------------------------------------------

#[test]
fn no_cross_refs_means_no_super_imports() {
    let code = r#"
        pub struct Alpha {
            pub a: i32,
        }
        pub struct Beta {
            pub b: i32,
        }

        impl Alpha {
            pub fn ping(&self) -> i32 {
                self.a
            }
        }
        impl Beta {
            pub fn pong(&self) -> i32 {
                self.b
            }
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    // No functions module should exist (no standalone fns), and types.rs
    // should not pull in `use super::functions::*` since there are no
    // sibling refs.
    if let Some(types_rs) = rendered.get("types") {
        assert!(
            !types_rs.contains("use super::functions"),
            "types.rs must not import from non-existent `functions` module; got:\n{}",
            types_rs
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Multiple cross refs: grouped (brace) import
// ---------------------------------------------------------------------------

#[test]
fn multiple_cross_refs_use_grouped_import() {
    let code = r#"
        pub struct Calc {
            pub state: i32,
        }

        impl Calc {
            pub fn run(&self) -> i32 {
                helper_one(self.state) + helper_two(self.state) + helper_three(self.state)
            }
        }

        fn helper_one(x: i32) -> i32 { x + 1 }
        fn helper_two(x: i32) -> i32 { x + 2 }
        fn helper_three(x: i32) -> i32 { x + 3 }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let types_rs = rendered.get("types").expect("expected `types` module");
    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");

    // All three helpers should be elevated.
    for name in ["helper_one", "helper_two", "helper_three"] {
        assert!(
            functions_rs.contains(&format!("pub(super) fn {}", name)),
            "{} must be elevated to pub(super); got:\n{}",
            name,
            functions_rs
        );
    }

    // The import should be a single brace-grouped statement.
    let has_grouped = types_rs.contains("use super::functions::{")
        && types_rs.contains("helper_one")
        && types_rs.contains("helper_two")
        && types_rs.contains("helper_three");
    assert!(
        has_grouped,
        "types.rs should use a single grouped import; got:\n{}",
        types_rs
    );
}

// ---------------------------------------------------------------------------
// 5. Cross-ref to a sibling type via a free function: type goes pub
// ---------------------------------------------------------------------------
//
// When a free function in `functions.rs` references a struct in `types.rs`,
// the existing super:: imports infrastructure handles it (since structs are
// already `pub`). We verify the import flows correctly.

#[test]
fn function_referencing_sibling_type_gets_import() {
    let code = r#"
        pub struct Widget {
            pub id: u32,
        }

        impl Widget {
            pub fn new(id: u32) -> Self {
                Self { id }
            }
        }

        pub fn make_widget(id: u32) -> Widget {
            Widget::new(id)
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");

    assert!(
        functions_rs.contains("use super::types::Widget")
            || functions_rs.contains("use super::types::{") && functions_rs.contains("Widget"),
        "functions.rs must import Widget; got:\n{}",
        functions_rs
    );
}

// ---------------------------------------------------------------------------
// 6. External (non-mapped) function call: no spurious import, no panic
// ---------------------------------------------------------------------------

#[test]
fn external_fn_call_does_not_emit_super_import() {
    let code = r#"
        pub struct Foo;

        impl Foo {
            pub fn boom(&self) {
                external_crate_fn();
            }
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    // The only module here is `types` (no standalone helpers).
    let types_rs = rendered.get("types").expect("expected `types` module");

    // `external_crate_fn()` of course appears inside the method body —
    // but it must NOT appear in any `use super::*` statement.
    let mut in_uses = false;
    for line in types_rs.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("use super::") && trimmed.contains("external_crate_fn") {
            in_uses = true;
            break;
        }
    }
    assert!(
        !in_uses,
        "external_crate_fn must not appear in any use statement; got:\n{}",
        types_rs
    );
    assert!(
        !types_rs.contains("use super::functions"),
        "types.rs must not import from a non-existent `functions` module; got:\n{}",
        types_rs
    );
}

// ---------------------------------------------------------------------------
// 7. End-to-end: picking.rs-shaped fixture builds with `cargo check`
// ---------------------------------------------------------------------------
//
// A miniature reproduction of the picking.rs failure mode. We can't run
// rustc inside a unit test, but we *can* feed the rendered modules back
// through `syn::parse_file` and confirm they parse, and we can spot-check
// the key strings that the rustc-level errors complained about.

#[test]
fn picking_rs_shape_end_to_end() {
    let code = r#"
        use std::vec::Vec;

        pub struct Ray {
            pub origin: [f64; 3],
            pub direction: [f64; 3],
        }

        impl Ray {
            pub fn new(origin: [f64; 3], direction: [f64; 3]) -> Self {
                Self { origin, direction }
            }

            pub fn distance_to_point(&self, p: [f64; 3]) -> f64 {
                let diff = sub(p, self.origin);
                let crossed = cross(diff, self.direction);
                norm(crossed) / norm(self.direction)
            }
        }

        fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
            [
                a[1] * b[2] - a[2] * b[1],
                a[2] * b[0] - a[0] * b[2],
                a[0] * b[1] - a[1] * b[0],
            ]
        }
        fn norm(v: [f64; 3]) -> f64 {
            (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
        }
        fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
            [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");
    let types_rs = rendered.get("types").expect("expected `types` module");

    // The three helpers must all be elevated.
    for name in ["cross", "norm", "sub"] {
        assert!(
            functions_rs.contains(&format!("pub(super) fn {}", name)),
            "{} must be elevated to pub(super); got:\n{}",
            name,
            functions_rs
        );
    }

    // types.rs must import all three (either brace-grouped or singly).
    let needs_all = types_rs.contains("cross")
        && types_rs.contains("norm")
        && types_rs.contains("sub")
        && types_rs.contains("use super::functions::");
    assert!(
        needs_all,
        "types.rs must `use super::functions::{{...}}`; got:\n{}",
        types_rs
    );

    // Both generated modules must parse as valid Rust.
    syn::parse_file(types_rs).expect("types.rs must parse");
    syn::parse_file(functions_rs).expect("functions.rs must parse");
}

// ---------------------------------------------------------------------------
// 8. generate_tests_rs_with_uses forwards file-level use statements
// ---------------------------------------------------------------------------
//
// Reproduces the secondary `--extract-tests` failure that surfaced when
// running the picking.rs end-to-end: the inline `mod tests` relied on a
// file-level `use oxi3d_core::{PointCloud, PointXYZ};` for its
// `make_cloud` helper. Once lifted into `tests.rs`, those imports were
// dropped on the floor unless explicitly forwarded.

#[test]
fn tests_rs_forwards_external_use_statements() {
    let code = r#"
        use external_crate::{Alpha, Beta};

        pub struct Foo;

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn t() {
                let _a = Alpha::default();
                let _b = Beta::default();
            }
        }
    "#;

    let file = syn::parse_file(code).expect("fixture must parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_extract_tests(true);
    analyzer.analyze(&file);

    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 1, "exactly one inline test mod expected");

    let tests_rs = generate_tests_rs_with_uses(&extracted, &analyzer.use_statements);

    assert!(
        tests_rs.contains("use external_crate"),
        "tests.rs must forward `use external_crate::...`; got:\n{}",
        tests_rs
    );
    assert!(
        tests_rs.contains("use super::*;"),
        "tests.rs must still emit `use super::*;`; got:\n{}",
        tests_rs
    );
}

// ---------------------------------------------------------------------------
// 9. Trait-impl method inside a type bundle: helper still gets elevated
// ---------------------------------------------------------------------------
//
// Covers the second loop addition: methods inside `type_info.trait_impls`
// must also be inspected for helper calls. Without this, a trait impl
// bundled with its type would silently miss elevations.

#[test]
fn trait_impl_method_to_sibling_helper_elevates() {
    let code = r#"
        pub trait Doer {
            fn do_it(&self) -> i32;
        }

        pub struct Worker;

        impl Doer for Worker {
            fn do_it(&self) -> i32 {
                compute(7)
            }
        }

        fn compute(x: i32) -> i32 {
            x * 3
        }
    "#;

    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    // The trait impl is batched into `worker_traits` (per-type trait grouping).
    // The standalone `Doer` trait definition and the `compute` helper both sit
    // in `functions`; the `Worker` struct sits in `types`.
    let functions_rs = rendered
        .get("functions")
        .expect("expected `functions` module");
    let trait_impls_rs = rendered
        .get("worker_traits")
        .expect("expected `worker_traits` module (per-type trait grouping)");

    assert!(
        functions_rs.contains("pub(super) fn compute"),
        "compute must be elevated to pub(super); got:\n{}",
        functions_rs
    );
    // `compute` lives in `functions` and must be imported there. The import
    // generator groups multiple names from the same module into a braced
    // `use super::functions::{...}`, so accept either the single-name form
    // (`use super::functions::compute;`) or the grouped form
    // (`use super::functions::{Doer, compute};`).
    let imports_compute_from_functions = trait_impls_rs.contains("use super::functions::compute")
        || trait_impls_rs
            .split("use super::functions::{")
            .nth(1)
            .and_then(|after| after.split('}').next())
            .is_some_and(|group| group.contains("compute"));
    assert!(
        imports_compute_from_functions,
        "worker_traits.rs must import `compute` from `super::functions`; got:\n{}",
        trait_impls_rs
    );
}
