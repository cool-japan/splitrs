//! Integration tests for Feature C (`--split-nested-mods`): recursive descent
//! into inline `mod x { ... }` blocks, correct nested emission, visibility
//! preservation, doc/attr/cfg preservation, super-path deepening, and facade
//! generation.

use splitrs::config::{FacadeStyle, TargetModule};
use splitrs::file_analyzer::FileAnalyzer;
use splitrs::nested_mod_splitter::{
    add_child_mod_imports, dry_run_lines, plan_nested_split, write_plan, NestedModPlan,
    NestedSplitOptions,
};
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// The motivating shape: a file dominated by one inline `pub mod core` with
/// structs, enums, impls (inherent + trait), fns, consts, and use statements.
const CORE_FIXTURE: &str = r#"
use std::collections::HashMap;

/// Top-level entry.
pub fn init() -> bool {
    true
}

/// Core module docs.
#[cfg(not(feature = "never"))]
pub mod core {
    //! Inner docs for core.
    use super::*;
    use std::path::PathBuf;

    /// A filesystem entry.
    #[derive(Debug, Clone)]
    pub struct FsEntry {
        pub path: PathBuf,
        size: u64,
    }

    impl FsEntry {
        pub fn new(path: PathBuf) -> Self {
            Self { path, size: 0 }
        }

        pub fn size(&self) -> u64 {
            self.size
        }
    }

    impl Default for FsEntry {
        fn default() -> Self {
            Self::new(PathBuf::new())
        }
    }

    /// Sorting options.
    pub enum SortBy {
        Name,
        Size,
    }

    pub const MAX_DEPTH: usize = 16;

    fn helper(map: &HashMap<String, u64>) -> usize {
        map.len()
    }

    pub fn list_entries(map: &HashMap<String, u64>) -> usize {
        helper(map)
    }

    pub fn watch_path(entry: &FsEntry) -> bool {
        super::init() && entry.size() == 0
    }
}
"#;

fn default_opts(rules: &[TargetModule], max_lines: usize) -> NestedSplitOptions<'_> {
    NestedSplitOptions {
        split_impl_blocks: false,
        max_impl_lines: 500,
        max_lines,
        extract_tests: false,
        max_mod_depth: 8,
        seeded_assignment: false,
        all_rules: rules,
    }
}

fn first_inline_mod(code: &str) -> syn::ItemMod {
    let file = syn::parse_file(code).expect("fixture parses");
    file.items
        .into_iter()
        .find_map(|item| match item {
            syn::Item::Mod(m) if m.content.is_some() => Some(m),
            _ => None,
        })
        .expect("fixture has an inline mod")
}

fn plan_fixture(code: &str, max_lines: usize) -> NestedModPlan {
    let mod_item = first_inline_mod(code);
    let mod_path = mod_item.ident.to_string();
    plan_nested_split(&mod_item, code, &default_opts(&[], max_lines), &mod_path, 1)
        .expect("planning succeeds")
}

/// Every name exported anywhere across a plan's own modules.
fn all_exported(plan: &NestedModPlan) -> HashSet<String> {
    plan.modules
        .iter()
        .flat_map(|m| m.get_exported_types())
        .collect()
}

/// Recursively assert that every generated `.rs` file parses as valid Rust.
fn assert_all_files_parse(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            assert_all_files_parse(&path);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let content = fs::read_to_string(&path).expect("read generated file");
            syn::parse_file(&content)
                .unwrap_or_else(|e| panic!("generated file {:?} does not parse: {}", path, e));
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {:?}: {}", path, e))
}

// ---------------------------------------------------------------------------
// Analyzer diversion
// ---------------------------------------------------------------------------

#[test]
fn over_budget_inline_mod_is_diverted() {
    let file = syn::parse_file(CORE_FIXTURE).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_split_nested_mods(true, 20);
    analyzer.set_source(CORE_FIXTURE);
    analyzer.analyze(&file);

    let nested = analyzer.take_nested_mods();
    assert_eq!(nested.len(), 1, "the big `mod core` must be diverted");
    assert_eq!(nested[0].ident.to_string(), "core");
    assert!(
        !analyzer
            .standalone_items
            .iter()
            .any(|i| matches!(i, syn::Item::Mod(_))),
        "diverted mod must not remain in standalone_items"
    );
}

