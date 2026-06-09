//! CLI driver for the `--verify-equiv` mode.
//!
//! Parses two `FILE::FN` references, loads each function via `syn`, runs the
//! SMT equivalence oracle and prints the verdict. The core
//! [`run_verify_equiv_core`] returns a structured [`VerifyOutcome`] so tests
//! can assert on the result without scraping stdout.

use anyhow::{Context, Result};

use crate::smt::{prove_equivalent, Verdict};

/// The structured outcome of a verify-equiv run (mirrors [`Verdict`] but is the
/// stable surface tests assert against).
#[derive(Debug, Clone)]
pub enum VerifyOutcome {
    /// Proven equivalent for all inputs.
    Verified,
    /// Refuted with a concrete counterexample.
    Refuted(crate::smt::Counterexample),
    /// Outside the modelled fragment.
    Unsupported {
        /// Reason the proof could not be attempted.
        reason: String,
    },
}

impl From<Verdict> for VerifyOutcome {
    fn from(v: Verdict) -> Self {
        match v {
            Verdict::Verified => VerifyOutcome::Verified,
            Verdict::Refuted(cx) => VerifyOutcome::Refuted(cx),
            Verdict::Unsupported { reason } => VerifyOutcome::Unsupported { reason },
        }
    }
}

/// Entry point invoked from `main()`. Prints a human-readable report and
/// returns `Ok(())` for every legitimate verdict (including Unsupported, which
/// is not an error).
pub fn run_verify_equiv(left: Option<&str>, right: Option<&str>) -> Result<()> {
    let left = left.ok_or_else(|| {
        anyhow::anyhow!(
            "--verify-equiv requires --left FILE::FN.\n\
             Example: splitrs --verify-equiv --left a.rs::f --right b.rs::g"
        )
    })?;
    let right = right.ok_or_else(|| {
        anyhow::anyhow!(
            "--verify-equiv requires --right FILE::FN.\n\
             Example: splitrs --verify-equiv --left a.rs::f --right b.rs::g"
        )
    })?;

    let outcome = run_verify_equiv_core(left, right)?;
    print_outcome(&outcome);
    Ok(())
}

/// The testable core: parse both references, run the oracle, return the
/// structured outcome.
pub fn run_verify_equiv_core(left: &str, right: &str) -> Result<VerifyOutcome> {
    let (left_file, left_fn) = split_file_fn(left)?;
    let (right_file, right_fn) = split_file_fn(right)?;

    let fn_a = load_function(left_file, left_fn)?;
    let fn_b = load_function(right_file, right_fn)?;

    let verdict = prove_equivalent(&fn_a, &fn_b);
    Ok(verdict.into())
}

/// Split a `FILE::FN` spec on its LAST `::` (paths may themselves contain `::`).
fn split_file_fn(spec: &str) -> Result<(&str, &str)> {
    let idx = spec.rfind("::").ok_or_else(|| {
        anyhow::anyhow!(
            "expected FILE::FN (got `{spec}`).\n\
             Example: src/lib.rs::my_function"
        )
    })?;
    let file = &spec[..idx];
    let func = &spec[idx + 2..];
    if file.is_empty() || func.is_empty() {
        anyhow::bail!("expected a non-empty FILE and FN in `{spec}`");
    }
    Ok((file, func))
}

/// Parse `file` and locate the free function named `name`.
fn load_function(file: &str, name: &str) -> Result<syn::ItemFn> {
    let source = std::fs::read_to_string(file).with_context(|| {
        format!(
            "Failed to read source file `{file}` for verify-equiv.\n\
             Please ensure the file exists and is readable."
        )
    })?;
    let parsed = syn::parse_file(&source).with_context(|| {
        format!(
            "Failed to parse Rust source in `{file}`.\n\
             Please ensure the file contains valid Rust code."
        )
    })?;

    for item in parsed.items {
        if let syn::Item::Fn(func) = item {
            if func.sig.ident == name {
                return Ok(func);
            }
        }
    }
    anyhow::bail!("function `{name}` not found in `{file}`")
}

/// Print a human-readable report for an outcome.
fn print_outcome(outcome: &VerifyOutcome) {
    match outcome {
        VerifyOutcome::Verified => {
            println!("✓ Proven equivalent for all inputs (QF_BV).");
        }
        VerifyOutcome::Refuted(cx) => {
            println!("✗ Not equivalent — counterexample found:");
            println!("  ┌─ inputs ─────────────────────────");
            for (name, value) in &cx.inputs {
                println!("  │ {name} = {value}");
            }
            println!("  ├─ outputs ────────────────────────");
            println!("  │ left  = {}", cx.left_output.as_deref().unwrap_or("?"));
            println!("  │ right = {}", cx.right_output.as_deref().unwrap_or("?"));
            println!("  └──────────────────────────────────");
        }
        VerifyOutcome::Unsupported { reason } => {
            println!("⚠ Cannot prove: {reason}");
        }
    }
}
