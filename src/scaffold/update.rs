//! In-place project updates: mutate a reconstructed `Selection` and compute the
//! change set that turns the old project into the new one.
//!
//! This module is **pure** (no I/O): it diffs two selections at the
//! *component-directory* level and re-renders the shared top-level layer, all
//! from embedded assets. The disk side effects live in
//! [`super::writer::apply_update`]. See `docs/TECH_SPEC.md` §16.
//!
//! The load-bearing property is that each component is **standalone** (interface
//! contract): a slot whose tool is unchanged is not in the change set at all, so
//! the user's code in it is provably untouched.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, RoleAssignment, Selection};

use super::renderer::RenderedFile;
use super::{ScaffoldError, context, planner, renderer};

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// A single edit to a project's `Selection`.
#[derive(Debug, Clone)]
pub enum Mutation {
    /// Assign a tool to a role. For a single-tool role this **replaces** whatever
    /// currently fills it; for infrastructure it **appends** the provider
    /// (deduped keep-first). `--fullstack X` is expressed as two `Add`s (on-chain
    /// + off-chain, same tool), which the slot logic re-collapses to `protocol/`.
    Add(RoleAssignment),
    /// Remove a role entirely (all infrastructure providers, if Infrastructure).
    Remove(Role),
    /// Remove a single infrastructure provider by tool id.
    RemoveInfra(String),
}

/// Apply one mutation to `old`, returning the new selection (pure; the input is
/// left untouched).
pub fn apply(old: &Selection, mutation: &Mutation) -> Selection {
    let mut assignments = old.assignments.clone();
    match mutation {
        Mutation::Add(new_asg) if new_asg.role == Role::Infrastructure => {
            let already = assignments
                .iter()
                .any(|a| a.role == Role::Infrastructure && a.tool_id == new_asg.tool_id);
            if !already {
                assignments.push(new_asg.clone());
            }
        }
        Mutation::Add(new_asg) => {
            // Single-tool role: replace whatever is there.
            assignments.retain(|a| a.role != new_asg.role);
            assignments.push(new_asg.clone());
        }
        Mutation::Remove(role) => assignments.retain(|a| a.role != *role),
        Mutation::RemoveInfra(tool_id) => {
            assignments.retain(|a| !(a.role == Role::Infrastructure && a.tool_id == *tool_id))
        }
    }
    Selection {
        project_name: old.project_name.clone(),
        assignments,
        network: old.network,
        nix: old.nix,
    }
}

/// Fold a sequence of mutations left-to-right (e.g. a one-shot `add` carrying
/// several role flags).
pub fn apply_all(old: &Selection, mutations: &[Mutation]) -> Selection {
    mutations.iter().fold(old.clone(), |acc, m| apply(&acc, m))
}

// ---------------------------------------------------------------------------
// Change set
// ---------------------------------------------------------------------------

/// What happens to one component directory between the old and new selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOp {
    /// A component that did not exist: create its directory and files.
    Create(PathBuf),
    /// A component that is going away: remove its directory.
    Remove(PathBuf),
    /// A component whose tool changed: remove the old directory, write the new.
    Replace(PathBuf),
    /// The aggregated `infra/` component whose provider set changed: overwrite
    /// its managed files in place (the directory stays).
    RerenderInfra(PathBuf),
}

impl SlotOp {
    /// The directory this op acts on.
    pub fn dir(&self) -> &Path {
        match self {
            SlotOp::Create(p)
            | SlotOp::Remove(p)
            | SlotOp::Replace(p)
            | SlotOp::RerenderInfra(p) => p,
        }
    }
}

/// The full plan for turning the old project into the new one.
#[derive(Debug)]
pub struct UpdatePlan {
    /// Per-component-directory operations.
    pub slot_ops: Vec<SlotOp>,
    /// Rendered files for directories being created/replaced/re-rendered.
    pub creates: Vec<RenderedFile>,
    /// Re-rendered shared files, content-diffed at write time: the top-level
    /// layer plus the `.env` writer scripts, which are shared protocol rather
    /// than component code (see [`is_env_writer`]).
    pub shared_files: Vec<RenderedFile>,
    /// Shared files to delete (e.g. `blueprint/.gitkeep` when the predicate
    /// flips, or the nix files when `--nix` is turned off).
    pub shared_removals: Vec<PathBuf>,
}

/// One occupied component directory and the tool(s) in it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SlotTool {
    Single(String),
    Infra(BTreeSet<String>),
}

