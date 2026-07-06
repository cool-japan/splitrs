//! Integration tests for the F2 domain-mapping extensions of Feature B
//! (`--target-modules`): extended glob patterns, seeded assignment of
//! unlisted items, spec validation, per-rule budgets/docs, and the F1 x F2
//! composition (`parent = "core"`).

use splitrs::config::{matches_pattern, validate_target_modules, TargetModule, TargetModulesFile};
use splitrs::domain_router::{check_unmatched_patterns, routable_names};
use splitrs::file_analyzer::FileAnalyzer;
use splitrs::module_generator::Module;
use splitrs::nested_mod_splitter::{plan_nested_split, write_plan, NestedSplitOptions};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::process::Command;
use syn::Item;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn analyzer_with(code: &str, rules: Vec<TargetModule>, seeded: bool) -> (syn::File, FileAnalyzer) {
    let file = syn::parse_file(code).expect("fixture parses");
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.set_target_modules(rules);
    analyzer.set_seeded_assignment(seeded);
    analyzer.set_source(code);
    analyzer.analyze(&file);
    (file, analyzer)
}

fn standalone_names(module: &Module) -> HashSet<String> {
    module
        .standalone_items
        .iter()
        .filter_map(|item| match item {
            Item::Fn(f) => Some(f.sig.ident.to_string()),
            Item::Const(c) => Some(c.ident.to_string()),
            Item::Static(s) => Some(s.ident.to_string()),
            _ => None,
        })
        .collect()
}

fn find<'a>(modules: &'a [Module], name: &str) -> &'a Module {
    modules.iter().find(|m| m.name == name).unwrap_or_else(|| {
        panic!(
            "module `{name}` missing; have: {:?}",
            modules.iter().map(|m| &m.name).collect::<Vec<_>>()
        )
    })
}

// ---------------------------------------------------------------------------
// Extended glob patterns
// ---------------------------------------------------------------------------

#[test]
fn extended_patterns_route_items() {
    let code = r#"
        pub fn compute_md5_hash() -> u64 { 0 }
        pub fn sha_of_file() -> u64 { 1 }
        pub fn unrelated() {}
    "#;
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["*hash*".to_string(), "sha*file".to_string()],
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(code, rules, false);
    let modules = analyzer.group_by_module(500);
    let hash = find(&modules, "hash");
    let names = standalone_names(hash);
    assert!(names.contains("compute_md5_hash"));
    assert!(names.contains("sha_of_file"));
    assert!(!names.contains("unrelated"));
}

#[test]
fn pattern_sanity_at_integration_layer() {
    assert!(matches_pattern("compute_hash_fast", "*hash*"));
    assert!(matches_pattern("alpha_beta_gamma", "alpha*gamma"));
    assert!(matches_pattern("HashWriter", "Hash*"));
    assert!(!matches_pattern("digest", "*hash*"));
}

// ---------------------------------------------------------------------------
// Seeded assignment
// ---------------------------------------------------------------------------

const SEED_FIXTURE: &str = r#"
    pub fn compute_md5(data: &[u8]) -> String {
        hex_encode(hash_bytes(data))
    }

    fn hash_bytes(data: &[u8]) -> u64 {
        data.len() as u64
    }

    fn hex_encode(value: u64) -> String {
        format!("{value:x}")
    }

    pub fn unrelated() -> u8 { 7 }
"#;

#[test]
fn pull_dependencies_seeds_private_helpers_into_named_module() {
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["compute_md5".to_string()],
        pull_dependencies: true,
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(SEED_FIXTURE, rules, false);
    let modules = analyzer.group_by_module(500);

    let hash = find(&modules, "hash");
    let names = standalone_names(hash);
    assert!(names.contains("compute_md5"), "seed missing: {names:?}");
    assert!(
        names.contains("hash_bytes"),
        "direct dependency not pulled: {names:?}"
    );
    assert!(
        names.contains("hex_encode"),
        "direct dependency not pulled: {names:?}"
    );

    // The helpers must NOT also land in the heuristic functions bucket.
    let functions = find(&modules, "functions");
    let fn_names = standalone_names(functions);
    assert!(fn_names.contains("unrelated"));
    assert!(!fn_names.contains("hash_bytes"));
    assert!(!fn_names.contains("hex_encode"));
}

