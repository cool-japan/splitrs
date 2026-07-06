//! Acceptance layer for the nested-mod descent (`--split-nested-mods`) and
//! TOML domain-mapping features, exercised through the real binary.
//!
//! Two tiers:
//!
//! 1. A hermetic mini-crate reproducing the fvrs-core failure shape (one
//!    dominant inline `pub mod core` holding several domains). The emitted
//!    tree is assembled into a real crate inside a temp sandbox and must pass
//!    `cargo check`, proving that descent + mapping + facade produce code the
//!    compiler accepts and that historical `crate::core::*` paths survive.
//!
//! 2. A repository-backed smoke test (`#[ignore]` by default) that replays
//!    the actual historical fvrs-core monolith through an eight-domain map
//!    and asserts the emission distributes items across the named modules.
//!    The historical file has unrelated platform-specific bugs, so this tier
//!    only asserts parseable emission and distribution — never `cargo check`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Mini fvrs-core: one dominant inline `pub mod core` with four domains
/// (fs / hash / compare / monitor), cross-domain calls, shared private
/// helpers, use statements, doc comments, and a `#[cfg(windows)]` item.
/// Top-level consumers reference items through `crate::core::*` so the
/// compile step proves the historical paths survive the split.
const MINI_MONOLITH: &str = r#"
//! Mini fvrs-core: one dominant inline `pub mod core` holding four domains.

use std::collections::HashMap;
use std::path::PathBuf;

/// Boot the app through the historical `crate::core::*` paths.
pub fn boot() -> core::HashResult {
    let entry = crate::core::FileEntry::new(PathBuf::from("a"), 3);
    crate::core::compute_hash(&entry)
}

/// Consumer proving `crate::core::*` paths survive the split.
pub fn consume() -> crate::core::ComparisonResult {
    let a = crate::core::FileEntry::new(PathBuf::from("a"), 1);
    let b = crate::core::FileEntry::new(PathBuf::from("b"), 1);
    crate::core::compare_entries(&a, &b)
}

/// Core module holding every domain of the mini app.
pub mod core {
    //! Inner core docs.
    use super::*;

    /// Shared private helper used by the fs and compare domains.
    fn normalize(path: &PathBuf) -> String {
        path.to_string_lossy().to_ascii_lowercase()
    }

    /// A filesystem entry (fs domain).
    #[derive(Debug, Clone)]
    pub struct FileEntry {
        pub path: PathBuf,
        pub size: u64,
    }

    impl FileEntry {
        /// Create a new entry.
        pub fn new(path: PathBuf, size: u64) -> Self {
            Self { path, size }
        }

        /// Normalized display key.
        pub fn key(&self) -> String {
            normalize(&self.path)
        }
    }

    /// List entries below a size bound (fs domain).
    pub fn list_entries(index: &HashMap<String, u64>, bound: u64) -> usize {
        index.values().filter(|size| **size <= bound).count()
    }

    /// Windows-only attribute probe (fs domain).
    #[cfg(windows)]
    pub fn file_attributes(entry: &FileEntry) -> u32 {
        entry.size as u32
    }

    /// Hash outcome (hash domain).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct HashResult {
        pub value: u64,
    }

    fn mix(x: u64) -> u64 {
        x.rotate_left(7) ^ 0x9e37_79b9
    }

    /// Hash a single entry (hash domain).
    pub fn compute_hash(entry: &FileEntry) -> HashResult {
        HashResult {
            value: mix(entry.size) ^ entry.key().len() as u64,
        }
    }

    /// Comparison verdict (compare domain).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ComparisonResult {
        Same,
        Different,
    }

    /// Compare two entries via their hashes (cross-domain call).
    pub fn compare_entries(a: &FileEntry, b: &FileEntry) -> ComparisonResult {
        if normalize(&a.path) == normalize(&b.path) && compute_hash(a) == compute_hash(b) {
            ComparisonResult::Same
        } else {
            ComparisonResult::Different
        }
    }

    /// Watcher state (monitor domain).
    #[derive(Debug, Default)]
    pub struct MonitorState {
        pub events: Vec<String>,
    }

    impl MonitorState {
        /// Record one event line.
        pub fn record(&mut self, line: String) {
            self.events.push(line);
        }
    }

    /// Watch a single entry (cross-domain: fs -> monitor).
    pub fn watch_entry(state: &mut MonitorState, entry: &FileEntry) {
        state.record(entry.key());
    }
}
"#;