#[test]
fn under_budget_inline_mod_stays_standalone() {
    let file = syn::parse_file(CORE_FIXTURE).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_split_nested_mods(true, 10_000);
    analyzer.set_source(CORE_FIXTURE);
    analyzer.analyze(&file);

    assert!(analyzer.take_nested_mods().is_empty());
    assert!(
        analyzer
            .standalone_items
            .iter()
            .any(|i| matches!(i, syn::Item::Mod(_))),
        "under-budget mod stays an opaque standalone item"
    );
}

#[test]
fn disabled_flag_keeps_all_mods_standalone() {
    let file = syn::parse_file(CORE_FIXTURE).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.analyze(&file);
    assert!(analyzer.take_nested_mods().is_empty());
}

#[test]
fn cfg_test_mod_is_never_diverted_as_nested() {
    let code = r#"
        #[cfg(test)]
        mod tests {
            #[test] fn a() {}
            #[test] fn b() {}
            #[test] fn c() {}
            #[test] fn d() {}
        }
    "#;
    let file = syn::parse_file(code).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_split_nested_mods(true, 2);
    analyzer.analyze(&file);
    assert!(
        analyzer.take_nested_mods().is_empty(),
        "test mods flow through the tests machinery, not Feature C"
    );
}

#[test]
fn cfg_feature_testkit_mod_is_not_misclassified_as_test() {
    // Regression for the token-substring heuristic: `feature = "testkit"`
    // must not count as `cfg(test)`.
    let code = r#"
        #[cfg(feature = "testkit")]
        pub mod kit {
            pub fn a() {}
            pub fn b() {}
            pub fn c() {}
            pub fn d() {}
        }
    "#;
    let file = syn::parse_file(code).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_split_nested_mods(true, 2);
    analyzer.set_source(code);
    analyzer.analyze(&file);
    let nested = analyzer.take_nested_mods();
    assert_eq!(nested.len(), 1, "cfg(feature) mod must be divertible");
    assert_eq!(nested[0].ident.to_string(), "kit");
}

// ---------------------------------------------------------------------------
// Plan shape
// ---------------------------------------------------------------------------

#[test]
fn plan_covers_every_item_of_the_mod_body() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    assert_eq!(plan.name, "core");
    assert!(matches!(plan.vis, syn::Visibility::Public(_)));

    let exported = all_exported(&plan);
    for name in [
        "FsEntry",
        "SortBy",
        "MAX_DEPTH",
        "helper",
        "list_entries",
        "watch_path",
    ] {
        assert!(exported.contains(name), "missing {name} in plan modules");
    }
    assert!(plan.children.is_empty());
}

#[test]
fn plan_decl_preserves_outer_docs_cfg_and_visibility() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    let decl = plan.decl_item();
    let rendered = prettyplease::unparse(&syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![syn::Item::Mod(decl)],
    });
    assert!(rendered.contains("/// Core module docs."), "{rendered}");
    assert!(
        rendered.contains("#[cfg(not(feature = \"never\"))]"),
        "{rendered}"
    );
    assert!(rendered.contains("pub mod core;"), "{rendered}");
}

#[test]
fn private_mod_decl_stays_private() {
    let code = r#"
        mod internal {
            pub struct Hidden;
            fn helper() {}
            pub fn touch() { helper(); }
        }
    "#;
    let plan = plan_fixture(code, 2);
    assert!(matches!(plan.vis, syn::Visibility::Inherited));
    let rendered = prettyplease::unparse(&syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: vec![syn::Item::Mod(plan.decl_item())],
    });
    assert!(rendered.contains("mod internal;"), "{rendered}");
    assert!(!rendered.contains("pub mod internal;"), "{rendered}");
}