#[test]
fn global_seeded_mode_pulls_without_per_rule_flag() {
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["compute_md5".to_string()],
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(SEED_FIXTURE, rules, true);
    let modules = analyzer.group_by_module(500);
    let names = standalone_names(find(&modules, "hash"));
    assert!(names.contains("hash_bytes"), "{names:?}");
    assert!(names.contains("hex_encode"), "{names:?}");
}

#[test]
fn heuristic_mode_without_pull_leaves_helpers_in_functions() {
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["compute_md5".to_string()],
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(SEED_FIXTURE, rules, false);
    let modules = analyzer.group_by_module(500);
    let names = standalone_names(find(&modules, "hash"));
    assert!(names.contains("compute_md5"));
    assert!(!names.contains("hash_bytes"), "opt-in only: {names:?}");
    let fn_names = standalone_names(find(&modules, "functions"));
    assert!(fn_names.contains("hash_bytes"));
}

#[test]
fn seeding_reaches_fixpoint_over_chains() {
    // a (seed) -> b -> c: wave 1 pulls b, wave 2 pulls c.
    let code = r#"
        pub fn entry_a() -> u64 { mid_b() }
        fn mid_b() -> u64 { leaf_c() }
        fn leaf_c() -> u64 { 3 }
        fn island() -> u64 { 9 }
    "#;
    let rules = vec![TargetModule {
        name: "chain".to_string(),
        items: vec!["entry_a".to_string()],
        pull_dependencies: true,
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(code, rules, false);
    let modules = analyzer.group_by_module(500);
    let names = standalone_names(find(&modules, "chain"));
    assert!(names.contains("mid_b"), "{names:?}");
    assert!(
        names.contains("leaf_c"),
        "fixpoint wave 2 missed: {names:?}"
    );
    // Zero-affinity items stay heuristic.
    let fn_names = standalone_names(find(&modules, "functions"));
    assert!(fn_names.contains("island"));
}

#[test]
fn seeding_terminates_on_mutual_recursion() {
    let code = r#"
        pub fn seed() -> u64 { ping(0) }
        fn ping(x: u64) -> u64 { if x == 0 { pong(1) } else { x } }
        fn pong(x: u64) -> u64 { ping(x - 1) }
    "#;
    let rules = vec![TargetModule {
        name: "game".to_string(),
        items: vec!["seed".to_string()],
        pull_dependencies: true,
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(code, rules, false);
    let modules = analyzer.group_by_module(500);
    let names = standalone_names(find(&modules, "game"));
    assert!(names.contains("ping"), "{names:?}");
    assert!(names.contains("pong"), "{names:?}");
}

#[test]
fn seeding_pulls_types_with_their_impls() {
    let code = r#"
        pub fn monitor_start(cfg: &MonitorConfig) -> bool { cfg.enabled }

        pub struct MonitorConfig {
            pub enabled: bool,
        }

        impl MonitorConfig {
            pub fn new() -> Self { Self { enabled: true } }
        }

        impl Default for MonitorConfig {
            fn default() -> Self { Self::new() }
        }

        pub struct Unrelated;
    "#;
    let rules = vec![TargetModule {
        name: "monitor".to_string(),
        items: vec!["monitor_*".to_string()],
        pull_dependencies: true,
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(code, rules, false);
    let modules = analyzer.group_by_module(500);
    let monitor = find(&modules, "monitor");
    let type_names: HashSet<_> = monitor.types.iter().map(|t| t.name.clone()).collect();
    assert!(type_names.contains("MonitorConfig"), "{type_names:?}");
    let cfg = monitor
        .types
        .iter()
        .find(|t| t.name == "MonitorConfig")
        .expect("MonitorConfig");
    assert_eq!(
        cfg.impls.len(),
        1,
        "inherent impl must travel with the type"
    );
    assert_eq!(cfg.trait_impls.len(), 1, "trait impl must travel too");
    assert!(!type_names.contains("Unrelated"));
}

#[test]
fn seeded_grouping_is_deterministic() {
    let run = || -> Vec<(String, Vec<String>)> {
        let rules = vec![TargetModule {
            name: "hash".to_string(),
            items: vec!["compute_md5".to_string()],
            pull_dependencies: true,
            ..Default::default()
        }];
        let (_, analyzer) = analyzer_with(SEED_FIXTURE, rules, true);
        analyzer
            .group_by_module(500)
            .iter()
            .map(|m| {
                let mut exports = m.get_exported_types();
                exports.sort();
                (m.name.clone(), exports)
            })
            .collect()
    };
    assert_eq!(
        run(),
        run(),
        "two identical runs must produce identical grouping"
    );
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

#[test]
fn unknown_exact_item_is_hard_error_with_suggestions() {
    let (_, analyzer) = analyzer_with(SEED_FIXTURE, vec![], false);
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["compute_md6".to_string()],
        ..Default::default()
    }];
    let err = check_unmatched_patterns(&routable_names(&analyzer), &rules)
        .expect_err("unknown exact item must fail")
        .to_string();
    assert!(err.contains("compute_md6"), "{err}");
    assert!(
        err.contains("compute_md5"),
        "no near-miss suggestion: {err}"
    );
}

#[test]
fn early_wildcard_rule_is_rejected() {
    let rules = vec![
        TargetModule {
            name: "everything".to_string(),
            items: vec!["*".to_string()],
            ..Default::default()
        },
        TargetModule {
            name: "dead".to_string(),
            items: vec!["Foo".to_string()],
            ..Default::default()
        },
    ];
    assert!(validate_target_modules(&rules).is_err());
}

// ---------------------------------------------------------------------------
// Per-rule doc and max_lines
// ---------------------------------------------------------------------------

#[test]
fn per_rule_doc_becomes_module_header() {
    let rules = vec![TargetModule {
        name: "hash".to_string(),
        items: vec!["compute_md5".to_string()],
        doc: Some("Hashing (md5/sha) helpers".to_string()),
        ..Default::default()
    }];
    let (file, analyzer) = analyzer_with(SEED_FIXTURE, rules, false);
    let modules = analyzer.group_by_module(500);
    let hash = find(&modules, "hash");
    let content = hash.generate_content(
        &file,
        &analyzer.use_statements,
        &HashMap::new(),
        &HashSet::new(),
        None,
        &HashMap::new(),
        None,
    );
    assert!(
        content.starts_with("//! Hashing (md5/sha) helpers"),
        "custom doc header missing:\n{content}"
    );
}

#[test]
fn per_rule_max_lines_overflows_into_suffixed_modules() {
    let code = r#"
        pub fn big_a() { let _ = 1; let _ = 2; let _ = 3; let _ = 4; }
        pub fn big_b() { let _ = 1; let _ = 2; let _ = 3; let _ = 4; }
        pub fn big_c() { let _ = 1; let _ = 2; let _ = 3; let _ = 4; }
        pub fn big_d() { let _ = 1; let _ = 2; let _ = 3; let _ = 4; }
    "#;
    let rules = vec![TargetModule {
        name: "big".to_string(),
        items: vec!["big_*".to_string()],
        max_lines: Some(8),
        ..Default::default()
    }];
    let (_, analyzer) = analyzer_with(code, rules, false);
    let modules = analyzer.group_by_module(500);
    let names: Vec<&String> = modules.iter().map(|m| &m.name).collect();
    assert!(names.contains(&&"big".to_string()), "{names:?}");
    assert!(names.contains(&&"big_2".to_string()), "{names:?}");
    // All four functions present overall, none duplicated.
    let mut all: Vec<String> = modules
        .iter()
        .filter(|m| m.name.starts_with("big"))
        .flat_map(|m| standalone_names(m).into_iter())
        .collect();
    all.sort();
    assert_eq!(all, vec!["big_a", "big_b", "big_c", "big_d"]);
}

// ---------------------------------------------------------------------------
// F1 x F2 composition: parent = "core"
// ---------------------------------------------------------------------------

const PARENT_FIXTURE: &str = r#"
pub mod core {
    /// Copy one file.
    pub fn copy_file(src: &str, dst: &str) -> bool {
        !src.is_empty() && !dst.is_empty()
    }

    pub fn copy_tree(src: &str) -> bool {
        copy_file(src, src)
    }

    pub fn compute_hash(data: &[u8]) -> u64 {
        data.len() as u64
    }

    pub fn other_thing() -> u8 { 1 }
}
"#;

#[test]
fn parent_rules_route_inside_descended_mod() {
    let rules = vec![
        TargetModule {
            name: "fs".to_string(),
            items: vec!["copy_*".to_string()],
            parent: Some("core".to_string()),
            ..Default::default()
        },
        TargetModule {
            name: "hash".to_string(),
            items: vec!["*hash*".to_string()],
            parent: Some("core".to_string()),
            ..Default::default()
        },
    ];
    let file = syn::parse_file(PARENT_FIXTURE).expect("parse");
    let mod_item = file
        .items
        .into_iter()
        .find_map(|i| match i {
            Item::Mod(m) => Some(m),
            _ => None,
        })
        .expect("mod core");
    let opts = NestedSplitOptions {
        split_impl_blocks: false,
        max_impl_lines: 500,
        max_lines: 5,
        extract_tests: false,
        max_mod_depth: 8,
        seeded_assignment: false,
        all_rules: &rules,
    };
    let plan = plan_nested_split(&mod_item, PARENT_FIXTURE, &opts, "core", 1).expect("plan");

    let fs_mod = find(&plan.modules, "fs");
    let fs_names = standalone_names(fs_mod);
    assert!(fs_names.contains("copy_file"), "{fs_names:?}");
    assert!(fs_names.contains("copy_tree"), "{fs_names:?}");

    let hash_mod = find(&plan.modules, "hash");
    assert!(standalone_names(hash_mod).contains("compute_hash"));

    // Unlisted item falls through to the heuristic bucket at this level.
    let functions = find(&plan.modules, "functions");
    assert!(standalone_names(functions).contains("other_thing"));

    // Emit and verify core/fs.rs really exists.
    let dir = TempDir::new().expect("tempdir");
    write_plan(&plan, dir.path(), splitrs::config::FacadeStyle::Glob).expect("write");
    assert!(dir.path().join("core").join("fs.rs").exists());
    assert!(dir.path().join("core").join("hash.rs").exists());
}

#[test]
fn parent_rules_do_not_apply_at_top_level() {
    // A rule with parent = "core" must not capture a same-named top-level fn.
    let code = r#"
        pub fn copy_file() -> bool { true }
    "#;
    let rules = [TargetModule {
        name: "fs".to_string(),
        items: vec!["copy_*".to_string()],
        parent: Some("core".to_string()),
        ..Default::default()
    }];
    // Top level installs only parent-less rules (mirrors main.rs wiring).
    let top_rules: Vec<TargetModule> = rules
        .iter()
        .filter(|r| r.parent.is_none())
        .cloned()
        .collect();
    let (_, analyzer) = analyzer_with(code, top_rules, false);
    let modules = analyzer.group_by_module(500);
    assert!(
        modules.iter().all(|m| m.name != "fs"),
        "parent-scoped rule leaked to top level"
    );
}

// ---------------------------------------------------------------------------
// CLI end-to-end: --target-modules composing with --split-nested-mods
// ---------------------------------------------------------------------------

#[test]
fn cli_target_modules_with_parent_and_nested_split() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, PARENT_FIXTURE).expect("write fixture");
    let spec = dir.path().join("domains.toml");
    fs::write(
        &spec,
        r#"
[[target_modules]]
name = "fs"
parent = "core"
items = ["copy_*"]
doc = "Filesystem helpers"

[[target_modules]]
name = "hash"
parent = "core"
items = ["*hash*"]
"#,
    )
    .expect("write spec");
    let output = dir.path().join("out");

    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--split-nested-mods")
        .arg("true")
        .arg("--max-lines")
        .arg("5")
        .arg("--target-modules")
        .arg(&spec)
        .output()
        .expect("run splitrs");
    assert!(
        cmd.status.success(),
        "splitrs failed: {}\n{}",
        String::from_utf8_lossy(&cmd.stdout),
        String::from_utf8_lossy(&cmd.stderr)
    );

    let fs_rs = fs::read_to_string(output.join("core").join("fs.rs")).expect("core/fs.rs");
    assert!(fs_rs.contains("copy_file"), "{fs_rs}");
    assert!(
        fs_rs.starts_with("//! Filesystem helpers"),
        "per-rule doc missing:\n{fs_rs}"
    );
    assert!(output.join("core").join("hash.rs").exists());
    let mod_rs = fs::read_to_string(output.join("mod.rs")).expect("mod.rs");
    assert!(mod_rs.contains("pub mod core;"), "{mod_rs}");
}

#[test]
fn cli_parent_rule_without_nested_split_is_an_error() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, PARENT_FIXTURE).expect("write fixture");
    let spec = dir.path().join("domains.toml");
    fs::write(
        &spec,
        r#"
[[target_modules]]
name = "fs"
parent = "core"
items = ["copy_*"]
"#,
    )
    .expect("write spec");
    let output = dir.path().join("out");

    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--target-modules")
        .arg(&spec)
        .output()
        .expect("run splitrs");
    assert!(
        !cmd.status.success(),
        "parent rules without --split-nested-mods must fail"
    );
    let stderr = String::from_utf8_lossy(&cmd.stderr);
    assert!(stderr.contains("split-nested-mods"), "{stderr}");
}

#[test]
fn cli_unknown_exact_item_fails_with_suggestion() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(
        &input,
        "pub fn compute_md5() -> u64 { 0 }\npub fn other() {}\n",
    )
    .expect("write fixture");
    let spec = dir.path().join("domains.toml");
    fs::write(
        &spec,
        r#"
[[target_modules]]
name = "hash"
items = ["compute_md6"]
"#,
    )
    .expect("write spec");
    let output = dir.path().join("out");

    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--target-modules")
        .arg(&spec)
        .output()
        .expect("run splitrs");
    assert!(!cmd.status.success(), "unknown item must be a hard error");
    let stderr = String::from_utf8_lossy(&cmd.stderr);
    assert!(stderr.contains("compute_md6"), "{stderr}");
    assert!(stderr.contains("compute_md5"), "no suggestion: {stderr}");
}