/// Domain map for [`MINI_MONOLITH`]: every rule routes inside the descended
/// `core` mod; ordering matters (first match wins).
const MINI_DOMAIN_SPEC: &str = r#"
[[target_modules]]
name = "hash"
parent = "core"
items = ["Hash*", "compute_hash"]
pull_dependencies = true
doc = "Hashing domain"

[[target_modules]]
name = "compare"
parent = "core"
items = ["Comparison*", "compare_*"]
doc = "Comparison domain"

[[target_modules]]
name = "monitor"
parent = "core"
items = ["Monitor*", "watch_*"]
doc = "Watcher domain"

[[target_modules]]
name = "fs"
parent = "core"
items = ["File*", "list_*", "file_attributes"]
doc = "Filesystem domain"
"#;

/// Eight-domain map for the historical fvrs-core monolith. Rule order is
/// significant: event/permission globs must win before the broad `File*` /
/// `Fs*` filesystem rule.
const FVRS_DOMAIN_SPEC: &str = r#"
[[target_modules]]
name = "permissions"
parent = "core"
items = ["*Permissions*"]
doc = "Per-platform permission handling"

[[target_modules]]
name = "search"
parent = "core"
items = ["Search*"]
doc = "Name/content search"

[[target_modules]]
name = "hash"
parent = "core"
items = ["Hash*", "*hash*"]
doc = "File and directory hashing"

[[target_modules]]
name = "compare"
parent = "core"
items = ["Comparison*", "Difference*", "compare_*"]
doc = "File and directory comparison"

[[target_modules]]
name = "monitor"
parent = "core"
items = ["*Event*", "Monitoring*", "*Monitor*", "watch_*"]
doc = "Watcher lifecycle and event delivery"

[[target_modules]]
name = "fs"
parent = "core"
items = ["File*", "Fs*"]
doc = "Filesystem operations"

[[target_modules]]
name = "config"
items = ["Config", "SortBy"]
doc = "Application configuration"

[[target_modules]]
name = "plugin"
items = ["*Plugin*"]
doc = "Plugin system"
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path:?}: {e}"))
}

/// Recursively assert that every generated `.rs` file parses as valid Rust.
fn assert_all_files_parse(dir: &Path) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        if path.is_dir() {
            assert_all_files_parse(&path);
        } else if path.extension().is_some_and(|e| e == "rs") {
            let content = read(&path);
            syn::parse_file(&content)
                .unwrap_or_else(|e| panic!("generated file {path:?} does not parse: {e}"));
        }
    }
}

/// Recursively copy the emitted tree into the sandbox crate's `src/`.
fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("mkdir {dst:?}: {e}"));
    for entry in fs::read_dir(src).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to);
        } else {
            fs::copy(&from, &to).unwrap_or_else(|e| panic!("copy {from:?} -> {to:?}: {e}"));
        }
    }
}

/// Run the built splitrs binary; panic (with full output) on failure.
fn run_splitrs(args: &[&std::ffi::OsStr]) {
    let output = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .args(args)
        .output()
        .expect("spawn splitrs");
    assert!(
        output.status.success(),
        "splitrs failed ({}):\n--- stdout ---\n{}\n--- stderr ---\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

/// The cargo that is driving this test run (falls back to `cargo` in PATH).
fn cargo_command() -> Command {
    match std::env::var_os("CARGO") {
        Some(cargo) => Command::new(cargo),
        None => Command::new("cargo"),
    }
}

/// Every `.rs` file directly inside `dir`, sorted by file name.
fn rs_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {dir:?}: {e}"))
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "rs"))
        .collect();
    files.sort();
    files
}

// ---------------------------------------------------------------------------
// Tier 1: hermetic end-to-end compile validation
// ---------------------------------------------------------------------------

