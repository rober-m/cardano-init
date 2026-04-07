use super::CliError;
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

/// Build a `Selection` from one-shot CLI flags.
///
/// Assumes `args.name` is `Some` (caller verified before calling this).
pub fn build_selection(
    name: &str,
    on_chain: Option<&str>,
    off_chain: Option<&str>,
    infra: &[String],
    testing: Option<&str>,
    network: &str,
    nix: bool,
    registry: &Registry,
) -> Result<Selection, CliError> {
    validate_project_name(name)?;

    let mut assignments = Vec::new();

    if let Some(tool_id) = on_chain {
        validate_tool_for_role(tool_id, Role::OnChain, registry)?;
        assignments.push(RoleAssignment {
            role: Role::OnChain,
            tool_id: tool_id.to_string(),
        });
    }

    if let Some(tool_id) = off_chain {
        validate_tool_for_role(tool_id, Role::OffChain, registry)?;
        assignments.push(RoleAssignment {
            role: Role::OffChain,
            tool_id: tool_id.to_string(),
        });
    }

    for tool_id in infra {
        validate_tool_for_role(tool_id, Role::Infrastructure, registry)?;
        assignments.push(RoleAssignment {
            role: Role::Infrastructure,
            tool_id: tool_id.clone(),
        });
    }

    if let Some(tool_id) = testing {
        validate_tool_for_role(tool_id, Role::Testing, registry)?;
        assignments.push(RoleAssignment {
            role: Role::Testing,
            tool_id: tool_id.to_string(),
        });
    }

    if assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    let network = Network::from_str(network).map_err(|_| CliError::InvalidNetwork {
        value: network.to_string(),
    })?;

    Ok(Selection {
        project_name: name.to_string(),
        assignments,
        network,
        nix,
    })
}

/// Validate a project name: non-empty, no path separators, no leading dots,
/// only alphanumeric + hyphens + underscores.
pub fn validate_project_name(name: &str) -> Result<(), CliError> {
    if name.is_empty() {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if name.starts_with('.') {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "must not start with a dot".to_string(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "may only contain letters, digits, hyphens, and underscores".to_string(),
        });
    }
    Ok(())
}

fn validate_tool_for_role(tool_id: &str, role: Role, registry: &Registry) -> Result<(), CliError> {
    let tool = registry.get(tool_id).ok_or_else(|| CliError::UnknownTool {
        tool_id: tool_id.to_string(),
        role: role.to_string(),
    })?;
    if !tool.roles.contains_key(&role) {
        return Err(CliError::ToolRoleMismatch {
            tool_id: tool_id.to_string(),
            role: role.to_string(),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    #[test]
    fn valid_single_role() {
        let sel = build_selection(
            "my-project",
            Some("aiken"),
            None,
            &[],
            None,
            "preview",
            false,
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.project_name, "my-project");
        assert_eq!(sel.assignments.len(), 1);
        assert_eq!(sel.assignments[0].tool_id, "aiken");
        assert_eq!(sel.assignments[0].role, Role::OnChain);
    }

    #[test]
    fn valid_multiple_roles() {
        let sel = build_selection(
            "test-proj",
            Some("aiken"),
            Some("meshjs"),
            &[],
            Some("scalus"),
            "preprod",
            true,
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.assignments.len(), 3);
        assert!(sel.nix);
        assert_eq!(sel.network.to_string(), "preprod");
    }

    #[test]
    fn unknown_tool_errors() {
        let result = build_selection(
            "test",
            Some("nonexistent"),
            None,
            &[],
            None,
            "preview",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::UnknownTool { .. })));
    }

    #[test]
    fn tool_role_mismatch_errors() {
        // Aiken doesn't support off-chain
        let result = build_selection(
            "test",
            None,
            Some("aiken"),
            &[],
            None,
            "preview",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::ToolRoleMismatch { .. })));
    }

    #[test]
    fn no_roles_errors() {
        let result = build_selection("test", None, None, &[], None, "preview", false, &registry());
        assert!(matches!(result, Err(CliError::NoRolesSelected)));
    }

    #[test]
    fn invalid_network_errors() {
        let result = build_selection(
            "test",
            Some("aiken"),
            None,
            &[],
            None,
            "badnet",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidNetwork { .. })));
    }

    #[test]
    fn invalid_project_name_empty() {
        let result = build_selection(
            "",
            Some("aiken"),
            None,
            &[],
            None,
            "preview",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }

    #[test]
    fn invalid_project_name_dot() {
        let result = build_selection(
            ".hidden",
            Some("aiken"),
            None,
            &[],
            None,
            "preview",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }

    #[test]
    fn invalid_project_name_slash() {
        let result = build_selection(
            "bad/name",
            Some("aiken"),
            None,
            &[],
            None,
            "preview",
            false,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }
}
