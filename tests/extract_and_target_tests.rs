//! Integration tests for the v0.3.2 features:
//! - `--extract-tests` (Feature A): consolidates inline `#[cfg(test)] mod`
//!   blocks into a single `tests.rs`.
//! - `--target-modules` (Feature B): routes items to named output modules
//!   based on TOML rules.

use splitrs::config::{matches_pattern, route_item, Config, TargetModule, TargetModulesFile};
use splitrs::file_analyzer::FileAnalyzer;
use splitrs::module_generator::{generate_mod_rs, generate_tests_rs};
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use syn::Item;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse(code: &str) -> syn::File {
    syn::parse_file(code).expect("test fixture failed to parse as Rust")
}

fn analyzer_with(
    code: &str,
    extract_tests: bool,
    rules: Vec<TargetModule>,
) -> (syn::File, FileAnalyzer) {
    let file = parse(code);
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_extract_tests(extract_tests);
    analyzer.set_target_modules(rules);
    analyzer.analyze(&file);
    (file, analyzer)
}

fn module_names(modules: &[splitrs::module_generator::Module]) -> HashSet<String> {
    modules.iter().map(|m| m.name.clone()).collect()
}

// ---------------------------------------------------------------------------
// Feature A: --extract-tests
// ---------------------------------------------------------------------------

#[test]
fn extract_tests_single_inline_block_is_diverted() {
    let code = r#"
        pub struct Foo;
        impl Foo {
            pub fn answer() -> i32 { 42 }
        }

        #[cfg(test)]
        mod tests {
            use super::*;

            #[test]
            fn answer_is_42() {
                assert_eq!(Foo::answer(), 42);
            }
        }
    "#;

    let (_, mut analyzer) = analyzer_with(code, true, vec![]);

    // The inline test mod must be diverted out of standalone_items.
    let test_count_in_standalone = analyzer
        .standalone_items
        .iter()
        .filter(|i| matches!(i, Item::Mod(_)))
        .count();
    assert_eq!(
        test_count_in_standalone, 0,
        "inline test mod should not appear in standalone_items when --extract-tests is on"
    );

    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 1);

    let tests_rs = generate_tests_rs(&extracted);
    assert!(tests_rs.contains("use super::*;"));
    assert!(tests_rs.contains("mod tests"));
    assert!(tests_rs.contains("answer_is_42"));
}

#[test]
fn extract_tests_three_blocks_named_tests_collide_and_rename() {
    let code = r#"
        pub struct Foo;

        #[cfg(test)]
        mod tests {
            #[test]
            fn a() {}
        }

        #[cfg(test)]
        mod tests {
            #[test]
            fn b() {}
        }

        #[cfg(test)]
        mod tests {
            #[test]
            fn c() {}
        }
    "#;

    let (_, mut analyzer) = analyzer_with(code, true, vec![]);
    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 3);

    let tests_rs = generate_tests_rs(&extracted);

    // The name `tests` collides with the surrounding `tests.rs` file (and would
    // trigger `clippy::module_inception`), so every occurrence is bumped to a
    // distinct suffix — `tests_2`, `tests_3`, `tests_4` — when emitted.
    assert!(
        tests_rs.contains("mod tests_2 {"),
        "first occurrence must be renamed away from `tests`"
    );
    assert!(
        tests_rs.contains("mod tests_3 {"),
        "second occurrence must be `tests_3`"
    );
    assert!(
        tests_rs.contains("mod tests_4 {"),
        "third occurrence must be `tests_4`"
    );

    // All test bodies must be preserved.
    assert!(tests_rs.contains("fn a"));
    assert!(tests_rs.contains("fn b"));
    assert!(tests_rs.contains("fn c"));
}

#[test]
fn extract_tests_distinct_names_are_preserved_verbatim() {
    let code = r#"
        pub struct Foo;

        #[cfg(test)]
        mod tests {
            #[test]
            fn a() {}
        }

        #[cfg(test)]
        mod helper_tests {
            #[test]
            fn b() {}
        }
    "#;

    let (_, mut analyzer) = analyzer_with(code, true, vec![]);
    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 2);

    let tests_rs = generate_tests_rs(&extracted);
    // `mod tests` is always reserved (clippy::module_inception), so a single
    // `mod tests` becomes `mod tests_2`. `mod helper_tests` is distinct and
    // is preserved verbatim.
    assert!(
        tests_rs.contains("mod tests_2 {"),
        "single `mod tests` becomes `tests_2`"
    );
    assert!(tests_rs.contains("mod helper_tests {"));
    // `helper_tests` must not have been spuriously renamed.
    assert!(
        !tests_rs.contains("mod helper_tests_2"),
        "no collision rename expected for distinct name"
    );
}