#[test]
fn hermetic_core_descent_with_domain_map_compiles() {
    let sandbox = TempDir::new().expect("tempdir");
    let input = sandbox.path().join("lib_fixture.rs");
    fs::write(&input, MINI_MONOLITH).expect("write fixture");
    let spec = sandbox.path().join("domains.toml");
    fs::write(&spec, MINI_DOMAIN_SPEC).expect("write spec");
    let out = sandbox.path().join("out");

    run_splitrs(&[
        "-i".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--split-nested-mods".as_ref(),
        "true".as_ref(),
        "--max-lines".as_ref(),
        "30".as_ref(),
        "--target-modules".as_ref(),
        spec.as_os_str(),
    ]);

    // 1. Descent actually split INSIDE the mod: core/ is a directory with a
    //    file per domain, not one opaque `core.rs` re-dump.
    let core_dir = out.join("core");
    assert!(core_dir.is_dir(), "core must be emitted as a directory");
    assert!(
        !out.join("core.rs").exists(),
        "core must not be re-emitted as one opaque file"
    );
    let top_mod = read(&out.join("mod.rs"));
    assert!(
        !top_mod.contains("pub mod core {"),
        "core must not stay inline in mod.rs:\n{top_mod}"
    );

    // 2. The mapping puts each cluster into its named module.
    for (file, marker) in [
        ("fs.rs", "pub struct FileEntry"),
        ("hash.rs", "pub struct HashResult"),
        ("compare.rs", "pub enum ComparisonResult"),
        ("monitor.rs", "pub struct MonitorState"),
    ] {
        let content = read(&core_dir.join(file));
        assert!(
            content.contains(marker),
            "core/{file} must contain `{marker}`:\n{content}"
        );
    }
    // pull_dependencies drags the private `mix` helper into the hash domain.
    assert!(
        read(&core_dir.join("hash.rs")).contains("fn mix"),
        "pull_dependencies must move `mix` into core/hash.rs"
    );
    // The `#[cfg(windows)]` item travels with its fs cluster, cfg intact.
    assert!(
        read(&core_dir.join("fs.rs")).contains("#[cfg(windows)]"),
        "cfg(windows) attribute must be preserved in core/fs.rs"
    );

    // 3. Facade: the parent declares `pub mod core;` (with its doc comment)
    //    and must NOT re-export core's items at the root — the historical
    //    `crate::core::Item` paths depend on that.
    assert!(top_mod.contains("pub mod core;"), "{top_mod}");
    assert!(
        top_mod.contains("/// Core module holding every domain of the mini app."),
        "outer doc comment lost:\n{top_mod}"
    );
    assert!(
        !top_mod.contains("pub use core::*;"),
        "core items must stay under crate::core::*, never re-exported:\n{top_mod}"
    );
    let core_mod = read(&core_dir.join("mod.rs"));
    assert!(
        core_mod.contains("//! Inner core docs."),
        "inner mod docs lost:\n{core_mod}"
    );
    for decl in [
        "pub mod fs;",
        "pub mod hash;",
        "pub mod compare;",
        "pub mod monitor;",
    ] {
        assert!(core_mod.contains(decl), "missing `{decl}` in:\n{core_mod}");
    }
    assert!(
        core_mod.contains("pub use "),
        "glob facade must re-export domain items inside core/mod.rs:\n{core_mod}"
    );

    assert_all_files_parse(&out);

    // 4. The emitted crate COMPILES. Assemble a real crate in the sandbox:
    //    mod.rs becomes src/lib.rs, the rest of the tree is copied verbatim.
    let crate_dir = sandbox.path().join("emitted_crate");
    let src_dir = crate_dir.join("src");
    copy_tree(&out, &src_dir);
    fs::rename(src_dir.join("mod.rs"), src_dir.join("lib.rs")).expect("mod.rs -> lib.rs");
    fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"splitrs-hermetic-fixture\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .expect("write Cargo.toml");

    let check = cargo_command()
        .arg("check")
        .arg("--quiet")
        .current_dir(&crate_dir)
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("spawn cargo check");
    assert!(
        check.status.success(),
        "emitted crate does not compile:\n--- stdout ---\n{}\n--- stderr ---\n{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr),
    );
}

// ---------------------------------------------------------------------------
// Tier 2: real-world smoke against the historical fvrs-core monolith
// ---------------------------------------------------------------------------

/// The historical monolith is the newest revision of
/// `crates/fvrs-core/src/lib.rs` with at least this many lines (the current
/// HEAD is already refactored down to ~100 lines).
const MONOLITH_MIN_LINES: usize = 1000;

