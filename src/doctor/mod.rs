//! The dependency doctor (TECH_SPEC §9, ARCHITECTURE §8).
//!
//! Checks which required dependencies are present and, for missing ones,
//! produces an OS-aware, ordered install plan via a pure, recursive,
//! cycle-guarded resolver.
//!
//! Boundary: `mod.rs` (resolver), `installers.rs` (closed installer vocab), and
//! `catalog.rs` (recipe data) are **pure** and unit-tested with synthetic
//! `Environment`s; only `probe.rs` touches the system. `doctor` depends on
//! `registry`/`contract`, never on `cli`.

pub mod catalog;
pub mod installers;
pub mod probe;

use std::collections::HashSet;

use serde::Serialize;

use catalog::{DepCatalog, DepRecipe};
use installers::Installer;
use probe::Environment;

/// The base dependency every project needs (universal task runner). Owned by no
/// tool; has a `registry/deps.toml` entry like any dep (TECH_SPEC §9.1).
pub const BASE_DEP: &str = "just";

/// Synthetic component id the project scan reports for the aggregated `infra/`
/// component (which has no per-tool subdirs to identify). It represents the
/// cardano-up driver; its required deps are the union of all infra tools'
/// `system_deps` (TECH_SPEC §9.6).
pub const INFRA_DRIVER_ID: &str = "cardano-up";

// ---------------------------------------------------------------------------
// Report types (serialize to the §9.4 JSON `data` payload)
// ---------------------------------------------------------------------------

/// One step of an install plan.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub installer: Installer,
    pub command: String,
}

/// An alternative install method for a missing dep — a recipe method other than
/// the one the recommended `plan` uses, so the user can choose. `available` is
/// whether its installer is present on this host right now (an unavailable one
/// needs its installer installed first).
#[derive(Debug, Clone, Serialize)]
pub struct AltMethod {
    pub installer: Installer,
    pub command: String,
    pub available: bool,
}

/// The resolved status of a single dependency.
#[derive(Debug, Clone, Serialize)]
pub struct DepStatus {
    pub id: String,
    pub required: bool,
    pub present: bool,
    /// Ordered install steps for this host. Empty when present or unresolved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<Step>,
    /// Other recipe methods beyond `plan` (available ones first, then those
    /// needing an installer this host lacks). Lets the user pick a different
    /// installer. Empty when present, or when the recipe offers no other method.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<AltMethod>,
    /// Docs URL — the universal fallback so advice is never empty (FR-20).
    /// Omitted only when the dep is already present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// The full doctor report for a set of required dependencies.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    pub all_required_present: bool,
    pub deps: Vec<DepStatus>,
}

impl Report {
    /// The missing required deps, in report order.
    pub fn missing_required(&self) -> impl Iterator<Item = &DepStatus> {
        self.deps.iter().filter(|d| d.required && !d.present)
    }
}

// ---------------------------------------------------------------------------
// Resolver (pure, recursive)
// ---------------------------------------------------------------------------

enum Resolved {
    Plan(Vec<Step>),
    Unresolved,
}

fn dep_present(recipe: &DepRecipe, env: &Environment) -> bool {
    recipe
        .binaries
        .iter()
        .chain(
            recipe
                .binaries_by_os
                .get(env.os.as_key())
                .into_iter()
                .flatten(),
        )
        .any(|b| env.present_binaries.contains(b))
}

