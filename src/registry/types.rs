use std::collections::HashMap;
use std::fmt;

use crate::contract;

// ---------------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------------

/// The functional roles a tool can fill within a Cardano protocol project.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    OnChain,
    OffChain,
    Infrastructure,
    Testing,
}

impl Role {
    /// All role variants, in display order.
    pub const ALL: &[Role] = &[
        Role::OnChain,
        Role::OffChain,
        Role::Infrastructure,
        Role::Testing,
    ];

    /// Parse from the kebab-case string used in TOML registry files.
    pub fn from_kebab(s: &str) -> Result<Self, UnknownRoleError> {
        match s {
            "on-chain" => Ok(Role::OnChain),
            "off-chain" => Ok(Role::OffChain),
            "infrastructure" => Ok(Role::Infrastructure),
            "testing" => Ok(Role::Testing),
            _ => Err(UnknownRoleError(s.to_string())),
        }
    }

    /// The kebab-case string used in TOML registry files.
    pub fn as_kebab(&self) -> &'static str {
        match self {
            Role::OnChain => "on-chain",
            Role::OffChain => "off-chain",
            Role::Infrastructure => "infrastructure",
            Role::Testing => "testing",
        }
    }

    /// The directory name for this role, as defined by the interface contract.
    pub fn dir(&self) -> &'static str {
        match self {
            Role::OnChain => contract::DIR_ON_CHAIN,
            Role::OffChain => contract::DIR_OFF_CHAIN,
            Role::Infrastructure => contract::DIR_INFRA,
            Role::Testing => contract::DIR_TESTING,
        }
    }
}

/// Human-readable display: "On-chain", "Off-chain", etc.
impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::OnChain => write!(f, "On-chain"),
            Role::OffChain => write!(f, "Off-chain"),
            Role::Infrastructure => write!(f, "Infrastructure"),
            Role::Testing => write!(f, "Testing"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnknownRoleError(pub String);

impl fmt::Display for UnknownRoleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown role: '{}'", self.0)
    }
}

impl std::error::Error for UnknownRoleError {}

// ---------------------------------------------------------------------------
// ToolDef / RoleConfig
// ---------------------------------------------------------------------------

/// Per-role configuration for a tool.
#[derive(Debug, Clone)]
pub struct RoleConfig {
    /// Path under `templates/` for this tool-role combination.
    pub template: String,
}

/// A loaded tool definition from the registry.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website: String,
    pub languages: Vec<String>,
    pub nix_packages: Vec<String>,
    pub roles: HashMap<Role, RoleConfig>,
}

// ---------------------------------------------------------------------------
// Selection / RoleAssignment / Network
// ---------------------------------------------------------------------------

/// One tool assigned to one role.
#[derive(Debug, Clone)]
pub struct RoleAssignment {
    pub role: Role,
    pub tool_id: String,
}

/// Target Cardano network.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Network {
    Preview,
    Preprod,
    Mainnet,
}

impl Network {
    pub fn from_str(s: &str) -> Result<Self, UnknownNetworkError> {
        match s {
            "preview" => Ok(Network::Preview),
            "preprod" => Ok(Network::Preprod),
            "mainnet" => Ok(Network::Mainnet),
            _ => Err(UnknownNetworkError(s.to_string())),
        }
    }
}

impl fmt::Display for Network {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Network::Preview => write!(f, "preview"),
            Network::Preprod => write!(f, "preprod"),
            Network::Mainnet => write!(f, "mainnet"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnknownNetworkError(pub String);

impl fmt::Display for UnknownNetworkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown network: '{}' (expected preview, preprod, or mainnet)",
            self.0
        )
    }
}

impl std::error::Error for UnknownNetworkError {}

/// The complete, fully resolved user selection.
#[derive(Debug, Clone)]
pub struct Selection {
    pub project_name: String,
    pub assignments: Vec<RoleAssignment>,
    pub network: Network,
    pub nix: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_from_kebab_valid() {
        assert_eq!(Role::from_kebab("on-chain").unwrap(), Role::OnChain);
        assert_eq!(Role::from_kebab("off-chain").unwrap(), Role::OffChain);
        assert_eq!(
            Role::from_kebab("infrastructure").unwrap(),
            Role::Infrastructure
        );
        assert_eq!(Role::from_kebab("testing").unwrap(), Role::Testing);
    }

    #[test]
    fn role_from_kebab_invalid() {
        assert!(Role::from_kebab("onchain").is_err());
        assert!(Role::from_kebab("").is_err());
        assert!(Role::from_kebab("build").is_err());
    }

    #[test]
    fn role_kebab_round_trip() {
        for role in Role::ALL {
            let kebab = role.as_kebab();
            let parsed = Role::from_kebab(kebab).unwrap();
            assert_eq!(*role, parsed);
        }
    }

    #[test]
    fn role_dir_matches_contract() {
        assert_eq!(Role::OnChain.dir(), "on-chain");
        assert_eq!(Role::OffChain.dir(), "off-chain");
        assert_eq!(Role::Infrastructure.dir(), "infra");
        assert_eq!(Role::Testing.dir(), "test");
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::OnChain.to_string(), "On-chain");
        assert_eq!(Role::OffChain.to_string(), "Off-chain");
        assert_eq!(Role::Infrastructure.to_string(), "Infrastructure");
        assert_eq!(Role::Testing.to_string(), "Testing");
    }

    #[test]
    fn role_all_has_four_variants() {
        assert_eq!(Role::ALL.len(), 4);
    }

    #[test]
    fn network_display_and_parse() {
        for (s, expected) in [
            ("preview", Network::Preview),
            ("preprod", Network::Preprod),
            ("mainnet", Network::Mainnet),
        ] {
            let parsed = Network::from_str(s).unwrap();
            assert_eq!(parsed, expected);
            assert_eq!(parsed.to_string(), s);
        }
    }

    #[test]
    fn network_parse_invalid() {
        assert!(Network::from_str("testnet").is_err());
        assert!(Network::from_str("").is_err());
    }
}
