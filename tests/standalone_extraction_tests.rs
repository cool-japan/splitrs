//! Tests for Item #4: Const/Static/Macro/TypeAlias extraction.
//!
//! Verifies that standalone items are partitioned by kind into dedicated
//! modules (`constants`, `macros`, `type_aliases`, `functions`) instead of
//! everything going into the catch-all `functions.rs`.

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
// Test 1: const and static items go to `constants` module
// ---------------------------------------------------------------------------

#[test]
fn test_const_static_routed_to_constants_module() {
    let code = r#"
pub const MAX_SIZE: usize = 1024;
pub static GLOBAL_FLAG: bool = false;
pub const BUFFER_LEN: usize = 4096;
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        names.contains("constants"),
        "expected 'constants' module for const/static items; got: {:?}",
        names
    );
    assert!(
        !names.contains("functions"),
        "should NOT route const/static to 'functions'; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 2: macro_rules! items go to `macros` module
// ---------------------------------------------------------------------------

#[test]
fn test_macro_rules_routed_to_macros_module() {
    let code = r#"
#[macro_export]
macro_rules! my_vec {
    ($($x:expr),*) => {
        {
            let mut v = Vec::new();
            $(v.push($x);)*
            v
        }
    };
}

macro_rules! assert_close {
    ($a:expr, $b:expr, $tol:expr) => {
        assert!(($a - $b).abs() < $tol, "{} vs {} tolerance {}", $a, $b, $tol);
    };
}
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        names.contains("macros"),
        "expected 'macros' module for macro_rules! items; got: {:?}",
        names
    );
    assert!(
        !names.contains("functions"),
        "should NOT route macro_rules! to 'functions'; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 3: type alias items go to `type_aliases` module
// ---------------------------------------------------------------------------

#[test]
fn test_type_alias_routed_to_type_aliases_module() {
    let code = r#"
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
pub type ByteSlice<'a> = &'a [u8];
pub type Callback = fn(i32) -> i32;
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        names.contains("type_aliases"),
        "expected 'type_aliases' module for type alias items; got: {:?}",
        names
    );
    assert!(
        !names.contains("functions"),
        "should NOT route type aliases to 'functions'; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 4: mixed items are partitioned correctly into separate modules
// ---------------------------------------------------------------------------

#[test]
fn test_mixed_items_partitioned_correctly() {
    let code = r#"
pub const VERSION: &str = "1.0.0";

pub type Offset = usize;

macro_rules! log_info {
    ($msg:expr) => { println!("[INFO] {}", $msg); };
}

pub fn helper(x: i32) -> i32 {
    x + 1
}
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        names.contains("constants"),
        "expected 'constants' for const items; got: {:?}",
        names
    );
    assert!(
        names.contains("type_aliases"),
        "expected 'type_aliases' for type alias; got: {:?}",
        names
    );
    assert!(
        names.contains("macros"),
        "expected 'macros' for macro_rules!; got: {:?}",
        names
    );
    assert!(
        names.contains("functions"),
        "expected 'functions' for fn items; got: {:?}",
        names
    );
}

// ---------------------------------------------------------------------------
// Test 5: empty buckets do not produce empty modules
// ---------------------------------------------------------------------------

#[test]
fn test_empty_buckets_skip_module_emission() {
    // Only functions — no const, static, macro, or type alias
    let code = r#"
pub fn alpha(x: i32) -> i32 { x }
pub fn beta(x: i32) -> i32 { x + 1 }
"#;

    let (_, analyzer) = analyze(code);
    let names = module_names(&analyzer, 500);

    assert!(
        !names.contains("constants"),
        "should not emit empty 'constants' module; got: {:?}",
        names
    );
    assert!(
        !names.contains("macros"),
        "should not emit empty 'macros' module; got: {:?}",
        names
    );
    assert!(
        !names.contains("type_aliases"),
        "should not emit empty 'type_aliases' module; got: {:?}",
        names
    );
    assert!(
        names.contains("functions"),
        "should emit 'functions' for fn items; got: {:?}",
        names
    );
}