/// Resolve a single dep to an ordered install plan for this host, or
/// `Unresolved`. Recursive over installer `bootstrap` edges; `seen` guards
/// against cycles (TECH_SPEC §9.4).
fn resolve(
    dep_id: &str,
    env: &Environment,
    catalog: &DepCatalog,
    seen: &HashSet<String>,
) -> Resolved {
    if seen.contains(dep_id) {
        return Resolved::Unresolved; // cycle guard
    }
    let Some(recipe) = catalog.get(dep_id) else {
        return Resolved::Unresolved;
    };
    if dep_present(recipe, env) {
        return Resolved::Plan(Vec::new()); // already present
    }

    // Pass 1 (preferred): the first method whose installer is usable right now
    // yields a one-step plan. This is why `aiken` resolves directly via `nix`
    // when present, without bootstrapping `aikup` (TECH_SPEC §9.4).
    for method in &recipe.install {
        if env.installers.contains(&method.installer) {
            return Resolved::Plan(vec![Step {
                installer: method.installer,
                command: method.installer.command(&method.arg),
            }]);
        }
    }

    // Pass 2: no installer is directly available — bootstrap one, in recipe
    // order (empty `bootstrap` ⇒ terminal, skipped). Prepend the bootstrap
    // steps, then this method's step.
    for method in &recipe.install {
        for bdep in method.installer.bootstrap() {
            let mut next = seen.clone();
            next.insert(dep_id.to_string());
            if let Resolved::Plan(mut steps) = resolve(bdep, env, catalog, &next) {
                steps.push(Step {
                    installer: method.installer,
                    command: method.installer.command(&method.arg),
                });
                return Resolved::Plan(steps);
            }
        }
    }
    Resolved::Unresolved
}

/// The recipe methods to offer as alternatives to the recommended `plan`: every
/// method whose installer differs from the one `plan` settles on, ordered
/// available-first (each keeping the recipe's preference order within its
/// group). When `plan` is empty (unresolved), every method is offered — all
/// flagged unavailable, since a present installer would have produced a plan.
fn alt_methods(recipe: &DepRecipe, chosen: Option<Installer>, env: &Environment) -> Vec<AltMethod> {
    let (mut available, mut needs_installer) = (Vec::new(), Vec::new());
    for method in &recipe.install {
        if Some(method.installer) == chosen {
            continue;
        }
        let alt = AltMethod {
            installer: method.installer,
            command: method.installer.command(&method.arg),
            available: env.installers.contains(&method.installer),
        };
        if alt.available {
            available.push(alt);
        } else {
            needs_installer.push(alt);
        }
    }
    available.into_iter().chain(needs_installer).collect()
}

/// Resolve all required dependencies into a `Report`. Dependencies are
/// deduplicated and reported in a stable (sorted) order.
pub fn resolve_all(required: &[String], catalog: &DepCatalog, env: &Environment) -> Report {
    let mut ids: Vec<String> = required.to_vec();
    ids.sort();
    ids.dedup();

    let mut deps = Vec::with_capacity(ids.len());
    let mut all_required_present = true;

    for id in ids {
        let recipe = catalog.get(&id);
        let present = recipe.map(|r| dep_present(r, env)).unwrap_or(false);
        if !present {
            all_required_present = false;
        }

        let (plan, alternatives, docs) = if present {
            (Vec::new(), Vec::new(), None)
        } else {
            let docs = recipe.map(|r| r.docs.clone());
            let plan = match resolve(&id, env, catalog, &HashSet::new()) {
                Resolved::Plan(steps) => steps,
                Resolved::Unresolved => Vec::new(),
            };
            // Everything the recipe offers beyond the recommended plan.
            let chosen = plan.last().map(|s| s.installer);
            let alternatives = recipe.map_or_else(Vec::new, |r| alt_methods(r, chosen, env));
            (plan, alternatives, docs)
        };

        deps.push(DepStatus {
            id,
            required: true,
            present,
            plan,
            alternatives,
            docs,
        });
    }

    Report {
        all_required_present,
        deps,
    }
}

