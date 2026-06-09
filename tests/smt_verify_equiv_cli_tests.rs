//! Integration tests for the `--verify-equiv` CLI core (`smt` feature).
//!
//! Writes two `.rs` fixtures into a temp directory and drives the library
//! `run_verify_equiv_core` (the structured-return surface), asserting on the
//! outcome without scraping stdout. Temp files use the OS temp dir per policy.

#![cfg(feature = "smt")]

use std::fs;

use splitrs::smt_cli::{run_verify_equiv_core, VerifyOutcome};
use tempfile::TempDir;

/// Write `contents` to `<dir>/<name>` and return the path as a String.
fn write_fixture(dir: &TempDir, name: &str, contents: &str) -> String {
    let path = dir.path().join(name);
    fs::write(&path, contents).expect("fixture write should succeed");
    path.to_string_lossy().into_owned()
}

#[test]
fn equivalent_pair_verifies() {
    let dir = TempDir::new().expect("temp dir");
    let left = write_fixture(
        &dir,
        "left.rs",
        "pub fn add(a: u32, b: u32) -> u32 { a + b }\n",
    );
    let right = write_fixture(
        &dir,
        "right.rs",
        "pub fn add_swapped(a: u32, b: u32) -> u32 { b + a }\n",
    );

    let outcome = run_verify_equiv_core(&format!("{left}::add"), &format!("{right}::add_swapped"))
        .expect("verify-equiv core should run");
    assert!(
        matches!(outcome, VerifyOutcome::Verified),
        "expected Verified, got {outcome:?}"
    );
}

#[test]
fn non_equivalent_pair_refutes() {
    let dir = TempDir::new().expect("temp dir");
    let left = write_fixture(
        &dir,
        "sum.rs",
        "pub fn sum(a: u32, b: u32) -> u32 { a + b }\n",
    );
    let right = write_fixture(
        &dir,
        "diff.rs",
        "pub fn diff(a: u32, b: u32) -> u32 { a - b }\n",
    );

    let outcome = run_verify_equiv_core(&format!("{left}::sum"), &format!("{right}::diff"))
        .expect("verify-equiv core should run");
    match outcome {
        VerifyOutcome::Refuted(cx) => {
            assert_eq!(cx.inputs.len(), 2, "expected two input bindings");
        }
        other => panic!("expected Refuted, got {other:?}"),
    }
}

#[test]
fn missing_function_is_an_error() {
    let dir = TempDir::new().expect("temp dir");
    let left = write_fixture(&dir, "a.rs", "pub fn present(a: u32) -> u32 { a }\n");
    let right = write_fixture(&dir, "b.rs", "pub fn present(a: u32) -> u32 { a }\n");

    let result = run_verify_equiv_core(&format!("{left}::absent"), &format!("{right}::present"));
    assert!(result.is_err(), "missing function should be an error");
}