/// Map a selection to its occupied component directories, mirroring the planner's
/// dir logic (fullstack collapse → `protocol/`, infra aggregate → one `infra/`).
fn slots(sel: &Selection, registry: &Registry) -> BTreeMap<String, SlotTool> {
    let fullstack = planner::fullstack_tool_id(sel, registry);
    let mut map: BTreeMap<String, SlotTool> = BTreeMap::new();
    for asg in &sel.assignments {
        match asg.role {
            Role::Infrastructure => {
                let entry = map
                    .entry(contract::DIR_INFRA.to_string())
                    .or_insert_with(|| SlotTool::Infra(BTreeSet::new()));
                if let SlotTool::Infra(set) = entry {
                    set.insert(asg.tool_id.clone());
                }
            }
            Role::OnChain | Role::OffChain
                if fullstack.as_deref() == Some(asg.tool_id.as_str()) =>
            {
                map.insert(
                    contract::DIR_PROTOCOL.to_string(),
                    SlotTool::Single(asg.tool_id.clone()),
                );
            }
            Role::OnChain => {
                map.insert(
                    contract::DIR_ON_CHAIN.to_string(),
                    SlotTool::Single(asg.tool_id.clone()),
                );
            }
            Role::OffChain => {
                map.insert(
                    contract::DIR_OFF_CHAIN.to_string(),
                    SlotTool::Single(asg.tool_id.clone()),
                );
            }
            Role::Devnet => {
                map.insert(
                    contract::DIR_DEVNET.to_string(),
                    SlotTool::Single(asg.tool_id.clone()),
                );
            }
            Role::FormalMethods => {
                map.insert(
                    contract::DIR_FORMAL_METHODS.to_string(),
                    SlotTool::Single(asg.tool_id.clone()),
                );
            }
        }
    }
    map
}

/// The tool id(s) occupying each component directory in `sel`, keyed by dir and
/// sorted within each slot. Public so the CLI can name *which tool* changed in a
/// slot op, not merely which directory moved.
pub fn slot_tools(sel: &Selection, registry: &Registry) -> BTreeMap<String, Vec<String>> {
    slots(sel, registry)
        .into_iter()
        .map(|(dir, tool)| {
            let ids = match tool {
                SlotTool::Single(id) => vec![id],
                SlotTool::Infra(set) => set.into_iter().collect(),
            };
            (dir, ids)
        })
        .collect()
}

/// Compute the change set between two selections. Pure: renders only from
/// embedded assets, touches no disk.
pub fn plan_update(
    old: &Selection,
    new: &Selection,
    registry: &Registry,
) -> Result<UpdatePlan, ScaffoldError> {
    let old_slots = slots(old, registry);
    let new_slots = slots(new, registry);

    // Per-directory ops (canonical order via BTreeSet of keys).
    let dirs: BTreeSet<&String> = old_slots.keys().chain(new_slots.keys()).collect();
    let mut slot_ops = Vec::new();
    for dir in dirs {
        let path = PathBuf::from(dir);
        match (old_slots.get(dir), new_slots.get(dir)) {
            (None, Some(_)) => slot_ops.push(SlotOp::Create(path)),
            (Some(_), None) => slot_ops.push(SlotOp::Remove(path)),
            (Some(o), Some(n)) if o == n => {} // KEEP — untouched
            (Some(SlotTool::Infra(_)), Some(SlotTool::Infra(_))) => {
                slot_ops.push(SlotOp::RerenderInfra(path))
            }
            (Some(_), Some(_)) => slot_ops.push(SlotOp::Replace(path)),
            // `dir` comes from the union of both keysets, so this is unreachable.
            (None, None) => {}
        }
    }

    // Render the new project once; partition into shared layer vs component files.
    let plan = planner::plan(new, registry)?;
    let ctx = context::build_context(new, registry)?;
    let files = renderer::render(&plan, &ctx)?;

    let write_dirs: BTreeSet<PathBuf> = slot_ops
        .iter()
        .filter(|op| !matches!(op, SlotOp::Remove(_)))
        .map(|op| op.dir().to_path_buf())
        .collect();

    let mut shared_files = Vec::new();
    let mut creates = Vec::new();
    for file in files {
        if is_shared_layer(&file.dest) || is_env_writer(&file.dest) {
            shared_files.push(file);
        } else if let Some(top) = top_dir(&file.dest)
            && write_dirs.contains(&top)
        {
            creates.push(file);
            // A file under a KEEP directory falls through here — the user's copy
            // is left untouched.
        }
    }

    // Shared removals: the blueprint marker when the predicate flips off, and the
    // nix files when --nix is turned off (the latter can't happen via role
    // mutations today, but the diff stays honest).
    let mut shared_removals = Vec::new();
    if planner::blueprint_dir_present(old) && !planner::blueprint_dir_present(new) {
        shared_removals.push(PathBuf::from("blueprint/.gitkeep"));
    }
    if old.nix && !new.nix {
        shared_removals.push(PathBuf::from("flake.nix"));
        shared_removals.push(PathBuf::from(".envrc"));
    }

    Ok(UpdatePlan {
        slot_ops,
        creates,
        shared_files,
        shared_removals,
    })
}