#[test]
fn nested_in_nested_mods_recurse() {
    let code = r#"
        pub mod outer {
            pub fn outer_fn() -> usize { 1 }

            pub mod deep {
                pub struct DeepThing {
                    pub level: usize,
                }

                impl DeepThing {
                    pub fn level(&self) -> usize { self.level }
                }

                pub fn deep_fn() -> usize { 2 }
            }
        }
    "#;
    let plan = plan_fixture(code, 5);
    assert_eq!(plan.name, "outer");
    assert_eq!(plan.children.len(), 1);
    assert_eq!(plan.children[0].name, "deep");
    let deep_exports = all_exported(&plan.children[0]);
    assert!(deep_exports.contains("DeepThing"));
    assert!(deep_exports.contains("deep_fn"));
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

#[test]
fn write_plan_emits_parseable_tree_with_facade_and_inner_docs() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    let dir = TempDir::new().expect("tempdir");
    let created = write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");
    assert!(!created.is_empty());
    assert_all_files_parse(dir.path());

    let mod_rs = read(&dir.path().join("core").join("mod.rs"));
    // Inner `//!` docs of the mod body land at the top of core/mod.rs.
    assert!(mod_rs.contains("//! Inner docs for core."), "{mod_rs}");
    // Glob facade so `crate::core::FsEntry` keeps resolving.
    assert!(mod_rs.contains("pub use "), "{mod_rs}");
    assert!(mod_rs.contains("pub mod "), "{mod_rs}");
    // Child mods are declared by the PARENT mod.rs, never re-exported here.
    assert!(!mod_rs.contains("pub mod core;"), "{mod_rs}");
}

#[test]
fn super_paths_are_deepened_in_emitted_files() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");

    // Collect the concatenated content of every generated core/*.rs file.
    let core_dir = dir.path().join("core");
    let mut combined = String::new();
    for entry in fs::read_dir(&core_dir).expect("read core dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "rs") {
            combined.push_str(&read(&path));
        }
    }
    // `super::init()` in the mod body sits one level deeper now.
    assert!(
        combined.contains("super::super::init()"),
        "body super path was not deepened:\n{combined}"
    );
    assert!(
        !combined.contains("super::super::super::init()"),
        "body super path was deepened more than once:\n{combined}"
    );
    // The mod-body `use super::*;` is forwarded one level deeper (when kept).
    assert!(
        !combined.contains("use super::*;\nuse super::*;"),
        "sanity: no duplicated globs"
    );
}

#[test]
fn cross_module_visibility_upgrades_apply_inside_the_nested_mod() {
    // Force `helper` (private) and `list_entries` into DIFFERENT generated
    // modules by routing list_entries to a named module; the cross-module
    // pass must upgrade helper to pub(super) and import it.
    let rules = vec![TargetModule {
        name: "listing".to_string(),
        items: vec!["list_entries".to_string()],
        parent: Some("core".to_string()),
        ..Default::default()
    }];
    let mod_item = first_inline_mod(CORE_FIXTURE);
    let plan = plan_nested_split(
        &mod_item,
        CORE_FIXTURE,
        &default_opts(&rules, 20),
        "core",
        1,
    )
    .expect("planning succeeds");

    assert!(
        plan.needs_pub_super.contains("helper"),
        "private helper called cross-module must be upgraded: {:?}",
        plan.needs_pub_super
    );

    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");
    let listing = read(&dir.path().join("core").join("listing.rs"));
    assert!(
        listing.contains("::helper"),
        "listing.rs must import the relocated helper:\n{listing}"
    );
    assert_all_files_parse(dir.path());
}

#[test]
fn named_facade_lists_public_items_explicitly() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Named).expect("write_plan");
    let mod_rs = read(&dir.path().join("core").join("mod.rs"));
    assert!(
        !mod_rs.contains("::*;"),
        "named facade must not glob:\n{mod_rs}"
    );
    assert!(mod_rs.contains("FsEntry"), "{mod_rs}");
    assert!(mod_rs.contains("list_entries"), "{mod_rs}");
    // Private helper must NOT be re-exported.
    assert!(!mod_rs.contains("helper"), "{mod_rs}");
    assert_all_files_parse(dir.path());
}

#[test]
fn none_facade_emits_declarations_only() {
    let plan = plan_fixture(CORE_FIXTURE, 20);
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::None).expect("write_plan");
    let mod_rs = read(&dir.path().join("core").join("mod.rs"));
    assert!(!mod_rs.contains("pub use"), "{mod_rs}");
    assert!(mod_rs.contains("pub mod"), "{mod_rs}");
}

