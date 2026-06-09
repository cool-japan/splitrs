//! Integration tests for the SMT-verified extraction CLI wiring (`smt` feature).
//!
//! Phase 5 wires the proven function-extraction transform into the SplitRS CLI
//! via `--extract-pure` (an SMT-verified pre-pass) and adds an honest
//! `--verify` semantic-verification report. These tests exercise the *real*
//! `main.rs` wiring by spawning the built binary (`CARGO_BIN_EXE_splitrs`) so
//! that the committed helper genuinely flows through the
//! FileAnalyzer/group_by_module/write pipeline, and the report is the one the
//! user actually sees.
//!
//! A complementary library-level test drives `extract_pure_blocks` directly and
//! re-feeds the mutated AST through `FileAnalyzer` to assert the proven helper
//! survives the split as a real item — independent of stdout scraping.
//!
//! Temp files use the OS temp dir per project policy.

#![cfg(feature = "smt")]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use splitrs::extraction::{extract_pure_blocks, ExtractionOutcome};
use splitrs::file_analyzer::FileAnalyzer;

/// A pure fixed-width-integer function. Its body is one maximal straight-line
/// run of supported ops (`+ - * & | ^ <<`), so with a tiny `--max-lines` budget
/// the whole body is provably extractable into a helper.
const PURE_FN: &str = "\
fn mix(a: u32, b: u32) -> u32 {
    let s = a + b;
    let t = s * 3;
    let u = t ^ a;
    let v = u + b;
    let w = v & a;
    let x = w | b;
    let y = x << 1;
    let z = y + s;
    z
}
";

/// Build a unique temp working directory under the OS temp dir.
fn unique_tmp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "splitrs_extract_pure_{tag}_{}_{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::create_dir_all(&dir).expect("create temp working dir");
    dir
}

/// Recursively collect every `*.rs` file beneath `dir`.
fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().map(|e| e == "rs").unwrap_or(false) {
            out.push(path);
        }
    }
}

/// 1. Full pipeline: run the extraction pre-pass + split on a temp file with a
///    pure-integer function and a tiny `max_lines`, then assert the output
///    modules contain the proven helper fn and that EVERY written file
///    `syn::parse_file`-round-trips as valid Rust.
#[test]
fn extract_pure_pipeline_emits_proven_helper_and_round_trips() {
    let work = unique_tmp_dir("pipeline");
    let src = work.join("calc.rs");
    let out = work.join("out");
    fs::write(&src, PURE_FN).expect("write source fixture");

    let status = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("--input")
        .arg(&src)
        .arg("--output")
        .arg(&out)
        .arg("--max-lines")
        .arg("1")
        .arg("--extract-pure")
        .output()
        .expect("spawn splitrs binary");

    assert!(
        status.status.success(),
        "splitrs --extract-pure should exit 0.\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&status.stdout),
        String::from_utf8_lossy(&status.stderr)
    );

    // The proven helper must appear somewhere in the generated module tree.
    let mut rs_files = Vec::new();
    collect_rs_files(&out, &mut rs_files);
    assert!(
        !rs_files.is_empty(),
        "expected generated .rs files under {out:?}"
    );

    let mut helper_seen = false;
    for file in &rs_files {
        let content = fs::read_to_string(file).expect("read generated module");
        // Every written file must be valid Rust (round-trips through syn).
        syn::parse_file(&content)
            .unwrap_or_else(|e| panic!("generated file {file:?} must re-parse as valid Rust: {e}"));
        if content.contains("mix_extracted") {
            helper_seen = true;
        }
    }
    assert!(
        helper_seen,
        "the SMT-proven helper `mix_extracted` must appear in the split output \
         (files: {rs_files:?})"
    );

    let _ = fs::remove_dir_all(&work);
}

