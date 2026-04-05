use std::collections::HashMap;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::types::{Role, RoleConfig, ToolDef, UnknownRoleError};

// ---------------------------------------------------------------------------
// Embedded assets
// ---------------------------------------------------------------------------

#[derive(RustEmbed)]
#[folder = "registry/"]
struct RegistryAssets;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("failed to parse tool file '{file}': {source}")]
    Parse {
        file: String,
        source: toml::de::Error,
    },

    #[error("unknown role '{role}' in tool file '{file}': {source}")]
    UnknownRole {
        file: String,
        role: String,
        source: UnknownRoleError,
    },

    #[error("duplicate tool id '{id}'")]
    DuplicateId { id: String },

    #[error("no tool definitions found in registry")]
    Empty,
}

// ---------------------------------------------------------------------------
// TOML intermediate types (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ToolFileToml {
    tool: ToolMetaToml,
    #[serde(default)]
    roles: HashMap<String, RoleConfigToml>,
}

#[derive(Deserialize)]
struct ToolMetaToml {
    id: String,
    name: String,
    description: String,
    website: String,
    languages: Vec<String>,
    system_deps: Vec<String>,
}

#[derive(Deserialize)]
struct RoleConfigToml {
    template: String,
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

fn to_tool_def(file_name: &str, raw: ToolFileToml) -> Result<ToolDef, RegistryError> {
    let mut roles = HashMap::new();
    for (role_str, cfg) in raw.roles {
        let role = Role::from_kebab(&role_str).map_err(|e| RegistryError::UnknownRole {
            file: file_name.to_string(),
            role: role_str,
            source: e,
        })?;
        roles.insert(
            role,
            RoleConfig {
                template: cfg.template,
            },
        );
    }

    Ok(ToolDef {
        id: raw.tool.id,
        name: raw.tool.name,
        description: raw.tool.description,
        website: raw.tool.website,
        languages: raw.tool.languages,
        system_deps: raw.tool.system_deps,
        roles,
    })
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Immutable registry of all tool definitions, loaded from embedded TOML files.
pub struct Registry {
    tools: Vec<ToolDef>,
    by_id: HashMap<String, usize>,
    by_role: HashMap<Role, Vec<usize>>,
}

impl Registry {
    /// Load all tool definitions from the embedded `registry/tools/` directory.
    pub fn load() -> Result<Self, RegistryError> {
        let mut tools = Vec::new();
        let mut by_id = HashMap::new();
        let mut by_role: HashMap<Role, Vec<usize>> = HashMap::new();

        for file_name in RegistryAssets::iter() {
            if !file_name.starts_with("tools/") || !file_name.ends_with(".toml") {
                continue;
            }

            let data = RegistryAssets::get(&file_name).expect("asset listed by iter() must exist");
            let text =
                std::str::from_utf8(&data.data).expect("tool TOML files must be valid UTF-8");

            let raw: ToolFileToml = toml::from_str(text).map_err(|e| RegistryError::Parse {
                file: file_name.to_string(),
                source: e,
            })?;

            let tool = to_tool_def(&file_name, raw)?;
            let idx = tools.len();

            if by_id.contains_key(&tool.id) {
                return Err(RegistryError::DuplicateId {
                    id: tool.id.clone(),
                });
            }

            by_id.insert(tool.id.clone(), idx);
            for role in tool.roles.keys() {
                by_role.entry(*role).or_default().push(idx);
            }
            tools.push(tool);
        }

        if tools.is_empty() {
            return Err(RegistryError::Empty);
        }

        Ok(Registry {
            tools,
            by_id,
            by_role,
        })
    }

    /// Look up a tool by its id.
    pub fn get(&self, id: &str) -> Option<&ToolDef> {
        self.by_id.get(id).map(|&idx| &self.tools[idx])
    }

    /// All tools that can fill the given role.
    pub fn tools_for_role(&self, role: Role) -> Vec<&ToolDef> {
        self.by_role
            .get(&role)
            .map(|indices| indices.iter().map(|&idx| &self.tools[idx]).collect())
            .unwrap_or_default()
    }

    /// All loaded tool definitions.
    pub fn all_tools(&self) -> &[ToolDef] {
        &self.tools
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load successfully")
    }

    #[test]
    fn load_succeeds() {
        let _ = registry();
    }

    #[test]
    fn load_tool_count() {
        assert_eq!(registry().all_tools().len(), 3);
    }

    #[test]
    fn get_by_id() {
        let reg = registry();
        let aiken = reg.get("aiken").expect("aiken should exist");
        assert_eq!(aiken.name, "Aiken");
        assert_eq!(aiken.id, "aiken");
    }

    #[test]
    fn get_unknown_returns_none() {
        assert!(registry().get("nonexistent").is_none());
    }

    #[test]
    fn tools_for_role_on_chain() {
        let reg = registry();
        let on_chain = reg.tools_for_role(Role::OnChain);
        let mut ids: Vec<&str> = on_chain.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["aiken", "scalus"]);
    }

    #[test]
    fn tools_for_role_off_chain() {
        let reg = registry();
        let off_chain = reg.tools_for_role(Role::OffChain);
        let mut ids: Vec<&str> = off_chain.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["meshjs", "scalus"]);
    }

    #[test]
    fn tools_for_role_infra_empty() {
        let reg = registry();
        let infra = reg.tools_for_role(Role::Infrastructure);
        assert!(infra.is_empty());
    }

    #[test]
    fn scalus_multi_role() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        assert!(scalus.roles.contains_key(&Role::OnChain));
        assert!(scalus.roles.contains_key(&Role::OffChain));
        assert!(scalus.roles.contains_key(&Role::Testing));
        assert_eq!(scalus.roles.len(), 3);
    }

    #[test]
    fn all_fields_populated() {
        let reg = registry();
        for tool in reg.all_tools() {
            assert!(!tool.id.is_empty(), "id should not be empty");
            assert!(!tool.name.is_empty(), "name should not be empty");
            assert!(
                !tool.description.is_empty(),
                "description should not be empty"
            );
            assert!(!tool.website.is_empty(), "website should not be empty");
            assert!(!tool.languages.is_empty(), "languages should not be empty");
            assert!(!tool.roles.is_empty(), "roles should not be empty");

            for (role, cfg) in &tool.roles {
                assert!(
                    !cfg.template.is_empty(),
                    "template for {} in role {} should not be empty",
                    tool.id,
                    role
                );
            }
        }
    }
}