#[test]
fn extract_tests_no_inline_blocks_is_noop() {
    let code = r#"
        pub struct Foo;
        impl Foo {
            pub fn answer() -> i32 { 42 }
        }
    "#;
    let (_, mut analyzer) = analyzer_with(code, true, vec![]);
    let extracted = analyzer.take_extracted_tests();
    assert!(extracted.is_empty(), "no test mods to extract -> empty Vec");
}

#[test]
fn extract_tests_flag_off_leaves_inline_block_in_standalone_items() {
    let code = r#"
        pub struct Foo;

        #[cfg(test)]
        mod tests {
            #[test]
            fn a() {}
        }
    "#;
    let (_, mut analyzer) = analyzer_with(code, false, vec![]);
    assert!(
        analyzer.take_extracted_tests().is_empty(),
        "flag is off -> nothing extracted"
    );
    let test_count_in_standalone = analyzer
        .standalone_items
        .iter()
        .filter(|i| matches!(i, Item::Mod(_)))
        .count();
    assert_eq!(
        test_count_in_standalone, 1,
        "with flag off, inline test mod stays in standalone_items"
    );
}

#[test]
fn extract_tests_external_path_mod_is_left_alone() {
    // The existing `#[cfg(test)] #[path = "..."] mod tests;` codepath must
    // not be disturbed by --extract-tests.
    let code = r#"
        pub struct Foo;

        #[cfg(test)]
        #[path = "external_tests.rs"]
        mod tests;
    "#;
    let (_, mut analyzer) = analyzer_with(code, true, vec![]);
    assert!(
        analyzer.take_extracted_tests().is_empty(),
        "external #[path] mod is NOT an inline test block"
    );
}

#[test]
fn mod_rs_appends_cfg_test_when_tests_extracted() {
    let modules: Vec<splitrs::module_generator::Module> = vec![];
    let dir = PathBuf::from("/tmp/splitrs-test-mod-rs");
    let content = generate_mod_rs(&modules, &dir, None, true, &[]).expect("generate_mod_rs failed");
    assert!(content.contains("#[cfg(test)]"));
    assert!(content.contains("mod tests;"));
}

#[test]
fn mod_rs_omits_cfg_test_when_no_tests_extracted() {
    let modules: Vec<splitrs::module_generator::Module> = vec![];
    let dir = PathBuf::from("/tmp/splitrs-test-mod-rs");
    let content =
        generate_mod_rs(&modules, &dir, None, false, &[]).expect("generate_mod_rs failed");
    // The literal substring `mod tests;` must not be present.
    assert!(
        !content.contains("mod tests;"),
        "should not declare tests module when none were extracted"
    );
}

// ---------------------------------------------------------------------------
// Feature B: --target-modules
// ---------------------------------------------------------------------------

#[test]
fn target_modules_routes_struct_to_named_module() {
    let code = r#"
        pub struct Foo;
        pub struct Bar;
        pub struct BazExt;
        pub struct Other;
    "#;
    let rules = vec![
        TargetModule {
            name: "core".to_string(),
            items: vec!["Foo".to_string(), "Bar".to_string()],
        },
        TargetModule {
            name: "extended".to_string(),
            items: vec!["Baz*".to_string()],
        },
    ];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);
    let names = module_names(&modules);

    assert!(names.contains("core"), "named `core` module missing");
    assert!(names.contains("extended"), "named `extended` missing");
    // Items not matching any rule fall through to the heuristic.
    assert!(
        names.contains("types") || names.contains("types_1") || names.contains("types_2"),
        "Other should be in the heuristic `types` bucket"
    );

    // Verify which types each module holds.
    let core = modules.iter().find(|m| m.name == "core").unwrap();
    let core_types: HashSet<_> = core.types.iter().map(|t| t.name.clone()).collect();
    assert!(core_types.contains("Foo"));
    assert!(core_types.contains("Bar"));
    assert!(!core_types.contains("BazExt"));
    assert!(!core_types.contains("Other"));

    let extended = modules.iter().find(|m| m.name == "extended").unwrap();
    let ext_types: HashSet<_> = extended.types.iter().map(|t| t.name.clone()).collect();
    assert!(ext_types.contains("BazExt"));
    assert!(!ext_types.contains("Foo"));
}

