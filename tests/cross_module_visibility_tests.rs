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
use std::collections::{HashMap, HashSet};

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
                &HashSet::new(),
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

// ---------------------------------------------------------------------------
// 10. `--split-impl-blocks` chunk calling a private method in a SIBLING chunk
// ---------------------------------------------------------------------------
//
// Reproduces the oxisqlite-core `jsonb.rs` regression this test module didn't
// yet cover, in two layers:
//
// Layer 1 (visibility): `compute_cross_module_visibility` built its "who
// defines this name" map (`fn_to_module`) from `module.standalone_items`
// only. Methods living solely inside `module.method_group` -- i.e. methods of
// an oversized `impl` block that `--split-impl-blocks` moved into their own
// per-chunk module -- were valid *callees* (the call-detection side already
// saw them) but were never recorded as *definition sites*. A private helper
// called from a sibling chunk (extremely common: `--split-impl-blocks`
// bin-packs methods by source order/line budget, not by call graph, per
// `find_clusters`'s "fine-grained individual clusters" design) then silently
// kept its original private visibility -- `error[E0624]: method ... is
// private`.
//
// Layer 2 (import): naively fixing layer 1 by widening `fn_to_module` to
// cover methods everywhere (including import-emission) overcorrects: unlike
// a free function, a method is not a module-level path item.
// `use super::some_module::a_method_name;` is syntactically valid but a hard
// compile error (E0432, caught only by `rustc`/`cargo check` -- NOT by
// `syn::parse_file`, which just confirms the `use` statement parses). Methods
// resolve through type-directed lookup gated purely by visibility; they must
// receive the `pub(super)` upgrade but never a `use` import.
//
// The fixture below exercises both: `helper` (a method) must be elevated
// but NOT imported, while `helper_fn` (a plain function, called the same
// way — across the same chunk boundary) must be BOTH elevated and imported.
//
// A single small private `helper` plus enough padding methods to spill the
// `impl` block across multiple `--max-impl-lines`-budgeted chunks reproduces
// this deterministically: bin-packing is sequential in source order, so
// `helper` (declared first) lands in an earlier chunk than `caller` (declared
// last), which must still resolve `self.helper()` across that chunk boundary.
#[test]
fn split_impl_block_chunk_calling_sibling_chunk_private_method_elevates_and_imports() {
    let mut code = String::from(
        r#"
        pub struct Big {
            pub value: i32,
        }

        fn helper_fn(x: i32) -> i32 {
            x * 3
        }

        impl Big {
            fn helper(&self) -> i32 {
                self.value * 2
            }
"#,
    );
    // Padding methods, each large enough on its own to force a fresh
    // `--max-impl-lines` chunk boundary between `helper` and `caller` below.
    for i in 0..6 {
        code.push_str(&format!(
            "            pub fn padding_{i}(&self) -> i32 {{\n                \
                 let mut acc = self.value;\n                \
                 acc += {i};\n                \
                 acc += {i} * 2;\n                \
                 acc += {i} * 3;\n                \
                 acc\n            \
             }}\n"
        ));
    }
    code.push_str(
        r#"
            pub fn caller(&self) -> i32 {
                self.helper() + helper_fn(self.value)
            }
        }
    "#,
    );

    let mut analyzer = FileAnalyzer::new(true, 8);
    let file = syn::parse_file(&code).expect("test fixture failed to parse as Rust");
    analyzer.analyze(&file);
    // `max_impl_lines` only governs the internal method-to-group bin-packing;
    // `group_by_module`'s own `max_lines` then consolidates adjacent small
    // groups back up to ITS budget when generating the final module list
    // (see the "batching logic" this test's own module-doc references). Use
    // the same small budget for both so the chunk boundaries this test
    // depends on survive into the final `modules` list.
    let modules = analyzer.group_by_module(8);

    // Sanity check the fixture actually reproduces the multi-chunk shape: at
    // least one module holding `method_group` methods, and `helper` /
    // `caller` must land in *different* modules (otherwise the fixture failed
    // to force the split this test exists to exercise).
    let method_group_modules: Vec<&splitrs::module_generator::Module> = modules
        .iter()
        .filter(|m| m.method_group.is_some())
        .collect();
    assert!(
        method_group_modules.len() >= 2,
        "fixture must force `impl Big` across >= 2 method_group chunks; got {} chunk module(s): {:?}",
        method_group_modules.len(),
        modules.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    let module_of = |method_name: &str| {
        method_group_modules
            .iter()
            .find(|m| {
                m.method_group
                    .as_ref()
                    .is_some_and(|g| g.methods.iter().any(|meth| meth.name == method_name))
            })
            .map(|m| m.name.clone())
    };
    let helper_module = module_of("helper").expect("fixture must define `helper`");
    let caller_module = module_of("caller").expect("fixture must define `caller`");
    assert_ne!(
        helper_module, caller_module,
        "fixture must place `helper` and `caller` in different chunks to reproduce the bug"
    );

    // The actual assertion: `helper` (a METHOD) must be elevated to
    // pub(super) but must NOT get a `use super::<mod>::helper;` import (that
    // would be E0432: methods aren't module-path items). `helper_fn` (a
    // plain FUNCTION called across the exact same chunk boundary) is the
    // control case: it must be BOTH elevated and imported, proving the fix
    // didn't overcorrect by dropping free-function imports too.
    let (needs_pub_super, cross_module_imports, _fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    assert!(
        needs_pub_super.contains("helper"),
        "`helper` (method) must be elevated to pub(super); got: {:?}",
        needs_pub_super
    );
    assert!(
        needs_pub_super.contains("helper_fn"),
        "`helper_fn` (free fn) must be elevated to pub(super); got: {:?}",
        needs_pub_super
    );
    let caller_imports = cross_module_imports.get(&caller_module);
    let imports_helper_method = caller_imports.is_some_and(|by_module| {
        by_module
            .values()
            .any(|names| names.iter().any(|n| n == "helper"))
    });
    assert!(
        !imports_helper_method,
        "{caller_module:?} must NOT import the method `helper` via `use` \
         (E0432: methods aren't module-path items); got: {:?}",
        caller_imports
    );
    let imports_helper_fn = caller_imports.is_some_and(|by_module| {
        by_module
            .values()
            .any(|names| names.iter().any(|n| n == "helper_fn"))
    });
    assert!(
        imports_helper_fn,
        "{caller_module:?} must import the free function `helper_fn`; got: {:?}",
        caller_imports
    );

    // And the full pipeline must actually produce valid, self-contained Rust:
    // render every module, confirm each parses, `helper` is `pub(super)` in
    // its own file with NO matching `use ... helper;` anywhere, and
    // `helper_fn` IS imported by name into `caller`'s file.
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for m in &modules {
        for t in m.get_exported_types() {
            type_to_module.insert(t, m.name.clone());
        }
    }
    let rendered: HashMap<String, String> = modules
        .iter()
        .map(|m| {
            let content = m.generate_content(
                &file,
                &analyzer.use_statements,
                &type_to_module,
                &needs_pub_super,
                cross_module_imports.get(&m.name),
                &_fields_need_pub_super,
                Some(&analyzer.trait_tracker),
                &HashSet::new(),
            );
            (m.name.clone(), content)
        })
        .collect();
    for (name, content) in &rendered {
        syn::parse_file(content)
            .unwrap_or_else(|e| panic!("generated module {name:?} must parse: {e}\n{content}"));
    }
    let helper_rs = &rendered[&helper_module];
    assert!(
        helper_rs.contains("pub(super) fn helper("),
        "{helper_module:?} must render `helper` as `pub(super)`; got:\n{helper_rs}"
    );
    let caller_rs = &rendered[&caller_module];
    assert!(
        !caller_rs.lines().any(|l| {
            let t = l.trim_start();
            t.starts_with("use ") && t.contains("::helper;") && !t.contains("helper_fn")
        }),
        "{caller_module:?} must not `use` the method `helper` by path; got:\n{caller_rs}"
    );
    assert!(
        caller_rs.contains("helper_fn"),
        "{caller_module:?} must import `helper_fn` from `super::functions`; got:\n{caller_rs}"
    );
}

// ---------------------------------------------------------------------------
// 11. Extracted test referencing a sibling module's constant by name
// ---------------------------------------------------------------------------
//
// Reproduces the oxisqlite-core `jsonb.rs` regression this test module didn't
// yet cover: the extracted-tests special case in
// `compute_cross_module_visibility` only ever consulted `fn_to_module`
// (functions/methods) when deciding what `tests.rs` needs imported. A test
// referencing a private `const`/`static` from a sibling production module
// directly by name -- `SIZE_MARKER_8BIT`, `MAX_JSON_DEPTH`, and similar
// bit-format constants in the real regression -- was invisible to that map,
// so no `use super::<module>::<CONST>;` was ever emitted for `tests.rs`.
// Unlike a private *function*, the const's own visibility was never the
// problem (`upgrade_type_visibility` unconditionally widens every private
// const/static to `pub(super)`, regardless of `needs_pub_super`) -- only the
// import was missing, so this failed at the `cargo test`/`nextest` stage
// with `error[E0425]: cannot find value ... in this scope`, invisible to
// `cargo build`/`cargo check` (which don't compile `#[cfg(test)]` code by
// default) -- exactly how the real regression slipped past this crate's
// non-test build/check gates and was only caught running the actual test
// suite.
#[test]
fn extracted_test_referencing_sibling_const_gets_import() {
    let code = r#"
        const MAGIC: u32 = 42;

        pub struct Foo {
            pub value: u32,
        }

        impl Foo {
            pub fn new(value: u32) -> Self {
                Self { value }
            }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn uses_magic_constant() {
                assert_eq!(MAGIC, 42);
            }
        }
    "#;

    let file = syn::parse_file(code).expect("fixture must parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_extract_tests(true);
    analyzer.analyze(&file);
    let modules = analyzer.group_by_module(500);

    // `compute_cross_module_visibility`'s "extracted tests" special case
    // reads `self.extracted_tests` -- it MUST run before `take_extracted_tests`
    // (which, per its name, `std::mem::take`s that field, i.e. drains it to
    // empty) or it silently sees zero extracted tests and no-ops. `main.rs`
    // gets this order right (`compute_cross_module_visibility` at line ~976,
    // `take_extracted_tests` at ~1068); mirror it here.
    let (needs_pub_super, cross_module_imports, _fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 1, "exactly one inline test mod expected");

    let empty = HashMap::new();
    let tests_sibling_imports = cross_module_imports.get("tests").unwrap_or(&empty);
    let tests_rs = splitrs::module_generator::generate_tests_rs_with_imports(
        &extracted,
        &analyzer.use_statements,
        tests_sibling_imports,
    );

    assert!(
        tests_rs.contains("MAGIC"),
        "tests.rs must import the sibling module's `MAGIC` const; got:\n{tests_rs}"
    );
    let imports_magic = tests_rs.lines().any(|l| {
        let t = l.trim_start();
        t.starts_with("use ") && t.contains("::MAGIC")
    });
    assert!(
        imports_magic,
        "tests.rs must contain a `use super::<module>::MAGIC;`-shaped import; got:\n{tests_rs}"
    );
    syn::parse_file(&tests_rs)
        .unwrap_or_else(|e| panic!("generated tests.rs must parse: {e}\n{tests_rs}"));

    // And every production module must still parse too (sanity: the fixture
    // itself, and the `needs_pub_super`/rendering pipeline, are well-formed).
    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for m in &modules {
        for t in m.get_exported_types() {
            type_to_module.insert(t, m.name.clone());
        }
    }
    for m in &modules {
        let content = m.generate_content(
            &file,
            &analyzer.use_statements,
            &type_to_module,
            &needs_pub_super,
            cross_module_imports.get(&m.name),
            &_fields_need_pub_super,
            Some(&analyzer.trait_tracker),
            &HashSet::new(),
        );
        syn::parse_file(&content)
            .unwrap_or_else(|e| panic!("module {:?} must parse: {e}\n{content}", m.name));
    }
}

// ---------------------------------------------------------------------------
// N. File-backed sibling `mod` declarations (`pub mod x;`, `content: None`)
//    must stay pinned to the root `mod.rs`, and cross-references to their
//    symbols must gain a `super::` prefix once relocated to a sibling file.
// ---------------------------------------------------------------------------
//
// Reproduces the `ast/mod.rs` split failure in oxisql's
// oxisqlite-sqlite3-parser crate: `pub mod check;` / `pub mod fmt;` were
// relocated into a generated bucket file by the generic standalone-item
// heuristic. That broke BOTH the physical file lookup (`error[E0583]`:
// `mod check;` declared inside `functions.rs` makes rustc look for
// `functions/check.rs` instead of the real `check.rs`) and every OTHER
// sibling file's forwarded `use fmt::TokenStream;` (`error[E0432]`: valid
// only from the original root file, which is where `mod fmt;` used to
// live). Fixed by `FileAnalyzer::file_backed_mods` (never bucket these) and
// `rewrite_pinned_mod_refs_in_use` (prefix forwarded references to them).

#[test]
fn file_backed_mod_declaration_is_diverted_not_bucketed() {
    let code = r#"
        pub mod existing;

        pub struct Foo {
            pub value: i32,
        }
    "#;
    let (_file, mut analyzer) = analyze(code);
    let file_backed = analyzer.take_file_backed_mods();
    assert_eq!(
        file_backed
            .iter()
            .map(|m| m.ident.to_string())
            .collect::<Vec<_>>(),
        vec!["existing".to_string()],
        "file-backed `pub mod existing;` must be captured, not bucketed"
    );

    let modules = analyzer.group_by_module(500);
    for m in &modules {
        assert!(
            !m.standalone_items
                .iter()
                .any(|item| matches!(item, syn::Item::Mod(mi) if mi.ident == "existing")),
            "file-backed mod must never land in a generated bucket's \
             standalone_items (module {:?})",
            m.name
        );
    }
}

#[test]
fn use_of_pinned_sibling_mod_symbol_gets_super_prefix() {
    let code = r#"
        pub mod existing;

        use existing::Marker;

        pub struct Foo {
            pub value: i32,
        }

        impl Foo {
            pub fn make(&self) -> Marker {
                Marker
            }
        }
    "#;
    let (file, mut analyzer) = analyze(code);
    let file_backed = analyzer.take_file_backed_mods();
    let pinned_root_mod_names: HashSet<String> =
        file_backed.iter().map(|m| m.ident.to_string()).collect();

    let modules = analyzer.group_by_module(500);
    let types_mod = modules
        .iter()
        .find(|m| m.name == "types")
        .expect("expected a `types` module containing `Foo`");

    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);
    let type_to_module: HashMap<String, String> = HashMap::new();

    let content = types_mod.generate_content(
        &file,
        &analyzer.use_statements,
        &type_to_module,
        &needs_pub_super,
        cross_module_imports.get(&types_mod.name),
        &fields_need_pub_super,
        Some(&analyzer.trait_tracker),
        &pinned_root_mod_names,
    );

    assert!(
        content.contains("use super::existing::Marker"),
        "reference to a pinned sibling mod's symbol must be rewritten with a \
         `super::` prefix (a bare `use existing::Marker;`, valid only from \
         the original root file, does not resolve from a sibling generated \
         file); got:\n{}",
        content
    );
    assert!(
        !content.contains("\nuse existing::Marker"),
        "must not ALSO keep the unprefixed bare form; got:\n{}",
        content
    );
    syn::parse_file(&content)
        .unwrap_or_else(|e| panic!("generated module must parse: {e}\n{content}"));
}

#[test]
fn bitflags_invocation_defined_type_is_importable_across_modules() {
    let code = r#"
        bitflags::bitflags! {
            #[derive(Clone, Copy, Debug, Eq, PartialEq)]
            pub struct Flags: u8 {
                const A = 0x01;
                const B = 0x02;
            }
        }

        pub struct Holder {
            pub flags: Flags,
        }
    "#;
    let (file, analyzer) = analyze(code);
    let modules = analyzer.group_by_module(500);

    let macros_mod = modules
        .iter()
        .find(|m| m.name == "macros")
        .expect("expected a `macros` module containing the bitflags! invocation");
    assert!(
        macros_mod
            .get_exported_types()
            .contains(&"Flags".to_string()),
        "bitflags! invocation must register `Flags` as an exported/importable \
         type name, not just an opaque macro item; exported: {:?}",
        macros_mod.get_exported_types()
    );

    let mut type_to_module: HashMap<String, String> = HashMap::new();
    for m in &modules {
        for t in m.importable_exported_names() {
            type_to_module.insert(t, m.name.clone());
        }
    }
    let (needs_pub_super, cross_module_imports, fields_need_pub_super) =
        analyzer.compute_cross_module_visibility(&modules);

    let types_mod = modules
        .iter()
        .find(|m| m.name == "types")
        .expect("expected a `types` module containing `Holder`");
    let content = types_mod.generate_content(
        &file,
        &analyzer.use_statements,
        &type_to_module,
        &needs_pub_super,
        cross_module_imports.get(&types_mod.name),
        &fields_need_pub_super,
        Some(&analyzer.trait_tracker),
        &HashSet::new(),
    );

    assert!(
        content.contains("Flags"),
        "`Holder`'s `flags: Flags` field must still reference `Flags`; got:\n{}",
        content
    );
    assert!(
        content.contains("use super::macros::Flags")
            || content.contains("macros::{") && content.contains("Flags"),
        "`Holder`'s `flags: Flags` field must import `Flags` from wherever the \
         bitflags! invocation landed; got:\n{}",
        content
    );
    syn::parse_file(&content)
        .unwrap_or_else(|e| panic!("generated module must parse: {e}\n{content}"));

    // The facade must ALSO re-export `Flags` from the regenerated root
    // `mod.rs` (`pub use macros::*;`), matching its visibility before the
    // split — otherwise any file OUTSIDE the split that reached `Flags` via
    // the crate's public surface (`crate::ast::*`) loses access, even files
    // this tool has no business editing (e.g. a Lemon-generated parser).
    assert!(
        macros_mod.has_public_reexport(),
        "a module containing a `pub struct`-defining bitflags! invocation \
         must be considered to have a public reexport"
    );
    assert!(
        macros_mod
            .public_export_names()
            .contains(&"Flags".to_string()),
        "`Flags` must appear in the module's public export names; got: {:?}",
        macros_mod.public_export_names()
    );
}

// ---------------------------------------------------------------------------
// N+1. `Type::from_str(...)` (an associated-function call, not a `.method()`
//      call) implicitly needs `FromStr` in scope exactly like a real method
//      call would. `visit_expr_method_call` never sees it because it isn't
//      method-call syntax, so a forwarded `use std::str::FromStr;` looked
//      unused and got pruned -- same failure shape as the already-fixed
//      `write!`/`writeln!` case (`error[E0599]` instead of `E0433`/`E0432`).
// ---------------------------------------------------------------------------

#[test]
fn associated_from_str_call_keeps_fromstr_import() {
    let code = r#"
        use std::str::FromStr;

        pub struct Port(pub u16);

        impl Port {
            pub fn parse_from(text: &str) -> Port {
                Port(u16::from_str(text).unwrap_or(0))
            }
        }
    "#;
    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);

    let types_rs = rendered.get("types").expect("expected `types` module");
    assert!(
        types_rs.contains("FromStr"),
        "an associated `u16::from_str(...)` call must keep the forwarded \
         `use std::str::FromStr;` import alive; got:\n{}",
        types_rs
    );
    syn::parse_file(types_rs)
        .unwrap_or_else(|e| panic!("generated module must parse: {e}\n{types_rs}"));
}

// ---------------------------------------------------------------------------
// N+2. A `self` leaf inside a group (`use a::b::{self, *};`) binds the name
//      of the ENCLOSING path segment, not the literal text "self". A sibling
//      `*` glob is always unconditionally kept (its contribution can't be
//      statically narrowed) -- but that must not drag along an unrelated
//      `self` binding into a module that never references the enclosing
//      name itself, or rustc reports `unused_imports`.
// ---------------------------------------------------------------------------

#[test]
fn self_leaf_in_group_is_pruned_independently_of_sibling_glob() {
    let code = r#"
        use std::cmp::Ordering::{self, *};

        pub struct Foo {
            pub value: i32,
        }

        impl Foo {
            pub fn cmp_value(&self, other: i32) -> bool {
                self.value.cmp(&other) == Less
            }
        }
    "#;
    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);
    let types_rs = rendered.get("types").expect("expected `types` module");

    assert!(
        types_rs.contains("Ordering::{self, *}") || types_rs.contains("Ordering::*"),
        "the `*` glob (providing `Less`) must be kept even without `self`; \
         got:\n{}",
        types_rs
    );
    assert!(
        !types_rs.contains("{self, *}") && !types_rs.contains("{*, self}"),
        "`Ordering` (the `self` binding) is never referenced by name in this \
         module, only `Less` (via the glob) is -- keeping `self` produces an \
         `unused_imports` warning; got:\n{}",
        types_rs
    );
    syn::parse_file(types_rs)
        .unwrap_or_else(|e| panic!("generated module must parse: {e}\n{types_rs}"));
}

#[test]
fn self_leaf_in_group_is_kept_when_enclosing_name_is_used() {
    let code = r#"
        use std::cmp::Ordering::{self, *};

        pub struct Foo {
            pub ord: Ordering,
        }

        impl Foo {
            pub fn cmp_value(&self, other: i32) -> bool {
                self.ord == Less
            }
        }
    "#;
    let (file, analyzer) = analyze(code);
    let rendered = render_modules(&file, &analyzer);
    let types_rs = rendered.get("types").expect("expected `types` module");

    assert!(
        types_rs.contains("{self, *}") || types_rs.contains("{*, self}"),
        "`Ordering` (the `self` binding) IS used as the `ord` field's type, \
         so it must be kept alongside the glob providing `Less`; got:\n{}",
        types_rs
    );
    syn::parse_file(types_rs)
        .unwrap_or_else(|e| panic!("generated module must parse: {e}\n{types_rs}"));
}
