use std::path::PathBuf;

use serde::Deserialize;

use super::{ScaffoldError, TemplateAssets};
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Where a file's content comes from in the embedded templates.
#[derive(Debug, Clone)]
pub enum TemplateSource {
    /// From `templates/_base/<path>`
    Base(String),
    /// From `templates/<tool>/<role>/<path>`
    Role(String),
    /// From `templates/_nix/<path>`
    Optional(String),
    /// Inline content (e.g., empty `.gitkeep` files)
    Inline(Vec<u8>),
}

impl TemplateSource {
    /// The asset key used to look up this source in `TemplateAssets`.
    /// Returns `None` for `Inline` sources.
    pub fn asset_key(&self) -> Option<String> {
        match self {
            TemplateSource::Base(path) => Some(format!("_base/{path}")),
            TemplateSource::Role(path) => Some(path.clone()),
            TemplateSource::Optional(path) => Some(path.clone()),
            TemplateSource::Inline(_) => None,
        }
    }
}

/// One file to emit in the generated project.
#[derive(Debug, Clone)]
pub struct FileEntry {
    /// Destination path relative to the project root.
    pub dest: PathBuf,
    /// Where the content comes from.
    pub source: TemplateSource,
    /// Whether to render through MiniJinja.
    pub render: bool,
}

/// The complete list of files to generate.
#[derive(Debug)]
pub struct FilePlan {
    pub entries: Vec<FileEntry>,
}

// ---------------------------------------------------------------------------
// Manifest TOML (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ManifestToml {
    #[allow(dead_code)]
    manifest: ManifestMeta,
    #[serde(rename = "files")]
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestMeta {
    #[allow(dead_code)]
    summary: String,
}

#[derive(Deserialize)]
struct ManifestFile {
    source: String,
    dest: String,
    render: bool,
}

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Build a `FilePlan` from a `Selection` and the tool `Registry`.
///
/// This determines every file that will be written during scaffolding.
/// No I/O is performed — only embedded assets are read.
pub fn plan(selection: &Selection, registry: &Registry) -> Result<FilePlan, ScaffoldError> {
    let mut entries = vec![
        // --- Base layer ---
        FileEntry {
            dest: PathBuf::from("Justfile"),
            source: TemplateSource::Base("Justfile.jinja".into()),
            render: true,
        },
        FileEntry {
            dest: PathBuf::from("README.md"),
            source: TemplateSource::Base("README.md.jinja".into()),
            render: true,
        },
        FileEntry {
            dest: PathBuf::from(".gitignore"),
            source: TemplateSource::Base("gitignore".into()),
            render: false,
        },
        FileEntry {
            dest: PathBuf::from(".env"),
            source: TemplateSource::Base("env.jinja".into()),
            render: true,
        },
    ];

    // Blueprint directory (always present when on-chain is selected)
    let has_on_chain = selection
        .assignments
        .iter()
        .any(|a| a.role == Role::OnChain);
    if has_on_chain {
        entries.push(FileEntry {
            dest: PathBuf::from("blueprint/.gitkeep"),
            source: TemplateSource::Inline(Vec::new()),
            render: false,
        });
    }

    // --- Role layers ---
    for assignment in &selection.assignments {
        let tool =
            registry
                .get(&assignment.tool_id)
                .ok_or_else(|| ScaffoldError::ToolNotFound {
                    tool_id: assignment.tool_id.clone(),
                })?;

        let role_config =
            tool.roles
                .get(&assignment.role)
                .ok_or_else(|| ScaffoldError::RoleMismatch {
                    tool_id: assignment.tool_id.clone(),
                    role: assignment.role.to_string(),
                })?;

        let template_path = &role_config.template; // e.g., "aiken/on-chain"
        let role_dir = assignment.role.dir(); // e.g., "on-chain"

        // For infrastructure, each tool gets its own subdirectory
        let dest_prefix = if assignment.role == Role::Infrastructure {
            PathBuf::from(role_dir).join(&assignment.tool_id)
        } else {
            PathBuf::from(role_dir)
        };

        // Read the manifest
        let manifest_key = format!("{template_path}/manifest.toml");
        let manifest_data =
            TemplateAssets::get(&manifest_key).ok_or_else(|| ScaffoldError::AssetNotFound {
                path: manifest_key.clone(),
            })?;
        let manifest_text =
            std::str::from_utf8(&manifest_data.data).expect("manifest.toml must be valid UTF-8");
        let manifest: ManifestToml =
            toml::from_str(manifest_text).map_err(|e| ScaffoldError::ManifestParse {
                path: manifest_key,
                source: e,
            })?;

        for file in &manifest.files {
            entries.push(FileEntry {
                dest: dest_prefix.join(&file.dest),
                source: TemplateSource::Role(format!("{}/{}", template_path, file.source)),
                render: file.render,
            });
        }
    }

    // --- Optional layers ---
    if selection.nix {
        entries.push(FileEntry {
            dest: PathBuf::from("flake.nix"),
            source: TemplateSource::Optional("_nix/flake.nix.jinja".into()),
            render: true,
        });
        entries.push(FileEntry {
            dest: PathBuf::from(".envrc"),
            source: TemplateSource::Inline(b"use flake\n".to_vec()),
            render: false,
        });
    }

    Ok(FilePlan { entries })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, RoleAssignment};

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "test-project".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
    }

    #[test]
    fn base_files_always_present() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"Justfile"));
        assert!(dests.contains(&"README.md"));
        assert!(dests.contains(&".gitignore"));
        assert!(dests.contains(&".env"));
    }

    #[test]
    fn blueprint_gitkeep_when_on_chain() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"blueprint/.gitkeep"));
    }

    #[test]
    fn no_blueprint_without_on_chain() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(!dests.contains(&"blueprint/.gitkeep"));
    }

    #[test]
    fn aiken_on_chain_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"on-chain/aiken.toml"));
        assert!(dests.contains(&"on-chain/Justfile"));
        assert!(dests.contains(&"on-chain/validators/example.ak"));
    }

    #[test]
    fn meshjs_off_chain_entries() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"off-chain/package.json"));
        assert!(dests.contains(&"off-chain/Justfile"));
        assert!(dests.contains(&"off-chain/src/index.ts"));
    }

    #[test]
    fn combined_selection_entry_count() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
        ]);
        let plan = plan(&sel, &registry()).unwrap();

        // base: 4 (Justfile, README, .gitignore, .env)
        // blueprint/.gitkeep: 1
        // aiken on-chain: 3 (aiken.toml, Justfile, validators/example.ak)
        // meshjs off-chain: 3 (package.json, Justfile, src/index.ts)
        // total: 11
        assert_eq!(plan.entries.len(), 11);
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "nonexistent".into(),
        }]);
        assert!(matches!(
            plan(&sel, &registry()),
            Err(ScaffoldError::ToolNotFound { .. })
        ));
    }

    #[test]
    fn nix_true_includes_flake() {
        let mut sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        sel.nix = true;
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(dests.contains(&"flake.nix"));
    }

    #[test]
    fn nix_false_excludes_flake() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let plan = plan(&sel, &registry()).unwrap();

        let dests: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().unwrap())
            .collect();
        assert!(!dests.contains(&"flake.nix"));
    }
}