/// 1b. Library-level companion: drive `extract_pure_blocks` directly, then feed
///     the mutated AST back through `FileAnalyzer`/`group_by_module` and assert
///     the proven helper survives as a real standalone item (no stdout scrape).
#[test]
fn extract_pure_helper_survives_split_at_library_level() {
    let mut file: syn::File = syn::parse_str(PURE_FN).expect("fixture parses");
    let outcomes = extract_pure_blocks(&mut file, 1);

    let helper_ident = outcomes
        .iter()
        .find_map(|o| match o {
            ExtractionOutcome::Committed { helper_ident, .. } => Some(helper_ident.clone()),
            _ => None,
        })
        .expect("a Committed extraction must have produced a helper");
    assert!(
        helper_ident.starts_with("mix_extracted"),
        "helper should be named after the origin, got {helper_ident}"
    );

    // Re-run the split over the MUTATED file: the helper must be picked up as a
    // standalone item and distributed into a module.
    let mut analyzer = FileAnalyzer::new(false, 500);
    analyzer.analyze(&file);
    let modules = analyzer.group_by_module(1);

    let helper_in_modules = modules.iter().any(|m| {
        m.standalone_items.iter().any(|item| match item {
            syn::Item::Fn(f) => f.sig.ident == helper_ident.as_str(),
            _ => false,
        })
    });
    assert!(
        helper_in_modules,
        "the proven helper `{helper_ident}` must survive the split as a real item"
    );
}

/// 2. `--verify` report: assert the report text contains the SMT-Verified line
///    for the committed extraction AND the structural-identity statement for the
///    moved items. Driven via the binary so we assert on the report the user
///    actually sees; exit code must be 0.
#[test]
fn verify_report_distinguishes_proven_from_structural() {
    let work = unique_tmp_dir("verify");
    let src = work.join("calc.rs");
    let out = work.join("out");
    fs::write(&src, PURE_FN).expect("write source fixture");

    let result = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("--input")
        .arg(&src)
        .arg("--output")
        .arg(&out)
        .arg("--max-lines")
        .arg("1")
        .arg("--extract-pure")
        .arg("--verify")
        .output()
        .expect("spawn splitrs binary");

    assert!(
        result.status.success(),
        "splitrs --extract-pure --verify should exit 0.\nstderr:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);

    // Report section header is present.
    assert!(
        stdout.contains("Semantic verification report"),
        "missing report header.\nstdout:\n{stdout}"
    );

    // SMT-Verified line for the committed extraction (proven half).
    assert!(
        stdout.contains("SMT-Verified (body rewrite):"),
        "missing SMT-Verified section.\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("mix: function extraction — SMT-Verified equivalent (QF_BV, all inputs)")
            && stdout.contains("helper mix_extracted"),
        "missing the SMT-Verified line for `mix` -> `mix_extracted`.\nstdout:\n{stdout}"
    );

    // Structural-identity statement (assumed, NOT proven half) — exact framing.
    assert!(
        stdout.contains("Structural identity (NOT SMT-proven):"),
        "missing structural-identity section.\nstdout:\n{stdout}"
    );
    assert!(
        stdout.contains("items relocated unchanged (structural identity)")
            && stdout.contains("NOT proven by SMT"),
        "missing the honest structural-identity statement.\nstdout:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&work);
}

/// 2b. `--verify` WITHOUT `--extract-pure`: the report still prints the
///     structural-identity statement and notes that `--extract-pure` is where
///     SMT proofs apply (no committed extraction in the proven section).
#[test]
fn verify_report_without_extract_pure_is_structural_only() {
    let work = unique_tmp_dir("verify_only");
    let src = work.join("plain.rs");
    let out = work.join("out");
    fs::write(
        &src,
        "pub struct Foo { x: u32 }\npub struct Bar { y: u32 }\npub fn helper() -> u32 { 42 }\n",
    )
    .expect("write source fixture");

    let result = Command::new(env!("CARGO_BIN_EXE_splitrs"))
        .arg("--input")
        .arg(&src)
        .arg("--output")
        .arg(&out)
        .arg("--max-lines")
        .arg("1")
        .arg("--verify")
        .output()
        .expect("spawn splitrs binary");

    assert!(
        result.status.success(),
        "splitrs --verify (no extract) should exit 0.\nstderr:\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8_lossy(&result.stdout);

    assert!(
        stdout.contains("Semantic verification report"),
        "missing report header.\nstdout:\n{stdout}"
    );
    // No SMT-Verified commit; the proven section points at --extract-pure.
    assert!(
        stdout.contains("`--extract-pure` was not requested"),
        "report should note that --extract-pure is where SMT proofs apply.\nstdout:\n{stdout}"
    );
    // The structural-identity statement is still present.
    assert!(
        stdout.contains("items relocated unchanged (structural identity)")
            && stdout.contains("NOT proven by SMT"),
        "structural-identity statement must always print.\nstdout:\n{stdout}"
    );

    let _ = fs::remove_dir_all(&work);
}