/// Locate the fvrs repository without hardcoding machine paths: honor
/// `FVRS_REPO`, then fall back to `<home>/work/fvrs`.
fn locate_fvrs_repo() -> PathBuf {
    if let Some(repo) = std::env::var_os("FVRS_REPO") {
        return PathBuf::from(repo);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .expect("neither FVRS_REPO nor HOME/USERPROFILE is set");
    PathBuf::from(home).join("work").join("fvrs")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git {args:?} failed in {repo:?}:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Walk the file's history newest-first and return the first revision that
/// still is the monolith.
fn historical_monolith(repo: &Path) -> String {
    let log = git_stdout(
        repo,
        &["log", "--format=%H", "--", "crates/fvrs-core/src/lib.rs"],
    );
    for commit in log.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let spec = format!("{commit}:crates/fvrs-core/src/lib.rs");
        let source = git_stdout(repo, &["show", &spec]);
        if source.lines().count() >= MONOLITH_MIN_LINES {
            return source;
        }
    }
    panic!(
        "no >= {MONOLITH_MIN_LINES}-line revision of crates/fvrs-core/src/lib.rs found in {repo:?}"
    );
}

/// Real-world smoke: the historical fvrs-core monolith must be distributed
/// across the eight named domains instead of being re-emitted as one file.
///
/// The historical source carries unrelated platform bugs (windows-only
/// imports on every target), so this test deliberately stops at parseable
/// emission + distribution and never runs `cargo check` on the output.
#[test]
#[ignore = "requires a local fvrs checkout (set FVRS_REPO, default <home>/work/fvrs)"]
fn real_world_fvrs_monolith_distributes_across_domains() {
    let repo = locate_fvrs_repo();
    assert!(
        repo.join(".git").exists(),
        "fvrs repository not found at {repo:?}; set FVRS_REPO"
    );
    let monolith = historical_monolith(&repo);

    let sandbox = TempDir::new().expect("tempdir");
    let input = sandbox.path().join("lib_monolith.rs");
    fs::write(&input, &monolith).expect("write monolith");
    let spec = sandbox.path().join("domains.toml");
    fs::write(&spec, FVRS_DOMAIN_SPEC).expect("write spec");
    let out = sandbox.path().join("out");

    run_splitrs(&[
        "-i".as_ref(),
        input.as_os_str(),
        "-o".as_ref(),
        out.as_os_str(),
        "--split-nested-mods".as_ref(),
        "true".as_ref(),
        "--max-lines".as_ref(),
        "300".as_ref(),
        "--target-modules".as_ref(),
        spec.as_os_str(),
    ]);

    // Distribution: every core-domain seed produced its named module with
    // the expected cluster inside.
    let core_dir = out.join("core");
    let expectations = [
        ("permissions.rs", "pub struct FilePermissions"),
        ("search.rs", "pub struct SearchOptions"),
        ("hash.rs", "pub enum HashAlgorithm"),
        ("compare.rs", "pub struct ComparisonResult"),
        ("monitor.rs", "pub struct MonitoringHistory"),
        ("fs.rs", "pub struct FileSystem"),
    ];
    for (file, marker) in expectations {
        let content = read(&core_dir.join(file));
        assert!(
            content.contains(marker),
            "core/{file} must contain `{marker}`"
        );
    }
    // Not one opaque file: the descended dir holds all domain files plus
    // its mod.rs, and no single file swallowed the whole mod again.
    let core_files = rs_files(&core_dir);
    assert!(
        core_files.len() > expectations.len(),
        "expected more than {} files under core/ (domains + mod.rs), got {core_files:?}",
        expectations.len()
    );
    let monolith_lines = monolith.lines().count();
    for file in &core_files {
        let lines = read(file).lines().count();
        assert!(
            lines < monolith_lines * 3 / 4,
            "{file:?} holds {lines} of {monolith_lines} monolith lines — not a split"
        );
    }

    // Top-level seeds: config routed by name; the tiny inline `mod plugin`
    // stays opaque (under budget) but must survive emission somewhere.
    let config_rs = read(&out.join("config.rs"));
    assert!(config_rs.contains("pub struct Config"), "{config_rs}");
    assert!(config_rs.contains("pub enum SortBy"), "{config_rs}");
    let top_mod = read(&out.join("mod.rs"));
    assert!(top_mod.contains("pub mod core;"), "{top_mod}");
    assert!(!top_mod.contains("pub use core::*;"), "{top_mod}");
    let mut plugin_home = None;
    for file in rs_files(&out) {
        if read(&file).contains("pub mod plugin") {
            plugin_home = Some(file);
            break;
        }
    }
    let plugin_home = plugin_home.expect("`pub mod plugin` lost during emission");

    // Emission must not error at the syntax level anywhere in the tree.
    assert_all_files_parse(&out);

    // Report the distribution for manual runs.
    println!("real-world fvrs monolith distribution ({monolith_lines} input lines):");
    for file in rs_files(&out).iter().chain(core_files.iter()) {
        let lines = read(file).lines().count();
        let rel = file.strip_prefix(&out).unwrap_or(file);
        println!("  {:>5} lines  {}", lines, rel.display());
    }
    println!(
        "  plugin mod preserved in {}",
        plugin_home
            .strip_prefix(&out)
            .unwrap_or(&plugin_home)
            .display()
    );
}
