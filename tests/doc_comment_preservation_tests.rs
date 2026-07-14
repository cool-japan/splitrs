//! Tests for Item #5: Doc-comment preservation.
//!
//! Verifies that file-level `//!` inner doc comments are captured from the
//! source file and emitted at the top of the generated `mod.rs`. Also checks
//! that per-item `///` doc comments travel with their items unchanged.

use splitrs::file_analyzer::FileAnalyzer;
use splitrs::module_generator::{generate_mod_rs, Module};

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

// ---------------------------------------------------------------------------
// Test 1: file-level `//!` docs are captured and emitted in mod.rs
// ---------------------------------------------------------------------------

#[test]
fn test_file_inner_doc_preserved_in_mod_rs() {
    let code = r#"
//! This is crate documentation.
//! It spans multiple lines.

pub struct Foo {
    value: i32,
}
"#;

    let (_, analyzer) = analyze(code);

    assert!(
        !analyzer.file_inner_docs.is_empty(),
        "expected file_inner_docs to be populated from //! attrs"
    );

    let modules: Vec<Module> = analyzer.group_by_module(500);
    let mod_rs = generate_mod_rs(
        &modules,
        &std::env::temp_dir().join("splitrs-doc-test"),
        None,
        false,
        &analyzer.file_inner_docs,
    )
    .expect("generate_mod_rs must succeed");

    assert!(
        mod_rs.contains("This is crate documentation"),
        "expected doc text in mod.rs output; got:\n{}",
        mod_rs
    );
    assert!(
        mod_rs.contains("//!"),
        "expected //! style doc comment in mod.rs; got:\n{}",
        mod_rs
    );
}

// ---------------------------------------------------------------------------
// Test 2: per-type `///` doc comments travel with the struct definition
// (regression guard — this should always have worked; we confirm it does
// after our changes)
// ---------------------------------------------------------------------------

#[test]
fn test_type_doc_comment_preserved() {
    let code = r#"
/// A documented struct.
/// Has multiple doc lines.
pub struct Documented {
    value: i32,
}

impl Documented {
    pub fn new(value: i32) -> Self { Self { value } }
}
"#;

    let (file, analyzer) = analyze(code);
    let modules = analyzer.group_by_module(500);

    let dummy_needs_pub: std::collections::HashSet<String> = std::collections::HashSet::new();
    let dummy_fields: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    let empty_uses: Vec<syn::Item> = Vec::new();
    let type_to_module: std::collections::HashMap<String, String> = modules
        .iter()
        .flat_map(|m| m.types.iter().map(|t| (t.name.clone(), m.name.clone())))
        .collect();

    // Find the module that contains `Documented`
    let doc_module = modules
        .iter()
        .find(|m| m.types.iter().any(|t| t.name == "Documented"))
        .expect("expected a module containing 'Documented'");

    let content = doc_module.generate_content(
        &file,
        &empty_uses,
        &type_to_module,
        &dummy_needs_pub,
        None,
        &dummy_fields,
        None,
        &dummy_needs_pub,
    );

    assert!(
        content.contains("A documented struct"),
        "expected struct doc comment in module content; got:\n{}",
        content
    );
}

// ---------------------------------------------------------------------------
// Test 3: `///` docs on a function land in `functions.rs` unchanged
// ---------------------------------------------------------------------------

#[test]
fn test_regular_item_doc_unchanged() {
    let code = r#"
/// A very helpful utility function.
/// Returns x plus one.
pub fn increment(x: i32) -> i32 {
    x + 1
}
"#;

    let (file, analyzer) = analyze(code);
    let modules = analyzer.group_by_module(500);

    let dummy_needs_pub: std::collections::HashSet<String> = std::collections::HashSet::new();
    let dummy_fields: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    let empty_uses: Vec<syn::Item> = Vec::new();
    let type_to_module: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();

    // Find the functions module
    let fn_module = modules
        .iter()
        .find(|m| m.name == "functions")
        .expect("expected 'functions' module");

    let content = fn_module.generate_content(
        &file,
        &empty_uses,
        &type_to_module,
        &dummy_needs_pub,
        None,
        &dummy_fields,
        None,
        &dummy_needs_pub,
    );

    assert!(
        content.contains("A very helpful utility function"),
        "expected function doc comment in functions module; got:\n{}",
        content
    );
    assert!(
        content.contains("Returns x plus one"),
        "expected second doc line in functions module; got:\n{}",
        content
    );
}
