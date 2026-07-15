//! Regression tests for two real defects found (2026-07-15) while splitting a
//! ~1900-line file with `splitrs --split-test-modules`:
//!
//!   1. **Silent inline-comment loss** — production items were regenerated
//!      through `prettyplease`, which drops every non-doc `//`/`/* */` comment
//!      (a ~90-line rationale block vanished). These tests assert every inline
//!      comment resurfaces in the split output.
//!   2. **build-passes / nextest-fails split** — the extracted `#[cfg(test)]`
//!      modules landed one level deeper without a file-level `use super::*;`, so
//!      they could not resolve the parent module's production items. `cargo
//!      build` skips `#[cfg(test)]` and stayed green; `cargo nextest run
//!      --no-run` / `cargo test --no-run` failed with E0422/E0425. These tests
//!      compile the emitted tree as a **test target** (`rustc --test`) so the
//!      regression is caught.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Run `splitrs --split-test-modules <file>` on `code` written to `sample.rs`,
/// returning the generated `<dir>/sample/` module directory.
fn run_split_test_modules(dir: &TempDir, code: &str) -> std::path::PathBuf {
    let input = dir.path().join("sample.rs");
    fs::write(&input, code).expect("write fixture");
    let out = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("--split-test-modules")
        .arg(&input)
        .output()
        .expect("run splitrs");
    assert!(
        out.status.success(),
        "splitrs --split-test-modules failed:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    dir.path().join("sample")
}

/// Concatenated content of every `.rs` file directly inside `dir`.
fn read_combined(dir: &Path) -> String {
    let mut combined = String::new();
    for entry in fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("dir entry").path();
        if path.is_file() && path.extension().is_some_and(|e| e == "rs") {
            combined.push_str(&fs::read_to_string(&path).expect("read file"));
            combined.push('\n');
        }
    }
    combined
}

/// Compile the emitted module tree as a **test crate** (`rustc --test`), which —
/// unlike `cargo build` — actually compiles `#[cfg(test)]` code. `--emit=metadata`
/// runs full name/type resolution (catching E0422/E0425/E0432) without linking.
fn assert_test_target_compiles(mod_dir: &Path) {
    let parent = mod_dir.parent().expect("mod dir has a parent");
    let mod_name = mod_dir
        .file_name()
        .and_then(|n| n.to_str())
        .expect("mod dir name is utf-8");
    let probe = parent.join("probe.rs");
    fs::write(
        &probe,
        format!("#[path = \"{mod_name}/mod.rs\"]\nmod split_output;\n"),
    )
    .expect("write probe");
    let out = Command::new("rustc")
        .arg("--edition")
        .arg("2021")
        .arg("--test")
        .arg("--emit=metadata")
        .arg("--out-dir")
        .arg(parent)
        .arg(&probe)
        .output()
        .expect("run rustc");
    assert!(
        out.status.success(),
        "emitted test target does not compile (imports unresolved?):\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ---------------------------------------------------------------------------
// Single test module -> tests.rs + mod.rs fallback
// ---------------------------------------------------------------------------

const SINGLE_MODULE_FIXTURE: &str = r#"// Copyright header line (production).

// Design rationale block (free-standing, belongs to no AST node).
// It documents WHY `saturating_add` is used below and MUST survive the split.
// Removing it silently would destroy load-bearing design intent.

/// Saturating adder.
pub fn clamp_add(a: u32, b: u32) -> u32 {
    // saturating on purpose — do not "simplify" to a plain `+`
    a.saturating_add(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_instead_of_wrapping() {
        // rationale: adding past u32::MAX must clamp, not wrap
        assert_eq!(clamp_add(u32::MAX, 10), u32::MAX);
    }
}
"#;

#[test]
fn single_module_split_preserves_comments_and_compiles_as_test() {
    let dir = TempDir::new().expect("tempdir");
    let mod_dir = run_split_test_modules(&dir, SINGLE_MODULE_FIXTURE);

    let mod_rs = fs::read_to_string(mod_dir.join("mod.rs")).expect("mod.rs");
    let tests_rs = fs::read_to_string(mod_dir.join("tests.rs")).expect("tests.rs");

    // (1) Free-standing + inline production comments survive in mod.rs.
    assert!(
        mod_rs.contains("// Design rationale block (free-standing, belongs to no AST node)."),
        "free-standing rationale block lost from mod.rs:\n{mod_rs}"
    );
    assert!(
        mod_rs.contains("// It documents WHY `saturating_add` is used below and MUST survive"),
        "rationale line lost from mod.rs:\n{mod_rs}"
    );
    assert!(
        mod_rs.contains(r#"// saturating on purpose — do not "simplify" to a plain `+`"#),
        "inline body comment lost from mod.rs:\n{mod_rs}"
    );

    // (1b) The test body's inline comment survives in tests.rs.
    assert!(
        tests_rs.contains("// rationale: adding past u32::MAX must clamp, not wrap"),
        "inline comment inside the test body lost from tests.rs:\n{tests_rs}"
    );

    // (2) The emitted tree compiles as a TEST target (cfg(test) active).
    assert_test_target_compiles(&mod_dir);
}

// ---------------------------------------------------------------------------
// Multiple test modules -> one file each + mod.rs (the bug #2 path)
// ---------------------------------------------------------------------------

const MULTI_MODULE_FIXTURE: &str = r#"// Free-standing production comment at the top of the file.

pub fn helper_double(x: u32) -> u32 {
    x * 2 // inline: doubling helper used by the tests below
}

/// A production widget referenced from a test module.
pub struct Widget {
    // field note: kept crate-private on purpose
    pub value: u32,
}

#[cfg(test)]
mod tests_helper {
    use super::*;

    #[test]
    fn helper_doubles() {
        // rationale: verify the parent `helper_double`
        assert_eq!(helper_double(21), 42);
    }
}

#[cfg(test)]
mod tests_widget {
    use super::*;

    #[test]
    fn widget_builds() {
        // rationale: verify parent `Widget` construction
        let w = Widget { value: 5 };
        assert_eq!(w.value, 5);
    }
}
"#;

#[test]
fn multi_module_split_resolves_imports_and_preserves_comments() {
    let dir = TempDir::new().expect("tempdir");
    let mod_dir = run_split_test_modules(&dir, MULTI_MODULE_FIXTURE);

    // Two per-module files + mod.rs were emitted.
    assert!(
        mod_dir.join("tests_helper.rs").exists(),
        "tests_helper.rs missing"
    );
    assert!(
        mod_dir.join("tests_widget.rs").exists(),
        "tests_widget.rs missing"
    );
    assert!(mod_dir.join("mod.rs").exists(), "mod.rs missing");

    let combined = read_combined(&mod_dir);

    // (1) Production + test comments survive somewhere in the output.
    for needle in [
        "// Free-standing production comment at the top of the file.",
        "// inline: doubling helper used by the tests below",
        "// field note: kept crate-private on purpose",
        "// rationale: verify the parent `helper_double`",
        "// rationale: verify parent `Widget` construction",
    ] {
        assert!(
            combined.contains(needle),
            "comment lost from split output: {needle:?}\n--- combined ---\n{combined}"
        );
    }

    // (2) The moved test modules resolve `helper_double`/`Widget` through the
    // file-level `use super::*;`. This is exactly what `cargo build` cannot
    // catch but `nextest --no-run` (a test-target compile) does.
    assert_test_target_compiles(&mod_dir);
}