// ---------------------------------------------------------------------------
// Tests (synthetic Environment — no system access)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::probe::Os;

    fn env_for(os: Os, installers: &[Installer], present_bins: &[&str]) -> Environment {
        Environment {
            os,
            installers: installers.iter().copied().collect(),
            present_binaries: present_bins.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn env(installers: &[Installer], present_bins: &[&str]) -> Environment {
        env_for(Os::Linux, installers, present_bins)
    }

    fn catalog() -> DepCatalog {
        DepCatalog::load().expect("embedded catalog loads")
    }

    fn status<'a>(report: &'a Report, id: &str) -> &'a DepStatus {
        report
            .deps
            .iter()
            .find(|d| d.id == id)
            .expect("dep present")
    }

    #[test]
    fn present_dep_yields_empty_plan() {
        let report = resolve_all(&["just".to_string()], &catalog(), &env(&[], &["just"]));
        let just = status(&report, "just");
        assert!(just.present);
        assert!(just.plan.is_empty());
        assert!(just.docs.is_none());
        assert!(report.all_required_present);
    }

    #[test]
    fn env_lock_uses_only_the_current_platform_binaries() {
        for (os, present) in [
            (Os::Linux, "flock"),
            (Os::MacOs, "lockf"),
            (Os::Windows, "powershell.exe"),
            (Os::Windows, "pwsh.exe"),
        ] {
            let report = resolve_all(
                &["env-lock".to_string()],
                &catalog(),
                &env_for(os, &[], &[present]),
            );
            assert!(status(&report, "env-lock").present, "{os:?}: {present}");
        }

        for (os, wrong) in [
            (Os::Linux, "lockf"),
            (Os::MacOs, "flock"),
            (Os::Windows, "flock"),
            (Os::Windows, "powershell"),
            (Os::Other, "flock"),
        ] {
            let report = resolve_all(
                &["env-lock".to_string()],
                &catalog(),
                &env_for(os, &[], &[wrong]),
            );
            let lock = status(&report, "env-lock");
            assert!(!lock.present, "{os:?} must not accept {wrong}");
            assert!(lock.plan.is_empty());
            assert!(lock.docs.is_some());
        }
    }

    #[test]
    fn single_step_when_installer_detected() {
        // node missing, brew available → one-step `brew install node`.
        let report = resolve_all(
            &["node".to_string()],
            &catalog(),
            &env(&[Installer::Brew], &[]),
        );
        let node = status(&report, "node");
        assert!(!node.present);
        assert_eq!(node.plan.len(), 1);
        assert_eq!(node.plan[0].installer, Installer::Brew);
        assert_eq!(node.plan[0].command, "brew install node");
        assert!(node.docs.is_some());
        assert!(!report.all_required_present);
    }

    #[test]
    fn multi_step_bootstrap_aiken_via_npm() {
        // aiken missing, no nix/aikup, but npm present → bootstrap aikup via npm,
        // then `aikup install` (bare ⇒ latest).
        let report = resolve_all(
            &["aiken".to_string()],
            &catalog(),
            &env(&[Installer::Npm], &[]),
        );
        let aiken = status(&report, "aiken");
        assert!(!aiken.present);
        assert_eq!(aiken.plan.len(), 2);
        assert_eq!(aiken.plan[0].installer, Installer::Npm);
        assert_eq!(aiken.plan[0].command, "npm install -g @aiken-lang/aikup");
        assert_eq!(aiken.plan[1].installer, Installer::Aikup);
        assert_eq!(aiken.plan[1].command, "aikup install");
    }

    #[test]
    fn nix_short_circuits_aiken_to_one_step() {
        // With nix present, aiken resolves directly (no aikup needed).
        let report = resolve_all(
            &["aiken".to_string()],
            &catalog(),
            &env(&[Installer::Nix], &[]),
        );
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan.len(), 1);
        assert_eq!(aiken.plan[0].installer, Installer::Nix);
        assert_eq!(aiken.plan[0].command, "nix profile install nixpkgs#aiken");
    }

    #[test]
    fn unresolved_falls_back_to_docs_only() {
        // aiken missing and no usable installer at all → no plan, docs present.
        let report = resolve_all(&["aiken".to_string()], &catalog(), &env(&[], &[]));
        let aiken = status(&report, "aiken");
        assert!(!aiken.present);
        assert!(aiken.plan.is_empty());
        assert_eq!(
            aiken.docs.as_deref(),
            Some("https://aiken-lang.org/installation-instructions")
        );
    }

    #[test]
    fn alternatives_offer_other_methods_available_first() {
        // sbt recipe = [brew, nix]. With only nix present: plan uses nix, and
        // brew is offered as an alternative flagged unavailable.
        let report = resolve_all(
            &["sbt".to_string()],
            &catalog(),
            &env(&[Installer::Nix], &[]),
        );
        let sbt = status(&report, "sbt");
        assert_eq!(sbt.plan[0].installer, Installer::Nix);
        assert_eq!(sbt.alternatives.len(), 1);
        assert_eq!(sbt.alternatives[0].installer, Installer::Brew);
        assert!(!sbt.alternatives[0].available);
        assert_eq!(sbt.alternatives[0].command, "brew install sbt");
    }

    #[test]
    fn alternatives_sort_available_before_needs_installer() {
        // just recipe = [brew, apt, cargo, nix]. With cargo + nix present: plan
        // uses cargo (first available); the other available method (nix) is
        // offered before the ones that still need their installer (brew, apt).
        let report = resolve_all(
            &["just".to_string()],
            &catalog(),
            &env(&[Installer::Cargo, Installer::Nix], &[]),
        );
        let just = status(&report, "just");
        assert_eq!(just.plan[0].installer, Installer::Cargo);
        // The chosen installer is never repeated as an alternative.
        assert!(
            just.alternatives
                .iter()
                .all(|a| a.installer != Installer::Cargo)
        );
        // First alternative is the other available installer.
        assert_eq!(just.alternatives[0].installer, Installer::Nix);
        assert!(just.alternatives[0].available);
        // Unavailable ones come after and are flagged.
        assert!(
            just.alternatives
                .iter()
                .any(|a| a.installer == Installer::Brew && !a.available)
        );
    }

    #[test]
    fn dedup_and_sorted_order() {
        let report = resolve_all(
            &["node".to_string(), "just".to_string(), "node".to_string()],
            &catalog(),
            &env(&[], &["node", "just"]),
        );
        let ids: Vec<&str> = report.deps.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["just", "node"]); // deduped, sorted
    }

    #[test]
    fn whole_catalog_resolves_without_infinite_recursion() {
        // Cycle guard: resolving every dep id terminates (worst case: empty env).
        let cat = catalog();
        let env = env(&[], &[]);
        for id in cat.dep_ids() {
            let _ = resolve(id, &env, &cat, &HashSet::new()); // must return, not hang
        }
    }

    #[test]
    fn report_json_shape_matches_spec() {
        // present dep → only {id, required, present}; missing+resolved →
        // {id, required, present, plan, docs} (TECH_SPEC §9.4).
        let report = resolve_all(
            &["just".to_string(), "node".to_string()],
            &catalog(),
            &env(&[Installer::Brew], &["just"]),
        );
        let value = serde_json::to_value(&report).unwrap();
        let deps = value["deps"].as_array().unwrap();

        let just = deps.iter().find(|d| d["id"] == "just").unwrap();
        assert_eq!(just["present"], true);
        assert!(just.get("plan").is_none(), "present dep must omit plan");
        assert!(just.get("docs").is_none(), "present dep must omit docs");

        let node = deps.iter().find(|d| d["id"] == "node").unwrap();
        assert_eq!(node["present"], false);
        assert_eq!(node["plan"][0]["installer"], "brew");
        assert_eq!(node["plan"][0]["command"], "brew install node");
        assert!(node["docs"].is_string());
    }

    // ---- Referential integrity (TECH_SPEC §9.5) ----

    #[test]
    fn every_system_dep_plus_base_has_a_recipe() {
        let registry = crate::registry::loader::Registry::load().unwrap();
        let cat = catalog();
        assert!(
            cat.get(BASE_DEP).is_some(),
            "base dep '{BASE_DEP}' must have a recipe"
        );
        for tool in registry.all_tools() {
            for dep in &tool.system_deps {
                assert!(
                    cat.get(dep).is_some(),
                    "tool '{}' requires dep '{}' with no registry/deps.toml entry",
                    tool.id,
                    dep
                );
            }
        }
    }

    #[test]
    fn every_bootstrap_dep_has_a_recipe() {
        let cat = catalog();
        for installer in Installer::ALL {
            for bdep in installer.bootstrap() {
                assert!(
                    cat.get(bdep).is_some(),
                    "installer '{}' bootstraps via dep '{}' with no recipe",
                    installer,
                    bdep
                );
            }
        }
    }
}
