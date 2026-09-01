use std::collections::HashMap;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::types::{
    CompatConfig, DetectSignature, EnvMapping, InfraConfig, Role, RoleConfig, Seam, ToolDef,
    UnknownRoleError, UnknownSeamError,
};

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

    #[error("unknown seam '{seam}' in tool file '{file}': {source}")]
    UnknownSeam {
        file: String,
        seam: String,
        source: UnknownSeamError,
    },

    #[error("duplicate tool id '{id}'")]
    DuplicateId { id: String },

    #[error(
        "tool '{file}' fills the infrastructure role but is missing the required [infra] table"
    )]
    InfraConfigMissing { file: String },

    #[error(
        "tool '{file}' declares a [fullstack] template but must also declare both \
         [roles.on-chain] and [roles.off-chain]"
    )]
    FullstackRolesMissing { file: String },

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
    #[serde(default)]
    infra: Option<InfraToml>,
    #[serde(default)]
    fullstack: Option<RoleConfigToml>,
    #[serde(default)]
    compat: Option<CompatToml>,
}

#[derive(Deserialize)]
struct CompatToml {
    #[serde(default)]
    consumes: Vec<String>,
    #[serde(default)]
    serves: Vec<String>,
    #[serde(default)]
    self_contained_devnet: bool,
}

#[derive(Deserialize)]
struct InfraToml {
    cardano_up_package: String,
    #[serde(default)]
    env: Vec<EnvMappingToml>,
}

#[derive(Deserialize)]
struct EnvMappingToml {
    from: String,
    to: String,
}

#[derive(Deserialize)]
struct ToolMetaToml {
    id: String,
    name: String,
    description: String,
    website: String,
    languages: Vec<String>,
    #[serde(default)]
    system_deps: Vec<String>,
    #[serde(default)]
    nix_packages: Vec<String>,
    #[serde(default)]
    nix_self_contained: bool,
    #[serde(default)]
    detect: Vec<DetectToml>,
    #[serde(default)]
    experimental: bool,
}

#[derive(Deserialize)]
struct RoleConfigToml {
    template: String,
    #[serde(default)]
    system_deps: Option<Vec<String>>,
}

/// A `detect` entry is either a bare path (existence check) or a table with an
/// optional `contains` substring (content check).
#[derive(Deserialize)]
#[serde(untagged)]
enum DetectToml {
    Simple(String),
    Detailed {
        file: String,
        #[serde(default)]
        contains: Option<String>,
    },
}

impl DetectToml {
    fn into_signature(self) -> DetectSignature {
        match self {
            DetectToml::Simple(file) => DetectSignature {
                file,
                contains: None,
            },
            DetectToml::Detailed { file, contains } => DetectSignature { file, contains },
        }
    }
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
                system_deps: cfg.system_deps,
            },
        );
    }

    // A tool that fills the infrastructure role must declare [infra] (its
    // cardano-up package + env mappings). The driver template aggregates these.
    if roles.contains_key(&Role::Infrastructure) && raw.infra.is_none() {
        return Err(RegistryError::InfraConfigMissing {
            file: file_name.to_string(),
        });
    }

    let infra = raw.infra.map(|i| InfraConfig {
        cardano_up_package: i.cardano_up_package,
        env: i
            .env
            .into_iter()
            .map(|m| EnvMapping {
                from: m.from,
                to: m.to,
            })
            .collect(),
    });

    // A [fullstack] template collapses on-chain + off-chain into one `protocol/`
    // component, so the tool must be able to fill both roles.
    if raw.fullstack.is_some()
        && !(roles.contains_key(&Role::OnChain) && roles.contains_key(&Role::OffChain))
    {
        return Err(RegistryError::FullstackRolesMissing {
            file: file_name.to_string(),
        });
    }

    let fullstack = raw.fullstack.map(|f| RoleConfig {
        template: f.template,
        system_deps: f.system_deps,
    });

    let compat = match raw.compat {
        Some(c) => CompatConfig {
            consumes: parse_seams(file_name, &c.consumes)?,
            serves: parse_seams(file_name, &c.serves)?,
            self_contained_devnet: c.self_contained_devnet,
        },
        None => CompatConfig::default(),
    };

    Ok(ToolDef {
        id: raw.tool.id,
        name: raw.tool.name,
        description: raw.tool.description,
        website: raw.tool.website,
        languages: raw.tool.languages,
        system_deps: raw.tool.system_deps,
        nix_packages: raw.tool.nix_packages,
        nix_self_contained: raw.tool.nix_self_contained,
        detect: raw
            .tool
            .detect
            .into_iter()
            .map(DetectToml::into_signature)
            .collect(),
        roles,
        infra,
        fullstack,
        experimental: raw.tool.experimental,
        compat,
    })
}

