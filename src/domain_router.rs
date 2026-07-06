//! Domain-mapping support for Feature B (`--target-modules`).
//!
//! This module extends the pattern-based routing in `file_analyzer` with:
//!
//! - **Seeded assignment** (`assign_unlisted = "seeded"` / per-rule
//!   `pull_dependencies = true`): items not matched by any rule are pulled
//!   into the named module they have the strongest reference affinity with,
//!   iterated to a fixpoint, with the pattern-routed items acting as seeds.
//!   Zero-affinity items fall through to the classic heuristic buckets, so
//!   the behaviour is opt-in and monotonic.
//! - **Unknown-name validation**: exact (non-glob) patterns that name no item
//!   in the file are a hard error, with near-miss suggestions.
//! - **Dry-run attribution**: for each item in a named module, report which
//!   rule (or seed edge) pulled it there.

// Shared internal API between the lib and bin compilation units; some items
// are only called from one of the two targets (mirrors the established
// pattern in `file_analyzer` / `source_map`).
#![allow(dead_code)]

use crate::config::{self, TargetModule};
use crate::file_analyzer::{standalone_routing_name, FileAnalyzer, TargetRouting};
use crate::module_generator::{Module, RefVisitor};
use anyhow::Result;
use std::collections::HashSet;
use syn::visit::Visit;

/// One unrouted item that seeded assignment may pull into a named module.
struct Candidate {
    /// The routable name of the item (type name, fn name, const name, ...).
    name: String,
    /// Identity of the item in the analyzer's pools.
    key: CandidateKey,
    /// Path-root references of the item (AST-accurate, via `RefVisitor`).
    refs: HashSet<String>,
}

/// Identity of a candidate in the analyzer's pools.
enum CandidateKey {
    /// A type (struct/enum) keyed by name in `FileAnalyzer::types`; moving it
    /// moves its bundled inherent and trait impls too.
    Type(String),
    /// A standalone item, by index into `FileAnalyzer::standalone_items`.
    Standalone(usize),
}

