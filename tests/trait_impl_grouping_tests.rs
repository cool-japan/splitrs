//! Tests for Item #3: Per-type trait-impl grouping.
//!
//! Verifies that each type's trait implementations go to their own
//! `<type>_traits.rs` module rather than a shared `trait_impls.rs`.

use splitrs::file_analyzer::FileAnalyzer;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse(code: &str) -> syn::File {
    syn::parse_file(code).expect("test fixture must parse as Rust")
}

fn analyze(code: &str) -> (syn::File, FileAnalyzer) {
    let file = parse(code);
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.analyze(&file);
    (file, analyzer)
}

fn module_names(analyzer: &FileAnalyzer, max_lines: usize) -> HashSet<String> {
    analyzer
        .group_by_module(max_lines)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

// ---------------------------------------------------------------------------
// Test 1: each type gets its own traits module
// ---------------------------------------------------------------------------

#[test]
fn test_each_type_gets_own_traits_module() {
    // Both Foo and Bar have trait impls, each is an "oversized" type
    // (using fake inherent impls to simulate that). With item #3 each
    // gets its own `foo_traits` / `bar_traits` module.
    let code = r#"
use std::fmt;

pub struct Foo {
    value: i32,
}

pub struct Bar {
    value: i32,
}

impl fmt::Display for Foo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl fmt::Debug for Foo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Foo({})", self.value)
    }
}

impl fmt::Display for Bar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl fmt::Debug for Bar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bar({})", self.value)
    }
}
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        names.contains("foo_traits"),
        "expected 'foo_traits' module; got: {:?}",
        names
    );
    assert!(
        names.contains("bar_traits"),
        "expected 'bar_traits' module; got: {:?}",
        names
    );
    assert!(
        !names.contains("trait_impls"),
        "should NOT produce legacy 'trait_impls' module; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 2: single type with many traits batches within that type
// ---------------------------------------------------------------------------

fn trait_impl_block(trait_name: &str, type_name: &str, body_lines: usize) -> String {
    // Generate a fake trait impl body that occupies body_lines lines
    let body = (0..body_lines)
        .map(|i| format!("        let _line{i} = {i};", i = i))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        r#"
pub trait {trait_name} {{
    fn method(&self) -> i32;
}}

impl {trait_name} for {type_name} {{
    fn method(&self) -> i32 {{
{body}
        0
    }}
}}
"#,
        trait_name = trait_name,
        type_name = type_name,
        body = body,
    )
}

#[test]
fn test_single_type_with_many_traits_batches_within_type() {
    // Create several large trait impls for the same type so they overflow
    // the max_lines budget, forcing a second batch file.
    let t1 = trait_impl_block("TraitAlpha", "Baz", 60);
    let t2 = trait_impl_block("TraitBeta", "Baz", 60);

    let code = format!(
        r#"
pub struct Baz {{
    x: i32,
}}

{t1}
{t2}
"#
    );

    // max_lines=50 forces the second trait impl into a second batch
    let (_, analyzer) = analyze(&code);
    let names = module_names(&analyzer, 50);

    assert!(
        names.contains("baz_traits"),
        "expected 'baz_traits'; got: {:?}",
        names
    );
    assert!(
        names.contains("baz_traits_2"),
        "expected 'baz_traits_2' for overflow batch; got: {:?}",
        names
    );

    // All baz trait modules should NOT be mixed with other types
    let non_baz_traits: Vec<_> = names
        .iter()
        .filter(|n| n.ends_with("_traits") && !n.starts_with("baz"))
        .collect();
    assert!(
        non_baz_traits.is_empty(),
        "unexpected non-baz trait modules: {:?}",
        non_baz_traits
    );
}

// ---------------------------------------------------------------------------
// Test 3: types with only inherent impls produce no `_traits` module
// ---------------------------------------------------------------------------

#[test]
fn test_no_trait_impls_emits_no_traits_module() {
    let code = r#"
pub struct Plain {
    value: i32,
}

impl Plain {
    pub fn new(value: i32) -> Self {
        Self { value }
    }

    pub fn get(&self) -> i32 {
        self.value
    }
}
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    let traits_modules: Vec<_> = names.iter().filter(|n| n.ends_with("_traits")).collect();

    assert!(
        traits_modules.is_empty(),
        "should emit no '_traits' module when type has no trait impls; got: {:?}",
        traits_modules
    );
}