#[test]
fn standalone_spec_assign_unlisted_seeded_via_cli() {
    let dir = TempDir::new().expect("tempdir");
    let input = dir.path().join("lib_fixture.rs");
    fs::write(&input, SEED_FIXTURE).expect("write fixture");
    let spec = dir.path().join("domains.toml");
    fs::write(
        &spec,
        r#"
assign_unlisted = "seeded"

[[target_modules]]
name = "hash"
items = ["compute_md5"]
"#,
    )
    .expect("write spec");
    let output = dir.path().join("out");

    let cmd = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("-i")
        .arg(&input)
        .arg("-o")
        .arg(&output)
        .arg("--target-modules")
        .arg(&spec)
        .output()
        .expect("run splitrs");
    assert!(
        cmd.status.success(),
        "splitrs failed: {}",
        String::from_utf8_lossy(&cmd.stderr)
    );
    let hash_rs = fs::read_to_string(output.join("hash.rs")).expect("hash.rs");
    assert!(hash_rs.contains("compute_md5"), "{hash_rs}");
    assert!(
        hash_rs.contains("hash_bytes"),
        "seeded helper missing from hash.rs:\n{hash_rs}"
    );
}

#[test]
fn spec_file_parses_extended_schema_from_disk() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("spec.toml");
    fs::write(
        &path,
        r#"
assign_unlisted = "seeded"

[[target_modules]]
name = "fs"
parent = "core"
items = ["copy_*", "move_*", "FsEntry*"]
pull_dependencies = true
doc = "Filesystem domain"
max_lines = 1200
"#,
    )
    .expect("write");
    let spec = TargetModulesFile::from_file(&path).expect("load");
    assert_eq!(spec.assign_unlisted.as_deref(), Some("seeded"));
    assert_eq!(spec.target_modules.len(), 1);
    assert_eq!(spec.target_modules[0].parent.as_deref(), Some("core"));
    assert!(spec.target_modules[0].pull_dependencies);
}
