pub mod context;
pub mod planner;
pub mod renderer;
pub mod writer;

use std::path::Path;

use rust_embed::RustEmbed;

use crate::registry::loader::Registry;
use crate::registry::types::Selection;

// ---------------------------------------------------------------------------
// Embedded template assets
// ---------------------------------------------------------------------------

#[derive(RustEmbed)]
#[folder = "templates/"]
pub(crate) struct TemplateAssets;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("template asset not found: '{path}'")]
    AssetNotFound { path: String },

    #[error("failed to parse manifest '{path}': {source}")]
    ManifestParse {
        path: String,
        source: toml::de::Error,
    },

    #[error("tool '{tool_id}' not found in registry")]
    ToolNotFound { tool_id: String },

    #[error("tool '{tool_id}' does not support role '{role}'")]
    RoleMismatch { tool_id: String, role: String },

    #[error("template rendering failed for '{path}': {source}")]
    Render {
        path: String,
        source: minijinja::Error,
    },

    #[error("I/O error writing '{path}': {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Run the full scaffolding pipeline: context -> plan -> render -> write.
pub fn scaffold(
    selection: &Selection,
    registry: &Registry,
    root: &Path,
) -> Result<(), ScaffoldError> {
    let ctx = context::build_context(selection, registry)?;
    let plan = planner::plan(selection, registry)?;
    let files = renderer::render(&plan, &ctx)?;
    writer::write(&files, root)?;
    Ok(())
}

/// Plan-only mode: returns the file plan without writing anything.
pub fn dry_run(
    selection: &Selection,
    registry: &Registry,
) -> Result<planner::FilePlan, ScaffoldError> {
    planner::plan(selection, registry)
}

// ---------------------------------------------------------------------------
// Integration tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, Role, RoleAssignment};

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "my-protocol".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
    }

    #[test]
    fn scaffold_aiken_only() {
        let dir = tempfile::tempdir().unwrap();
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
        ]);

        scaffold(&sel, &registry(), dir.path()).unwrap();

        // Base files
        assert!(dir.path().join("Justfile").is_file());
        assert!(dir.path().join("README.md").is_file());
        assert!(dir.path().join(".gitignore").is_file());
        assert!(dir.path().join(".env").is_file());

        // Blueprint dir
        assert!(dir.path().join("blueprint/.gitkeep").is_file());

        // On-chain files
        assert!(dir.path().join("on-chain/aiken.toml").is_file());
        assert!(dir.path().join("on-chain/Justfile").is_file());
        assert!(dir.path().join("on-chain/validators/example.ak").is_file());

        // Content checks
        let justfile = std::fs::read_to_string(dir.path().join("Justfile")).unwrap();
        assert!(justfile.contains("my-protocol"));
        assert!(justfile.contains("build-on-chain"));
        assert!(!justfile.contains("build-off-chain"));

        let readme = std::fs::read_to_string(dir.path().join("README.md")).unwrap();
        assert!(readme.contains("my-protocol"));
        assert!(readme.contains("Aiken"));
    }

    #[test]
    fn scaffold_aiken_and_meshjs() {
        let dir = tempfile::tempdir().unwrap();
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
            RoleAssignment { role: Role::OffChain, tool_id: "meshjs".into() },
        ]);

        scaffold(&sel, &registry(), dir.path()).unwrap();

        // Both role directories present
        assert!(dir.path().join("on-chain/aiken.toml").is_file());
        assert!(dir.path().join("off-chain/package.json").is_file());
        assert!(dir.path().join("off-chain/src/index.ts").is_file());

        // Justfile references both roles
        let justfile = std::fs::read_to_string(dir.path().join("Justfile")).unwrap();
        assert!(justfile.contains("build-on-chain"));
        assert!(justfile.contains("build-off-chain"));

        // Package.json has project name
        let pkg = std::fs::read_to_string(dir.path().join("off-chain/package.json")).unwrap();
        assert!(pkg.contains("my-protocol"));
    }

    #[test]
    fn dry_run_returns_plan_without_writing() {
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
        ]);

        let plan = dry_run(&sel, &registry()).unwrap();

        assert!(!plan.entries.is_empty());
        let dests: Vec<&str> = plan.entries.iter().map(|e| e.dest.to_str().unwrap()).collect();
        assert!(dests.contains(&"Justfile"));
        assert!(dests.contains(&"on-chain/aiken.toml"));
    }
}