/// Whether `dest` is a shared top-level file (re-rendered from the whole
/// selection) rather than a component file.
fn is_shared_layer(dest: &Path) -> bool {
    matches!(
        dest.to_str(),
        Some(
            "Justfile"
                | "README.md"
                | "AGENTS.md"
                | "CLAUDE.md"
                | ".gitignore"
                | ".env"
                | "flake.nix"
                | ".envrc"
                | "blueprint/.gitkeep"
        )
    )
}

/// Whether `dest` is one of the generated scripts that write the shared `.env`.
///
/// These live inside component directories but do not belong to any one
/// component: they implement a mutual-exclusion protocol over a single file at
/// the project root, so they are only correct while every writer in the project
/// speaks the same version of it. A component slot whose tool is unchanged is
/// KEPT, and a KEPT directory is normally not re-rendered, so without this
/// exception `cardano-init add --devnet yaci` against a project whose `infra/`
/// is unchanged would leave the old infra writer in place beside a new devnet
/// writer. The two would not exclude each other and a concurrent
/// `just dev` could still lose one writer's keys.
///
/// Treating them as shared layer keeps every writer in a project on one
/// protocol version. It is the one thing written into a KEPT directory, and it
/// is safe because these are generated scripts the user is not expected to edit
/// rather than user code; `apply_update` still content-diffs before writing.
fn is_env_writer(dest: &Path) -> bool {
    let Some(parent) = dest.parent() else {
        return false;
    };
    // Only `<component>/scripts/<name>`, never a root-level file of the same name.
    if parent.file_name().and_then(|n| n.to_str()) != Some("scripts") || top_dir(dest).is_none() {
        return false;
    }
    matches!(
        dest.file_name().and_then(|n| n.to_str()),
        Some("write-env.sh" | "set-env.mjs" | "set-env.sh" | "with-env-lock.ps1")
    )
}