/// Seeded assignment of unlisted items (F2).
///
/// Wave 0 seeds are the pattern-routed items already present in `routing`.
/// Each wave scans the remaining unrouted items and computes, per attractor
/// module, an affinity score:
///
/// - +1 for every name *defined* in the module that the candidate references
///   (candidate → module edges: calls, type usage, field types, ...);
/// - +1 when the module's items reference the candidate's own name
///   (module → candidate edges: e.g. a routed function calling a private
///   helper).
///
/// The candidate is assigned to the highest-affinity module (ties break by
/// rule declaration order); newly assigned items become seeds for the next
/// wave; iteration stops at a fixpoint. Deterministic: candidates are visited
/// in sorted order, modules in declaration order, and all decisions within a
/// wave are based on the same pre-wave state.
pub(crate) fn seeded_assign(analyzer: &FileAnalyzer, routing: &mut TargetRouting) {
    let rules = analyzer.target_rules();
    if rules.is_empty() || routing.modules.is_empty() {
        return;
    }
    let global = analyzer.seeded_assignment_enabled();
    let attractors: Vec<usize> = rules
        .iter()
        .enumerate()
        .filter(|(_, rule)| global || rule.pull_dependencies)
        .map(|(idx, _)| idx)
        .collect();
    if attractors.is_empty() {
        return;
    }

    // Per-module state: names defined in the module and path roots referenced
    // by its items. Grows as candidates are assigned.
    let mut defined: Vec<HashSet<String>> = Vec::new();
    let mut referenced: Vec<HashSet<String>> = Vec::new();
    for module in &routing.modules {
        defined.push(module.get_exported_types().into_iter().collect());
        referenced.push(module_path_roots(module));
    }

    // Build the deterministic candidate list: unrouted types (sorted by
    // name), then unrouted named standalone items (in file order).
    let mut candidates: Vec<Candidate> = Vec::new();
    let mut type_names: Vec<&String> = analyzer.types.keys().collect();
    type_names.sort();
    for name in type_names {
        if routing.routed_type_names.contains(name.as_str()) {
            continue;
        }
        let Some(type_info) = analyzer.types.get(name) else {
            continue;
        };
        let mut visitor = RefVisitor::default();
        visitor.visit_item(&type_info.item);
        for impl_item in &type_info.impls {
            visitor.visit_item(impl_item);
        }
        for trait_impl in &type_info.trait_impls {
            visitor.visit_item(&trait_impl.impl_item);
        }
        candidates.push(Candidate {
            name: name.clone(),
            key: CandidateKey::Type(name.clone()),
            refs: visitor.path_roots,
        });
    }
    for (idx, item) in analyzer.standalone_items.iter().enumerate() {
        if routing.routed_standalone_indices.contains(&idx) {
            continue;
        }
        let Some(name) = standalone_routing_name(item) else {
            continue;
        };
        let mut visitor = RefVisitor::default();
        visitor.visit_item(item);
        candidates.push(Candidate {
            name,
            key: CandidateKey::Standalone(idx),
            refs: visitor.path_roots,
        });
    }

    // Fixpoint iteration. Every wave assigns at least one candidate or stops,
    // and the candidate pool is finite, so this terminates.
    loop {
        // Decisions for this wave, based on the same pre-wave state.
        let mut wave: Vec<(usize, usize)> = Vec::new(); // (candidate pos, module idx)
        for (pos, candidate) in candidates.iter().enumerate() {
            let mut best: Option<(usize, usize)> = None; // (affinity, module idx)
            for &module_idx in &attractors {
                let mut affinity = candidate.refs.intersection(&defined[module_idx]).count();
                if referenced[module_idx].contains(&candidate.name) {
                    affinity += 1;
                }
                if affinity == 0 {
                    continue;
                }
                let better = match best {
                    None => true,
                    // Strict `>` keeps the earlier-declared rule on ties.
                    Some((best_affinity, _)) => affinity > best_affinity,
                };
                if better {
                    best = Some((affinity, module_idx));
                }
            }
            if let Some((_, module_idx)) = best {
                wave.push((pos, module_idx));
            }
        }
        if wave.is_empty() {
            break;
        }

        // Apply the wave (descending positions so removals stay valid).
        for &(pos, module_idx) in wave.iter().rev() {
            let candidate = candidates.remove(pos);
            defined[module_idx].insert(candidate.name.clone());
            referenced[module_idx].extend(candidate.refs.iter().cloned());
            match candidate.key {
                CandidateKey::Type(name) => {
                    if let Some(type_info) = analyzer.types.get(&name) {
                        routing.modules[module_idx].types.push(type_info.clone());
                    }
                    routing.routed_type_names.insert(name);
                }
                CandidateKey::Standalone(idx) => {
                    if let Some(item) = analyzer.standalone_items.get(idx) {
                        routing.modules[module_idx]
                            .standalone_items
                            .push(item.clone());
                        routing.modules[module_idx]
                            .standalone_verbatim
                            .push(analyzer.standalone_verbatim_for(item));
                    }
                    routing.routed_standalone_indices.insert(idx);
                }
            }
        }
    }
}

/// Path roots referenced by every item currently held by `module`.
fn module_path_roots(module: &Module) -> HashSet<String> {
    let mut visitor = RefVisitor::default();
    for type_info in &module.types {
        visitor.visit_item(&type_info.item);
        for impl_item in &type_info.impls {
            visitor.visit_item(impl_item);
        }
        for trait_impl in &type_info.trait_impls {
            visitor.visit_item(&trait_impl.impl_item);
        }
    }
    for item in &module.standalone_items {
        visitor.visit_item(item);
    }
    visitor.path_roots
}

/// The set of names the routing rules can match in this analyzer: type names
/// plus every named standalone item (functions, consts, statics, aliases,
/// traits, macros, impl-target types).
pub fn routable_names(analyzer: &FileAnalyzer) -> HashSet<String> {
    let mut names: HashSet<String> = analyzer.types.keys().cloned().collect();
    for item in &analyzer.standalone_items {
        if let Some(name) = standalone_routing_name(item) {
            names.insert(name);
        }
    }
    names
}