/// Parse a list of kebab-case seam strings, surfacing an unknown value as a
/// registry load error (like an unknown role) rather than silently dropping it.
fn parse_seams(file_name: &str, raw: &[String]) -> Result<Vec<Seam>, RegistryError> {
    raw.iter()
        .map(|s| {
            Seam::from_kebab(s).map_err(|e| RegistryError::UnknownSeam {
                file: file_name.to_string(),
                seam: s.clone(),
                source: e,
            })
        })
        .collect()
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
        // aiken, plinth, scalus, meshjs, evolution, tx3, yaci, blaster + infra:
        // kupo, ogmios, dolos, tx-submit-api, cardano-node, cardano-node-api, dingo
        assert_eq!(registry().all_tools().len(), 15);
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
        assert_eq!(ids, vec!["aiken", "plinth", "scalus"]);
    }

    #[test]
    fn tools_for_role_off_chain() {
        let reg = registry();
        let off_chain = reg.tools_for_role(Role::OffChain);
        let mut ids: Vec<&str> = off_chain.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["evolution", "meshjs", "scalus", "tx3"]);
    }

    #[test]
    fn tools_for_role_devnet() {
        let reg = registry();
        let devnet = reg.tools_for_role(Role::Devnet);
        let mut ids: Vec<&str> = devnet.iter().map(|t| t.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["dingo", "yaci"]);
    }

    #[test]
    fn infrastructure_tools_present() {
        // Infra is filled by cardano-up-backed providers. Each shares the single
        // driver template and declares an [infra] config.
        let reg = registry();
        let mut ids: Vec<&str> = reg
            .tools_for_role(Role::Infrastructure)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        ids.sort();
        assert_eq!(
            ids,
            vec![
                "cardano-node",
                "cardano-node-api",
                "dingo",
                "dolos",
                "kupo",
                "ogmios",
                "tx-submit-api",
            ]
        );
    }

    #[test]
    fn infra_tools_declare_infra_config() {
        let reg = registry();
        for tool in reg.tools_for_role(Role::Infrastructure) {
            let infra = tool
                .infra
                .as_ref()
                .unwrap_or_else(|| panic!("infra tool '{}' must declare [infra]", tool.id));
            assert!(
                !infra.cardano_up_package.is_empty(),
                "infra tool '{}' needs a cardano_up_package",
                tool.id
            );
            // All infra tools share the single aggregated driver template.
            assert_eq!(
                tool.roles.get(&Role::Infrastructure).unwrap().template,
                "_infra/cardano-up"
            );
        }
    }

    #[test]
    fn experimental_flag_defaults_false_and_marks_blaster() {
        let reg = registry();
        // Blaster is the current formal-methods experimental tool (work in progress).
        assert!(
            reg.get("blaster")
                .expect("blaster should exist")
                .experimental,
            "blaster should be marked experimental"
        );
        // Released tools default to non-experimental (no flag needed in their TOML).
        for id in ["aiken", "meshjs", "scalus", "yaci", "kupo"] {
            assert!(
                !reg.get(id).unwrap().experimental,
                "{id} should not be experimental by default"
            );
        }
    }

    #[test]
    fn experimental_defaults_false_when_flag_absent() {
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]

            [roles.on-chain]
            template = "demo/on-chain"
        "#;
        let raw: ToolFileToml = toml::from_str(toml).unwrap();
        let def = to_tool_def("demo.toml", raw).unwrap();
        assert!(!def.experimental);
    }

    #[test]
    fn scalus_multi_role() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        assert!(scalus.roles.contains_key(&Role::OnChain));
        assert!(scalus.roles.contains_key(&Role::OffChain));
        assert_eq!(scalus.roles.len(), 2);
    }

    #[test]
    fn scalus_declares_fullstack_template() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        let fs = scalus
            .fullstack
            .as_ref()
            .expect("scalus should declare a [fullstack] template");
        assert_eq!(fs.template, "scalus/fullstack");
        // Aiken (on-chain only) has no fullstack capability.
        assert!(reg.get("aiken").unwrap().fullstack.is_none());
    }

    #[test]
    fn fullstack_with_both_roles_loads() {
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]

            [roles.on-chain]
            template = "demo/on-chain"
            [roles.off-chain]
            template = "demo/off-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        let raw: ToolFileToml = toml::from_str(toml).unwrap();
        let def = to_tool_def("demo.toml", raw).expect("valid fullstack tool");
        assert_eq!(def.fullstack.unwrap().template, "demo/fullstack");
    }

    #[test]
    fn fullstack_without_both_roles_is_rejected() {
        // [fullstack] but only on-chain declared → error (must fill both roles).
        let toml = r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]

            [roles.on-chain]
            template = "demo/on-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        let raw: ToolFileToml = toml::from_str(toml).unwrap();
        assert!(matches!(
            to_tool_def("demo.toml", raw),
            Err(RegistryError::FullstackRolesMissing { .. })
        ));
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
            // Infrastructure providers (e.g. kupo, ogmios) are not authored in a
            // user-facing language — they're cardano-up packages — so `languages`
            // may legitimately be empty for an infra-only tool. Every other tool
            // must declare at least one language.
            if !tool.roles.contains_key(&Role::Infrastructure) {
                assert!(!tool.languages.is_empty(), "languages should not be empty");
            }
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