#[test]
fn nested_in_nested_write_creates_grandchild_directory() {
    let code = r#"
        pub mod outer {
            pub fn outer_fn() -> usize { deep::deep_fn() }

            pub mod deep {
                pub fn deep_fn() -> usize { 2 }
                pub fn other_fn() -> usize { 3 }
                pub struct DeepThing;
            }
        }
    "#;
    let plan = plan_fixture(code, 4);
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");

    let outer_mod = read(&dir.path().join("outer").join("mod.rs"));
    assert!(outer_mod.contains("pub mod deep;"), "{outer_mod}");
    assert!(
        dir.path()
            .join("outer")
            .join("deep")
            .join("mod.rs")
            .exists(),
        "grandchild mod.rs missing"
    );
    // The sibling file calling `deep::deep_fn()` needs `use super::deep;`.
    let mut combined = String::new();
    for entry in fs::read_dir(dir.path().join("outer")).expect("read outer") {
        let path = entry.expect("entry").path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            combined.push_str(&read(&path));
        }
    }
    assert!(
        combined.contains("use super::deep;"),
        "sibling child-mod import missing:\n{combined}"
    );
    assert_all_files_parse(dir.path());
}

#[test]
fn generated_bucket_name_yields_to_real_child_mod_name() {
    // A nested `mod types` (a real module) must keep its name; the generated
    // `types.rs` bucket must be renamed.
    let code = r#"
        pub mod outer {
            pub struct Standalone {
                pub x: u8,
            }

            pub mod types {
                pub struct Inner;
                pub fn make() -> Inner { Inner }
                pub fn destroy(_: Inner) {}
            }
        }
    "#;
    let plan = plan_fixture(code, 4);
    assert_eq!(plan.children.len(), 1);
    assert_eq!(plan.children[0].name, "types");
    assert!(
        plan.modules.iter().all(|m| m.name != "types"),
        "generated bucket must have been renamed: {:?}",
        plan.modules.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");
    assert!(dir
        .path()
        .join("outer")
        .join("types")
        .join("mod.rs")
        .exists());
    assert_all_files_parse(dir.path());
}

#[test]
fn extracted_tests_inside_nested_mod_become_tests_rs() {
    let code = r#"
        pub mod core {
            pub fn answer() -> i32 { 42 }
            pub fn double(x: i32) -> i32 { x * 2 }
            pub struct Marker;

            #[cfg(test)]
            mod tests {
                use super::*;

                #[test]
                fn answer_is_42() {
                    assert_eq!(answer(), 42);
                }
            }
        }
    "#;
    let mod_item = first_inline_mod(code);
    let mut opts = default_opts(&[], 5);
    opts.extract_tests = true;
    let plan = plan_nested_split(&mod_item, code, &opts, "core", 1).expect("plan");
    assert_eq!(plan.extracted_tests.len(), 1);

    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), FacadeStyle::Glob).expect("write_plan");
    let tests_rs = read(&dir.path().join("core").join("tests.rs"));
    assert!(tests_rs.contains("answer_is_42"), "{tests_rs}");
    let mod_rs = read(&dir.path().join("core").join("mod.rs"));
    assert!(mod_rs.contains("#[cfg(test)]"), "{mod_rs}");
    assert!(mod_rs.contains("mod tests;"), "{mod_rs}");
    assert_all_files_parse(dir.path());
}

#[test]
fn dry_run_lines_render_the_full_tree() {
    let code = r#"
        pub mod outer {
            pub fn f1() {}
            pub mod deep {
                pub fn f2() {}
                pub fn f3() {}
                pub struct S;
            }
        }
    "#;
    let plan = plan_fixture(code, 3);
    let lines = dry_run_lines(&plan, 0);
    let joined = lines.join("\n");
    assert!(joined.contains("outer/"), "{joined}");
    assert!(joined.contains("deep/"), "{joined}");
    assert!(joined.contains("mod.rs"), "{joined}");
}

// ---------------------------------------------------------------------------
// Sibling child-mod imports (top-level composition unit)
// ---------------------------------------------------------------------------

#[test]
fn add_child_mod_imports_records_only_real_references() {
    let code = r#"
        pub fn boot() -> usize { core::count() }
        pub fn independent() -> usize { 7 }
    "#;
    let file = syn::parse_file(code).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.analyze(&file);
    let mut modules = analyzer.group_by_module(1000);
    add_child_mod_imports(&mut modules, &["core".to_string()]);
    let functions = modules
        .iter()
        .find(|m| m.name == "functions")
        .expect("functions bucket");
    assert_eq!(functions.sibling_mod_imports, vec!["core".to_string()]);

    let content = functions.generate_content(
        &file,
        &analyzer.use_statements,
        &std::collections::HashMap::new(),
        &HashSet::new(),
        None,
        &std::collections::HashMap::new(),
        None,
    );
    assert!(
        content.contains("use super::core;"),
        "sibling child-mod import not emitted:\n{content}"
    );
}