#[test]
fn target_modules_routes_functions_consts_statics() {
    let code = r#"
        pub fn do_v3_thing() {}
        pub const V3_CONST: i32 = 1;
        pub static V3_STATIC: i32 = 2;
        pub fn unrelated() {}
    "#;
    let rules = vec![TargetModule {
        name: "v3".to_string(),
        items: vec!["do_v3_thing".to_string(), "V3_*".to_string()],
    }];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);

    let v3 = modules.iter().find(|m| m.name == "v3").expect("v3 missing");
    let v3_names: HashSet<String> = v3
        .standalone_items
        .iter()
        .filter_map(|i| match i {
            Item::Fn(f) => Some(f.sig.ident.to_string()),
            Item::Const(c) => Some(c.ident.to_string()),
            Item::Static(s) => Some(s.ident.to_string()),
            _ => None,
        })
        .collect();
    assert!(v3_names.contains("do_v3_thing"));
    assert!(v3_names.contains("V3_CONST"));
    assert!(v3_names.contains("V3_STATIC"));
    assert!(!v3_names.contains("unrelated"));
}

#[test]
fn target_modules_routes_impl_for_foreign_type() {
    // `impl Drop for Foo` is in standalone_items because `Foo` is not in
    // the types map (it's a foreign / extern type for this file's view).
    let code = r#"
        impl Drop for Foo {
            fn drop(&mut self) {}
        }
    "#;
    let rules = vec![TargetModule {
        name: "foo_impls".to_string(),
        items: vec!["Foo".to_string()],
    }];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);

    let foo_impls = modules
        .iter()
        .find(|m| m.name == "foo_impls")
        .expect("foo_impls missing");
    let impl_count = foo_impls
        .standalone_items
        .iter()
        .filter(|i| matches!(i, Item::Impl(_)))
        .count();
    assert_eq!(impl_count, 1);
}

#[test]
fn target_modules_first_match_wins() {
    // `FooV3` should route to `v3` even though `Foo*` also matches.
    let code = r#"
        pub struct FooV3;
        pub struct FooBar;
    "#;
    let rules = vec![
        TargetModule {
            name: "v3".to_string(),
            items: vec!["FooV3".to_string()],
        },
        TargetModule {
            name: "extended".to_string(),
            items: vec!["Foo*".to_string()],
        },
    ];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);

    let v3 = modules.iter().find(|m| m.name == "v3").unwrap();
    let v3_types: HashSet<_> = v3.types.iter().map(|t| t.name.clone()).collect();
    assert!(v3_types.contains("FooV3"));
    assert!(!v3_types.contains("FooBar"));

    let extended = modules.iter().find(|m| m.name == "extended").unwrap();
    let ext_types: HashSet<_> = extended.types.iter().map(|t| t.name.clone()).collect();
    assert!(ext_types.contains("FooBar"));
    assert!(!ext_types.contains("FooV3"));
}

#[test]
fn target_modules_wildcard_catches_everything_else() {
    let code = r#"
        pub struct FooV3;
        pub struct Bar;
        pub struct Baz;
    "#;
    let rules = vec![
        TargetModule {
            name: "v3".to_string(),
            items: vec!["*V3".to_string()],
        },
        TargetModule {
            name: "everything".to_string(),
            items: vec!["*".to_string()],
        },
    ];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);

    // Nothing should remain for the heuristic `types` module.
    let heuristic = modules
        .iter()
        .any(|m| m.name == "types" && !m.types.is_empty());
    assert!(!heuristic, "wildcard catch-all should consume all types");

    let everything = modules.iter().find(|m| m.name == "everything").unwrap();
    let names: HashSet<_> = everything.types.iter().map(|t| t.name.clone()).collect();
    assert!(names.contains("Bar"));
    assert!(names.contains("Baz"));
    assert!(!names.contains("FooV3"));
}

#[test]
fn target_modules_empty_rule_module_is_not_emitted() {
    let code = r#"
        pub struct Foo;
    "#;
    let rules = vec![
        TargetModule {
            name: "v3".to_string(),
            items: vec!["FooV3".to_string()],
        },
        TargetModule {
            name: "core".to_string(),
            items: vec!["Foo".to_string()],
        },
    ];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);
    let names = module_names(&modules);
    // No item matched `v3` -> the empty module must be dropped.
    assert!(!names.contains("v3"));
    assert!(names.contains("core"));
}

// ---------------------------------------------------------------------------
// Combined: --extract-tests + --target-modules
// ---------------------------------------------------------------------------