/// Hard-error validation for exact (non-glob) patterns that name no known
/// item, with near-miss suggestions (edit distance <= 3, closest first).
///
/// Glob patterns that match nothing are *not* an error — they may
/// legitimately be speculative — their module is simply not emitted.
pub fn check_unmatched_patterns(available: &HashSet<String>, rules: &[TargetModule]) -> Result<()> {
    let mut problems: Vec<String> = Vec::new();
    for rule in rules {
        for pattern in &rule.items {
            if pattern.contains('*') || available.contains(pattern) {
                continue;
            }
            let mut near: Vec<(usize, &String)> = available
                .iter()
                .map(|name| (levenshtein(pattern, name), name))
                .filter(|(distance, _)| *distance <= 3)
                .collect();
            near.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(b.1)));
            let hint = if near.is_empty() {
                String::new()
            } else {
                let suggestions: Vec<&str> =
                    near.iter().take(3).map(|(_, name)| name.as_str()).collect();
                format!(" — did you mean {}?", suggestions.join(", "))
            };
            problems.push(format!(
                "rule `{}`: item `{}` does not name any known item{}",
                rule.name, pattern, hint
            ));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        anyhow::bail!(
            "target-modules spec references unknown items:\n  {}\n\
             (items inside a nested inline module need `parent = \"<mod path>\"` \
             on the rule and --split-nested-mods)",
            problems.join("\n  ")
        )
    }
}

/// Dry-run attribution: when `module` corresponds to one of the named rules
/// (or one of its `<name>_N` budget-overflow chunks), return one line per
/// contained item saying which rule pattern routed it or that it was pulled
/// in by seeding. Returns `None` for heuristic (non-rule) modules.
pub fn explain_named_module(module: &Module, rules: &[TargetModule]) -> Option<Vec<String>> {
    let is_rule_module = rules.iter().any(|rule| {
        module.name == rule.name
            || module
                .name
                .strip_prefix(&format!("{}_", rule.name))
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                })
    });
    if !is_rule_module {
        return None;
    }

    let mut lines = Vec::new();
    let mut explain = |name: &str| match config::route_item_detailed(name, rules) {
        Some((idx, pattern)) => lines.push(format!("{} (rule {}: {})", name, idx + 1, pattern)),
        None => lines.push(format!("{} (seeded)", name)),
    };
    for type_info in &module.types {
        explain(&type_info.name);
    }
    for item in &module.standalone_items {
        if let Some(name) = standalone_routing_name(item) {
            explain(&name);
        }
    }
    Some(lines)
}

/// Classic Levenshtein edit distance (small inputs: item names).
fn levenshtein(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b_chars.len()).collect();
    let mut current: Vec<usize> = vec![0; b_chars.len() + 1];
    for (i, &ca) in a_chars.iter().enumerate() {
        current[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let substitution = previous[j] + usize::from(ca != cb);
            current[j + 1] = substitution.min(previous[j + 1] + 1).min(current[j] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[b_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levenshtein_basics() {
        assert_eq!(levenshtein("", ""), 0);
        assert_eq!(levenshtein("abc", "abc"), 0);
        assert_eq!(levenshtein("abc", "abd"), 1);
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn unmatched_exact_pattern_is_hard_error_with_suggestion() {
        let mut available = HashSet::new();
        available.insert("compute_md5".to_string());
        available.insert("FsEntry".to_string());
        let rules = vec![TargetModule {
            name: "hash".to_string(),
            items: vec!["compute_md6".to_string()],
            ..Default::default()
        }];
        let err = check_unmatched_patterns(&available, &rules)
            .expect_err("unknown exact name must be a hard error")
            .to_string();
        assert!(err.contains("compute_md6"), "missing offender: {err}");
        assert!(err.contains("compute_md5"), "missing suggestion: {err}");
    }

    #[test]
    fn unmatched_glob_pattern_is_not_an_error() {
        let mut available = HashSet::new();
        available.insert("FsEntry".to_string());
        let rules = vec![TargetModule {
            name: "hash".to_string(),
            items: vec!["*hash*".to_string()],
            ..Default::default()
        }];
        assert!(check_unmatched_patterns(&available, &rules).is_ok());
    }

    #[test]
    fn matched_exact_pattern_passes() {
        let mut available = HashSet::new();
        available.insert("FsEntry".to_string());
        let rules = vec![TargetModule {
            name: "fs".to_string(),
            items: vec!["FsEntry".to_string()],
            ..Default::default()
        }];
        assert!(check_unmatched_patterns(&available, &rules).is_ok());
    }
}