// ---------------------------------------------------------------------------
// CLI end-to-end
// ---------------------------------------------------------------------------

#[test]
fn cli_split_nested_mods_end_to_end() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, CORE_FIXTURE).expect("write fixture");
    let output = dir.path().join("out");

    let status = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--split-nested-mods")
        .arg("true")
        .arg("--max-lines")
        .arg("20")
        .status()
        .expect("run splitrs");
    assert!(status.success(), "splitrs exited with {status}");

    let mod_rs = read(&output.join("mod.rs"));
    assert!(mod_rs.contains("pub mod core;"), "{mod_rs}");
    assert!(mod_rs.contains("/// Core module docs."), "{mod_rs}");
    assert!(
        mod_rs.contains("#[cfg(not(feature = \"never\"))]"),
        "{mod_rs}"
    );
    // The child mod is declared but NOT re-exported at the parent: items must
    // stay at crate::core::Item exactly where they were.
    assert!(!mod_rs.contains("pub use core::*;"), "{mod_rs}");
    // The mod body resolved `HashMap` through `use super::*;` — the original
    // file-scope binding must be recreated in mod.rs so the chain still works.
    assert!(
        mod_rs.contains("use std::collections::HashMap;"),
        "file-scope use binding not recreated for the descended mod:\n{mod_rs}"
    );
    assert!(output.join("core").join("mod.rs").exists());
    assert_all_files_parse(&output);
}

#[test]
fn parent_scope_items_bind_private_fns_and_prune_uses() {
    use splitrs::nested_mod_splitter::compute_parent_scope_items;

    let code = r#"
        use std::collections::HashMap;
        use std::path::PathBuf;

        fn top_helper() -> u64 { 41 }

        pub mod core {
            use super::*;

            pub fn probe(map: &HashMap<String, u64>) -> u64 {
                super::top_helper() + map.len() as u64
            }
        }
    "#;
    let file = syn::parse_file(code).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_split_nested_mods(true, 3);
    analyzer.set_source(code);
    analyzer.analyze(&file);
    let nested = analyzer.take_nested_mods();
    assert_eq!(nested.len(), 1);
    let modules = analyzer.group_by_module(1000);

    let mut needs_pub_super = HashSet::new();
    let scope_uses = compute_parent_scope_items(
        &nested,
        &analyzer.use_statements,
        &modules,
        &mut needs_pub_super,
        false,
    );
    let rendered = prettyplease::unparse(&syn::File {
        shebang: None,
        attrs: Vec::new(),
        items: scope_uses,
    });
    // The referenced file-scope import is kept; the unreferenced one pruned.
    assert!(
        rendered.contains("use std::collections::HashMap;"),
        "{rendered}"
    );
    assert!(!rendered.contains("PathBuf"), "{rendered}");
    // The private fn referenced via `super::` gets a binding + an upgrade.
    assert!(
        rendered.contains("use self::functions::top_helper;"),
        "{rendered}"
    );
    assert!(needs_pub_super.contains("top_helper"));
}

#[test]
fn macro_defining_module_gets_macro_use_and_first_position() {
    let code = r#"
        macro_rules! twice {
            ($e:expr) => { $e + $e };
        }

        pub fn use_it() -> u64 { twice!(21) }
    "#;
    let file = syn::parse_file(code).expect("parse");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_source(code);
    analyzer.analyze(&file);
    let modules = analyzer.group_by_module(1000);
    let mod_rs = splitrs::module_generator::generate_mod_rs(
        &modules,
        Path::new("/tmp/unused"),
        None,
        false,
        &[],
    )
    .expect("generate_mod_rs");
    assert!(mod_rs.contains("#[macro_use]\npub mod macros;"), "{mod_rs}");
    let macros_pos = mod_rs.find("pub mod macros;").expect("macros decl");
    let functions_pos = mod_rs.find("pub mod functions;").expect("functions decl");
    assert!(
        macros_pos < functions_pos,
        "macro-defining module must be declared first:\n{mod_rs}"
    );
}