#[test]
fn combined_extract_tests_and_target_modules() {
    let code = r#"
        pub struct Foo;
        pub struct Bar;

        impl Foo {
            pub fn answer() -> i32 { 42 }
        }

        #[cfg(test)]
        mod tests {
            use super::*;
            #[test]
            fn answer_is_42() {
                assert_eq!(Foo::answer(), 42);
            }
        }
    "#;
    let rules = vec![TargetModule {
        name: "core".to_string(),
        items: vec!["Foo".to_string(), "Bar".to_string()],
    }];

    let file = parse(code);
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_extract_tests(true);
    analyzer.set_target_modules(rules);
    analyzer.analyze(&file);

    let modules = analyzer.group_by_module(500);
    let names = module_names(&modules);
    assert!(names.contains("core"));

    let extracted = analyzer.take_extracted_tests();
    assert_eq!(extracted.len(), 1, "one inline test mod expected");

    let tests_rs = generate_tests_rs(&extracted);
    assert!(tests_rs.contains("answer_is_42"));

    // mod.rs must hook in both `core` and `tests`.
    let mod_rs = generate_mod_rs(&modules, &PathBuf::from("/tmp/splitrs"), None, true, &[])
        .expect("generate_mod_rs failed");
    assert!(mod_rs.contains("pub mod core;"));
    assert!(mod_rs.contains("mod tests;"));
}

// ---------------------------------------------------------------------------
// Standalone TOML config parsing (TargetModulesFile)
// ---------------------------------------------------------------------------

#[test]
fn target_modules_file_loads_from_disk() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("target_modules.toml");
    fs::write(
        &path,
        r#"
[[target_modules]]
name = "v3"
items = ["BoundaryExtV3", "StreamingBoundaryEstimator"]

[[target_modules]]
name = "core"
items = ["*"]
"#,
    )
    .expect("write");

    let spec = TargetModulesFile::from_file(&path).expect("load");
    assert_eq!(spec.target_modules.len(), 2);
    assert_eq!(spec.target_modules[0].name, "v3");
    assert_eq!(spec.target_modules[1].items, vec!["*"]);
}

#[test]
fn target_modules_embedded_in_main_config_file_round_trip() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join(".splitrs.toml");
    fs::write(
        &path,
        r#"
[splitrs]
max_lines = 800
extract_tests = true

[[target_modules]]
name = "wave"
items = ["BoundaryWave*"]
"#,
    )
    .expect("write");

    let config = Config::from_file(&path).expect("load");
    assert_eq!(config.splitrs.max_lines, 800);
    assert!(config.splitrs.extract_tests);
    assert_eq!(config.target_modules.len(), 1);
    assert_eq!(config.target_modules[0].name, "wave");
    assert_eq!(config.target_modules[0].items, vec!["BoundaryWave*"]);
}

// ---------------------------------------------------------------------------
// Pure-pattern matching tests (sanity duplicate of unit tests for visibility
// at the integration layer; quick to run).
// ---------------------------------------------------------------------------

#[test]
fn pattern_matching_basic_forms() {
    assert!(matches_pattern("Foo", "*"));
    assert!(matches_pattern("Foo", "Foo"));
    assert!(matches_pattern("FooBar", "Foo*"));
    assert!(matches_pattern("MyConfig", "*Config"));
    assert!(!matches_pattern("Bar", "Foo"));
}

#[test]
fn route_item_returns_none_when_no_rule_matches() {
    let rules = vec![TargetModule {
        name: "v3".to_string(),
        items: vec!["FooV3".to_string()],
    }];
    assert_eq!(route_item("Bar", &rules), None);
    assert_eq!(route_item("FooV3", &rules), Some("v3"));
}

#[test]
fn target_modules_bundles_trait_impls_with_type() {
    // When `Foo` is routed to `core`, its `impl Display for Foo` (a trait
    // impl) should travel with the type — NOT end up in the separate
    // `trait_impls` heuristic module.
    let code = r#"
        pub struct Foo;
        impl Foo {
            pub fn x() -> i32 { 1 }
        }
        impl std::fmt::Display for Foo {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "Foo")
            }
        }
    "#;
    let rules = vec![TargetModule {
        name: "core".to_string(),
        items: vec!["Foo".to_string()],
    }];
    let (_, analyzer) = analyzer_with(code, false, rules);
    let modules = analyzer.group_by_module(500);

    let names = module_names(&modules);
    assert!(names.contains("core"));
    // No `trait_impls` heuristic module should be produced because Foo
    // (the only type with trait impls) is routed away from the heuristic.
    assert!(
        !names.contains("trait_impls"),
        "trait impls of a routed type should not produce a heuristic trait_impls module"
    );

    let core = modules.iter().find(|m| m.name == "core").unwrap();
    let foo = core
        .types
        .iter()
        .find(|t| t.name == "Foo")
        .expect("Foo missing");
    assert_eq!(foo.trait_impls.len(), 1);
    assert_eq!(foo.trait_impls[0].trait_name, "Display");
}
