use std::collections::HashMap;

use serde::Serialize;

use super::ScaffoldError;
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Per-role information available to templates.
#[derive(Debug, Clone, Serialize)]
pub struct RoleContext {
    pub tool_id: String,
    pub tool_name: String,
    pub language: String,
    pub dir: String,
}

/// The complete context passed to MiniJinja templates.
#[derive(Debug, Serialize)]
pub struct TemplateContext {
    pub project_name: String,
    pub network: String,

    pub has_on_chain: bool,
    pub has_off_chain: bool,
    pub has_infra: bool,
    pub has_testing: bool,
    pub has_formal_methods: bool,

    pub on_chain: Option<RoleContext>,
    pub off_chain: Option<RoleContext>,
    pub infra_tools: Vec<RoleContext>,
    pub testing: Option<RoleContext>,
    pub formal_methods: Option<RoleContext>,

    pub blueprint_path: String,
    pub env_vars: HashMap<String, String>,

    pub nix: bool,
    pub nix_packages: Vec<String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build a `TemplateContext` from a `Selection` and the tool `Registry`.
pub fn build_context(
    selection: &Selection,
    registry: &Registry,
) -> Result<TemplateContext, ScaffoldError> {
    let mut on_chain = None;
    let mut off_chain = None;
    let mut infra_tools = Vec::new();
    let mut testing = None;
    let mut formal_methods = None;
    let mut nix_packages = Vec::new();

    for assignment in &selection.assignments {
        let tool =
            registry
                .get(&assignment.tool_id)
                .ok_or_else(|| ScaffoldError::ToolNotFound {
                    tool_id: assignment.tool_id.clone(),
                })?;

        if !tool.roles.contains_key(&assignment.role) {
            return Err(ScaffoldError::RoleMismatch {
                tool_id: assignment.tool_id.clone(),
                role: assignment.role.to_string(),
            });
        }

        let rc = RoleContext {
            tool_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            language: tool.languages.first().cloned().unwrap_or_default(),
            dir: assignment.role.dir().to_string(),
        };

        for pkg in &tool.nix_packages {
            if !nix_packages.contains(pkg) {
                nix_packages.push(pkg.clone());
            }
        }

        match assignment.role {
            Role::OnChain => on_chain = Some(rc),
            Role::OffChain => off_chain = Some(rc),
            Role::Infrastructure => infra_tools.push(rc),
            Role::Testing => testing = Some(rc),
            Role::FormalMethods => formal_methods = Some(rc),
        }
    }

    let mut env_vars = HashMap::new();
    env_vars.insert(
        contract::ENV_NETWORK.to_string(),
        selection.network.to_string(),
    );
    env_vars.insert(contract::ENV_INDEXER_URL.to_string(), String::new());
    env_vars.insert(contract::ENV_INDEXER_PORT.to_string(), String::new());
    env_vars.insert(contract::ENV_NODE_SOCKET_PATH.to_string(), String::new());

    Ok(TemplateContext {
        project_name: selection.project_name.clone(),
        network: selection.network.to_string(),

        has_on_chain: on_chain.is_some(),
        has_off_chain: off_chain.is_some(),
        has_infra: !infra_tools.is_empty(),
        has_testing: testing.is_some(),
        has_formal_methods: formal_methods.is_some(),

        on_chain,
        off_chain,
        infra_tools,
        testing,
        formal_methods,

        blueprint_path: contract::BLUEPRINT_PATH.to_string(),
        env_vars,

        nix: selection.nix,
        nix_packages,
    })
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
    fn context_with_all_roles() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
            RoleAssignment {
                role: Role::Testing,
                tool_id: "scalus".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(ctx.has_testing);

        assert_eq!(ctx.on_chain.as_ref().unwrap().tool_id, "aiken");
        assert_eq!(ctx.off_chain.as_ref().unwrap().tool_id, "meshjs");
        assert_eq!(ctx.testing.as_ref().unwrap().tool_id, "scalus");
    }

    #[test]
    fn context_on_chain_only() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(!ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(!ctx.has_testing);
        assert!(!ctx.has_formal_methods);
        assert!(ctx.off_chain.is_none());
        assert!(ctx.testing.is_none());
        assert!(ctx.formal_methods.is_none());
        assert!(ctx.infra_tools.is_empty());
    }

    #[test]
    fn has_flags_match_assignments() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(!ctx.has_on_chain);
        assert!(ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(!ctx.has_testing);
        assert!(!ctx.has_formal_methods);
    }

    #[test]
    fn context_with_formal_methods() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::FormalMethods,
                tool_id: "blaster".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(ctx.has_formal_methods);
        assert_eq!(ctx.formal_methods.as_ref().unwrap().tool_id, "blaster");
        assert_eq!(ctx.formal_methods.as_ref().unwrap().dir, "formal-methods");
    }

    #[test]
    fn contract_constants_propagated() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.blueprint_path, "blueprint/plutus.json");
        assert_eq!(ctx.network, "preview");
        assert!(ctx.env_vars.contains_key("CARDANO_NETWORK"));
    }

    #[test]
    fn role_dirs_match_contract() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
            RoleAssignment {
                role: Role::Testing,
                tool_id: "scalus".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.on_chain.as_ref().unwrap().dir, "on-chain");
        assert_eq!(ctx.off_chain.as_ref().unwrap().dir, "off-chain");
        assert_eq!(ctx.testing.as_ref().unwrap().dir, "test");
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "nonexistent".into(),
        }]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::ToolNotFound { .. })));
    }

    #[test]
    fn role_mismatch_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::Testing,
            tool_id: "aiken".into(),
        }]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::RoleMismatch { .. })));
    }

    #[test]
    fn nix_packages_collected() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();
        assert!(ctx.nix_packages.contains(&"aiken".to_string()));
    }

    #[test]
    fn nix_packages_deduped_across_tools() {
        // Scalus on-chain + scalus testing — same tool, same nix_packages
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "scalus".into(),
            },
            RoleAssignment {
                role: Role::Testing,
                tool_id: "scalus".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();
        // sbt and jdk should appear only once each
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "sbt").count(), 1);
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "jdk").count(), 1);
    }
}