/// The top-level directory of a nested path (e.g. `on-chain/aiken.toml` →
/// `on-chain`). Returns `None` for a root-level file.
fn top_dir(dest: &Path) -> Option<PathBuf> {
    let mut components = dest.components();
    let first = components.next()?;
    // Only a file *inside* a directory has a further component.
    components.next().map(|_| PathBuf::from(first.as_os_str()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::Network;

    fn registry() -> Registry {
        Registry::load().expect("registry loads")
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    fn sel(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "proj".into(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
    }

    fn dirs_touched(plan: &UpdatePlan) -> Vec<String> {
        plan.slot_ops
            .iter()
            .map(|op| op.dir().to_string_lossy().into_owned())
            .collect()
    }

    fn creates_under(plan: &UpdatePlan, dir: &str) -> bool {
        plan.creates.iter().any(|f| f.dest.starts_with(dir))
    }

    #[test]
    fn add_offchain_creates_only_that_slot() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "meshjs")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Create(PathBuf::from("off-chain"))]
        );
        assert!(creates_under(&plan, "off-chain"));
        // KEEP guarantee: on-chain is untouched — no op, no re-render.
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
        assert!(!creates_under(&plan, "on-chain"));
        // Shared layer always re-renders (root Justfile now wires off-chain).
        assert!(
            plan.shared_files
                .iter()
                .any(|f| f.dest.to_str() == Some("Justfile"))
        );
    }

    #[test]
    fn is_env_writer_matches_only_generated_env_writers() {
        for dest in [
            "infra/scripts/write-env.sh",
            "infra/scripts/with-env-lock.ps1",
            "devnet/scripts/set-env.mjs",
            "devnet/scripts/set-env.sh",
        ] {
            assert!(
                is_env_writer(Path::new(dest)),
                "{dest} should be an env writer"
            );
        }
        for dest in [
            // Root-level files of the same name are not component env writers.
            "write-env.sh",
            "set-env.mjs",
            // Not under a `scripts/` directory.
            "infra/write-env.sh",
            "devnet/lib/set-env.mjs",
            // Unrelated component files.
            "devnet/scripts/compose.sh",
            "infra/scripts/up.sh",
            "off-chain/src/index.ts",
        ] {
            assert!(
                !is_env_writer(Path::new(dest)),
                "{dest} should not be an env writer"
            );
        }
    }

    #[test]
    fn adding_devnet_refreshes_the_kept_infra_env_writer() {
        // The mixed-generation hazard: `add --devnet yaci` against a project
        // whose infra slot is unchanged KEEPs `infra/`, so without the env
        // writers in the shared layer the old `infra/scripts/write-env.sh`
        // would survive beside the new devnet writer and the two would not
        // exclude each other on `.env`.
        let reg = registry();
        let old = sel(vec![
            a(Role::OnChain, "aiken"),
            a(Role::Infrastructure, "cardano-node"),
        ]);
        let new = apply(&old, &Mutation::Add(a(Role::Devnet, "yaci")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        // infra is KEPT: no slot op, and no component-file writes into it.
        assert!(!dirs_touched(&plan).contains(&"infra".to_string()));
        assert!(!creates_under(&plan, "infra"));

        // Its env writer is refreshed anyway, via the shared layer.
        let shared: Vec<_> = plan
            .shared_files
            .iter()
            .filter_map(|f| f.dest.to_str())
            .collect();
        assert!(
            shared.contains(&"infra/scripts/write-env.sh"),
            "kept infra env writer must be refreshed, got {shared:?}"
        );
        assert!(
            shared.iter().any(|d| d.ends_with("/scripts/set-env.mjs")),
            "new devnet env writer must be present, got {shared:?}"
        );
    }

    #[test]
    fn remove_offchain_leaves_onchain_untouched() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let new = apply(&old, &Mutation::Remove(Role::OffChain));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Remove(PathBuf::from("off-chain"))]
        );
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
        assert!(!creates_under(&plan, "on-chain"));
    }

    #[test]
    fn swap_offchain_is_a_replace() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "tx3")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Replace(PathBuf::from("off-chain"))]
        );
        assert!(creates_under(&plan, "off-chain"));
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
    }

    #[test]
    fn fusion_removes_two_dirs_and_creates_protocol() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        // Switch both roles to a fullstack tool (scalus) → protocol/.
        let new = apply_all(
            &old,
            &[
                Mutation::Add(a(Role::OnChain, "scalus")),
                Mutation::Add(a(Role::OffChain, "scalus")),
            ],
        );
        let plan = plan_update(&old, &new, &reg).unwrap();

        let mut touched = dirs_touched(&plan);
        touched.sort();
        assert_eq!(touched, vec!["off-chain", "on-chain", "protocol"]);
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Remove(PathBuf::from("on-chain")))
        );
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Remove(PathBuf::from("off-chain")))
        );
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Create(PathBuf::from("protocol")))
        );
        assert!(creates_under(&plan, "protocol"));
    }

    #[test]
    fn adding_infra_provider_rerenders_infra_in_place() {
        let reg = registry();
        let old = sel(vec![
            a(Role::OnChain, "aiken"),
            a(Role::Infrastructure, "kupo"),
        ]);
        let new = apply(&old, &Mutation::Add(a(Role::Infrastructure, "ogmios")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::RerenderInfra(PathBuf::from("infra"))]
        );
        assert!(creates_under(&plan, "infra"));
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
    }

    #[test]
    fn removing_last_noninfra_role_drops_blueprint_marker() {
        let reg = registry();
        let old = sel(vec![
            a(Role::OnChain, "aiken"),
            a(Role::Infrastructure, "kupo"),
        ]);
        let new = apply(&old, &Mutation::Remove(Role::OnChain));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Remove(PathBuf::from("on-chain"))]
        );
        assert!(
            plan.shared_removals
                .contains(&PathBuf::from("blueprint/.gitkeep"))
        );
    }

    #[test]
    fn identical_selection_is_a_noop() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let plan = plan_update(&old, &old, &reg).unwrap();
        assert!(plan.slot_ops.is_empty());
        assert!(plan.shared_removals.is_empty());
    }

    #[test]
    fn add_infra_dedup_is_noop() {
        let old = sel(vec![a(Role::Infrastructure, "kupo")]);
        let new = apply(&old, &Mutation::Add(a(Role::Infrastructure, "kupo")));
        assert_eq!(new, old);
    }
}