/// Depth-2 fixture shared by the CLI facade / depth-guard tests: `deep` is
/// itself over budget at `--max-lines 4`, so unrestricted runs descend twice.
const DEPTH2_FIXTURE: &str = r#"
pub mod outer {
    /// Outer worker.
    pub fn outer_fn() -> usize { deep::deep_fn() }
    pub struct OuterThing {
        pub id: usize,
    }
    impl OuterThing {
        pub fn id(&self) -> usize { self.id }
    }
    pub mod deep {
        pub struct DeepThing;
        pub fn deep_fn() -> usize { 2 }
        pub fn other_fn() -> usize { 3 }
        pub fn third_fn() -> usize { 4 }
    }
}
"#;

fn run_depth2_cli(dir: &TempDir, extra: &[&str]) -> std::path::PathBuf {
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, DEPTH2_FIXTURE).expect("write fixture");
    let output = dir.path().join("out");
    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--split-nested-mods")
        .arg("true")
        .arg("--max-lines")
        .arg("4")
        .args(extra)
        .output()
        .expect("run splitrs");
    assert!(
        cmd.status.success(),
        "splitrs failed:\n{}\n{}",
        String::from_utf8_lossy(&cmd.stdout),
        String::from_utf8_lossy(&cmd.stderr)
    );
    output
}

#[test]
fn cli_facade_named_lists_explicit_reexports() {
    let dir = TempDir::new().expect("tempdir");
    let output = run_depth2_cli(&dir, &["--facade", "named"]);

    let outer_mod = read(&output.join("outer").join("mod.rs"));
    assert!(
        !outer_mod.contains("::*;"),
        "named facade must not glob:\n{outer_mod}"
    );
    assert!(
        outer_mod.contains("pub use types::OuterThing;"),
        "named facade must re-export public items explicitly:\n{outer_mod}"
    );
    // The descended grandchild is declared, never re-exported: items must
    // stay reachable at their original `outer::deep::*` paths only.
    assert!(outer_mod.contains("pub mod deep;"), "{outer_mod}");
    assert!(!outer_mod.contains("pub use deep"), "{outer_mod}");
    assert_all_files_parse(&output);
}

#[test]
fn cli_facade_none_emits_declarations_only() {
    let dir = TempDir::new().expect("tempdir");
    let output = run_depth2_cli(&dir, &["--facade", "none"]);

    let outer_mod = read(&output.join("outer").join("mod.rs"));
    assert!(
        !outer_mod.contains("pub use"),
        "facade none must not re-export anything:\n{outer_mod}"
    );
    assert!(outer_mod.contains("pub mod deep;"), "{outer_mod}");
    assert_all_files_parse(&output);
}

#[test]
fn cli_max_mod_depth_keeps_deeper_mods_opaque() {
    // Unrestricted control run: depth 2 is reached.
    let free = TempDir::new().expect("tempdir");
    let free_out = run_depth2_cli(&free, &[]);
    assert!(
        free_out.join("outer").join("deep").join("mod.rs").exists(),
        "control run must descend into outer/deep/"
    );

    // Guarded run: --max-mod-depth 1 descends into `outer` but must leave
    // the over-budget `deep` as an opaque inline mod inside outer/*.rs.
    let guarded = TempDir::new().expect("tempdir");
    let out = run_depth2_cli(&guarded, &["--max-mod-depth", "1"]);
    assert!(
        !out.join("outer").join("deep").exists(),
        "--max-mod-depth 1 must not create outer/deep/"
    );
    let mut inline_deep = false;
    for entry in fs::read_dir(out.join("outer")).expect("read outer") {
        let path = entry.expect("entry").path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            inline_deep |= read(&path).contains("pub mod deep {");
        }
    }
    assert!(
        inline_deep,
        "the depth-limited mod must survive as an inline `pub mod deep {{ ... }}`"
    );
    assert_all_files_parse(&out);
}

#[test]
fn cli_dry_run_previews_nested_tree_without_writing() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, CORE_FIXTURE).expect("write fixture");
    let output = dir.path().join("out");

    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--split-nested-mods")
        .arg("true")
        .arg("--max-lines")
        .arg("20")
        .arg("--dry-run")
        .output()
        .expect("run splitrs");
    assert!(cmd.status.success());
    let stdout = String::from_utf8_lossy(&cmd.stdout);
    assert!(stdout.contains("core/"), "{stdout}");
    assert!(!output.exists(), "dry run must not create files");
}
