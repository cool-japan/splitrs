//! Tests for Item #2: Semantic naming for batched impl blocks.
//!
//! Verifies that `MethodGroup::suggest_name()` results are used to produce
//! meaningful module names like `<type>_serialization.rs` instead of the
//! generic `<type>_impl_N.rs` fallback.

use splitrs::file_analyzer::FileAnalyzer;
use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse code and run FileAnalyzer with impl splitting enabled at a low threshold
/// so that even moderate-sized impl blocks trigger a split.
fn analyze_with_split(code: &str, max_impl_lines: usize) -> (syn::File, FileAnalyzer) {
    let file = syn::parse_file(code).expect("test fixture must parse as Rust");
    let mut analyzer = FileAnalyzer::new(true, max_impl_lines);
    analyzer.analyze(&file);
    (file, analyzer)
}

/// Return the set of module names produced by group_by_module.
fn module_names(analyzer: &FileAnalyzer, max_lines: usize) -> HashSet<String> {
    analyzer
        .group_by_module(max_lines)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

// ---------------------------------------------------------------------------
// Build a type body with enough lines to force splitting.
// Each method body is padded to ~20 lines with let-bindings so the impl
// block reliably exceeds max_impl_lines=30.
// ---------------------------------------------------------------------------

fn padded_method(name: &str, body: &str) -> String {
    format!(
        r#"
    pub fn {name}(&self) -> i32 {{
        {body}
        let _a1 = 1;
        let _a2 = 2;
        let _a3 = 3;
        let _a4 = 4;
        let _a5 = 5;
        let _a6 = 6;
        let _a7 = 7;
        let _a8 = 8;
        let _a9 = 9;
        let _a10 = 10;
        let _a11 = 11;
        let _a12 = 12;
        let _a13 = 13;
        let _a14 = 14;
        let _a15 = 15;
        let _a16 = 16;
        let _a17 = 17;
        let _a18 = 18;
        let _a19 = 19;
        42
    }}
"#,
        name = name,
        body = body,
    )
}

// ---------------------------------------------------------------------------
// Test 1: serialization methods → `<type>_serialization` module name
// ---------------------------------------------------------------------------

#[test]
fn test_serialization_methods_get_semantic_name() {
    let m1 = padded_method("serialize_json", "let _x = 0;");
    let m2 = padded_method("deserialize_json", "let _x = 1;");

    let code = format!(
        r#"
pub struct Payload {{
    data: Vec<u8>,
}}

impl Payload {{
    {m1}
    {m2}
}}
"#
    );

    let (_, analyzer) = analyze_with_split(&code, 30);
    let names = module_names(&analyzer, 500);

    assert!(
        names.iter().any(|n| n.contains("serialization")),
        "expected a module containing 'serialization' in name; got: {:?}",
        names
    );
    // Must NOT produce a generic _impl suffix for this batch
    assert!(
        !names.iter().any(|n| n == "payload_impl"),
        "should not produce generic 'payload_impl' when semantic name is available; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 2: builder methods → `<type>_builders` module name
// ---------------------------------------------------------------------------

#[test]
fn test_builder_methods_get_semantic_name() {
    let m1 = padded_method("with_value", "let _x = 0;");
    let m2 = padded_method("with_name", "let _x = 1;");

    let code = format!(
        r#"
pub struct Config {{
    value: i32,
    name: String,
}}

impl Config {{
    {m1}
    {m2}
}}
"#
    );

    let (_, analyzer) = analyze_with_split(&code, 30);
    let names = module_names(&analyzer, 500);

    assert!(
        names.iter().any(|n| n.contains("builders")),
        "expected a module containing 'builders' in name; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 3: collision → numeric suffix fallback
//
// Two separate impl blocks on the same type, each serialization-heavy.
// The second must be renamed to `foo_serialization_2`.
// ---------------------------------------------------------------------------

#[test]
fn test_collision_falls_back_to_numeric_suffix() {
    // Build two impl blocks each with serialization methods, both large enough
    // to trigger splitting individually.
    let m1 = padded_method("serialize_json", "let _x = 0;");
    let m2 = padded_method("deserialize_json", "let _x = 1;");
    let m3 = padded_method("serialize_xml", "let _x = 2;");
    let m4 = padded_method("deserialize_xml", "let _x = 3;");

    let code = format!(
        r#"
pub struct Document {{
    content: String,
}}

impl Document {{
    {m1}
    {m2}
}}

impl Document {{
    {m3}
    {m4}
}}
"#
    );

    // Use a very low threshold to guarantee both impl blocks are split
    let (_, analyzer) = analyze_with_split(&code, 10);
    let names = module_names(&analyzer, 500);

    let ser_count = names.iter().filter(|n| n.contains("serialization")).count();

    // We expect at least two modules with "serialization" in the name:
    // `document_serialization` and `document_serialization_2`
    assert!(
        ser_count >= 2,
        "expected at least 2 serialization-named modules for two separate impl blocks; got {:?}",
        names
    );

    assert!(
        names
            .iter()
            .any(|n| n.ends_with("_2") && n.contains("serialization")),
        "expected a collision-deduplicated `..._2` module; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 4: unstructured methods → legacy `<type>_impl` naming preserved
// ---------------------------------------------------------------------------

#[test]
fn test_unstructured_methods_preserve_legacy_naming() {
    let m1 = padded_method("foo_bar", "let _x = 0;");
    let m2 = padded_method("baz_qux", "let _x = 1;");

    let code = format!(
        r#"
pub struct Widget {{
    data: u32,
}}

impl Widget {{
    {m1}
    {m2}
}}
"#
    );

    let (_, analyzer) = analyze_with_split(&code, 30);
    let names = module_names(&analyzer, 500);

    // `suggest_name` for `foo_bar` / `baz_qux` falls through to `foo_bar_group`
    // (ends with `_group`), so the semantic check fails and we fall back to `_impl`.
    assert!(
        names
            .iter()
            .any(|n| n.contains("_impl") || n.contains("_group")),
        "expected legacy '_impl' or '_group' style name for unstructured methods; got: {:?}",
        names
    );

    // Must NOT produce a false semantic name
    assert!(
        !names
            .iter()
            .any(|n| n.contains("serialization") || n.contains("builders")),
        "should not produce semantic name for unstructured methods; got: {:?}",
        names
    );
}
