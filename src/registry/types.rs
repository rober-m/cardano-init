use std::collections::HashMap;
use std::fmt;

use serde::Serialize;

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
    Devnet,
    FormalMethods,
}

impl Role {
    /// All role variants, in display order.
    pub const ALL: &[Role] = &[
        Role::OnChain,
        Role::OffChain,
        Role::Infrastructure,
        Role::Devnet,
        Role::FormalMethods,
    ];

    /// Parse from the kebab-case string used in TOML registry files.
    pub fn from_kebab(s: &str) -> Result<Self, UnknownRoleError> {
        match s {
            "on-chain" => Ok(Role::OnChain),
            "off-chain" => Ok(Role::OffChain),
            "infrastructure" => Ok(Role::Infrastructure),
            "devnet" => Ok(Role::Devnet),
            "formal-methods" => Ok(Role::FormalMethods),
            _ => Err(UnknownRoleError(s.to_string())),
        }
    }

    /// The kebab-case string used in TOML registry files.
    pub fn as_kebab(&self) -> &'static str {
        match self {
            Role::OnChain => "on-chain",
            Role::OffChain => "off-chain",
            Role::Infrastructure => "infrastructure",
            Role::Devnet => "devnet",
            Role::FormalMethods => "formal-methods",
        }
    }

    /// Whether this role may be filled by multiple tools at once. Only
    /// Infrastructure is multi-tool; every other role takes at most one tool.
    pub fn multiple(&self) -> bool {
        matches!(self, Role::Infrastructure)
    }

    /// The directory name for this role, as defined by the interface contract.
    pub fn dir(&self) -> &'static str {
        match self {
            Role::OnChain => contract::DIR_ON_CHAIN,
            Role::OffChain => contract::DIR_OFF_CHAIN,
            Role::Infrastructure => contract::DIR_INFRA,
            Role::Devnet => contract::DIR_DEVNET,
            Role::FormalMethods => contract::DIR_FORMAL_METHODS,
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
            Role::Devnet => write!(f, "Devnet"),
            Role::FormalMethods => write!(f, "Formal methods"),
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
// Seam / compatibility
// ---------------------------------------------------------------------------

/// A connection "seam": the wire protocol an off-chain tool speaks to reach a
/// chain endpoint, and that a devnet (or infra) tool exposes. Whether a given
/// off-chain tool can use a given devnet is decided purely by whether their
/// declared seams overlap — see [`crate::registry::compat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seam {
    /// Blockfrost-compatible REST (Yaci Store, Blockfrost, …).
    Blockfrost,
    /// Tx3 Transaction Resolve Protocol (JSON-RPC), served by Dolos/Demeter.
    Trp,
    /// UTxORPC (u5c) gRPC.
    U5c,
    /// Ogmios JSON-RPC (half of a Kupmios provider; also a standalone provider
    /// for tools that speak it directly).
    Ogmios,
    /// Kupo HTTP (the other half of a Kupmios provider).
    Kupo,
}

impl Seam {
    /// Parse from the kebab-case string used in TOML registry files.
    pub fn from_kebab(s: &str) -> Result<Self, UnknownSeamError> {
        match s {
            "blockfrost" => Ok(Seam::Blockfrost),
            "trp" => Ok(Seam::Trp),
            "u5c" => Ok(Seam::U5c),
            "ogmios" => Ok(Seam::Ogmios),
            "kupo" => Ok(Seam::Kupo),
            _ => Err(UnknownSeamError(s.to_string())),
        }
    }

    /// Human-readable label used in compatibility messages.
    pub fn label(&self) -> &'static str {
        match self {
            Seam::Blockfrost => "Blockfrost",
            Seam::Trp => "TRP",
            Seam::U5c => "UTxORPC",
            Seam::Ogmios => "Ogmios",
            Seam::Kupo => "Kupo",
        }
    }
}

impl fmt::Display for Seam {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.label())
    }
}

#[derive(Debug, Clone)]
pub struct UnknownSeamError(pub String);

impl fmt::Display for UnknownSeamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown seam: '{}'", self.0)
    }
}

impl std::error::Error for UnknownSeamError {}

/// Devnet-compatibility metadata for a tool, from its optional `[compat]` table.
///
/// Off-chain tools declare the seam(s) they `consumes`; devnet/infra tools
/// declare the seam(s) they `serves`. A `self_contained_devnet` tool (e.g. Tx3,
/// which bundles its own Dolos) needs no devnet role at all — pairing it with
/// one is reported as incompatible. Every field defaults empty/false, so a tool
/// without a `[compat]` table imposes no constraints. See
/// [`crate::registry::compat`].
#[derive(Debug, Clone, Default)]
pub struct CompatConfig {
    pub consumes: Vec<Seam>,
    pub serves: Vec<Seam>,
    pub self_contained_devnet: bool,
}

// ---------------------------------------------------------------------------
// ToolDef / RoleConfig
// ---------------------------------------------------------------------------

/// Per-role configuration for a tool.
#[derive(Debug, Clone)]
pub struct RoleConfig {
    /// Path under `templates/` for this tool-role combination.
    pub template: String,
    /// Optional role-specific system dependencies. When absent, the tool's
    /// top-level `system_deps` apply.
    pub system_deps: Option<Vec<String>>,
}

/// A single `cardano-up context env` output → contract `.env` key mapping,
/// declared by an infrastructure tool (`[[infra.env]]`). `from` is the
/// `cardano-up` output var name (e.g. `KUPO_URL`); `to` is the `.env` key the
/// generated infra component writes (e.g. `INDEXER_URL`). See TECH_SPEC §3.2.
#[derive(Debug, Clone, Serialize)]
pub struct EnvMapping {
    pub from: String,
    pub to: String,
}

/// Infrastructure-specific config for a tool that fills the infrastructure role.
/// Infra tools are thin data: a `cardano-up` package id plus the env mappings
/// that translate its outputs into the `.env` contract keys. All infra tools
/// share a single driver template and aggregate into one `infra/` component.
#[derive(Debug, Clone)]
pub struct InfraConfig {
    /// The package id passed to `cardano-up install`.
    pub cardano_up_package: String,
    /// `cardano-up` output → `.env` contract key mappings.
    pub env: Vec<EnvMapping>,
}

/// A signature that identifies a tool's generated output inside a role
/// directory (used by `doctor` to recognize the tool in a scanned project).
///
/// A signature matches when `file` (relative to the role dir) exists and, if
/// `contains` is set, the file's text contains that substring. The substring
/// form disambiguates generic filenames (e.g. a `package.json` is only MeshJS
/// if it references `@meshsdk`), so foreign projects fall into the
/// "unrecognized" bucket instead of being mislabeled (TECH_SPEC §9.6).
#[derive(Debug, Clone)]
pub struct DetectSignature {
    pub file: String,
    pub contains: Option<String>,
}

/// A loaded tool definition from the registry.
#[derive(Debug, Clone)]
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website: String,
    pub languages: Vec<String>,
    /// Dependency ids this tool requires; each must have a `registry/deps.toml`
    /// entry. Consumed by the dependency doctor (TECH_SPEC §9.1).
    pub system_deps: Vec<String>,
    pub nix_packages: Vec<String>,
    /// When `true`, this tool ships its own component-local Nix flake (emitted
    /// under `--nix` via a manifest `when = "nix"` guard). Its `nix_packages`
    /// are *not* folded into the top-level shell as bare nixpkgs attrs — a plain
    /// `mkShell` cannot build such a tool (e.g. Plinth needs haskell.nix + CHaP).
    /// Instead the top-level flake references the component as a `path:` input
    /// and pulls its dev shell in via `inputsFrom`, so the toolchain composes
    /// into the root shell. Defaults to `false`.
    pub nix_self_contained: bool,
    /// Signatures that identify this tool's generated output. Used by `doctor`
    /// to recognize the tool in a scanned project. Only tools that declare a
    /// role are candidates for that role's directory, which resolves
    /// on-chain/off-chain ambiguity.
    pub detect: Vec<DetectSignature>,
    pub roles: HashMap<Role, RoleConfig>,
    /// Infrastructure config. Required when the tool fills the infrastructure
    /// role (validated at load), `None` otherwise.
    pub infra: Option<InfraConfig>,
    /// Whether this tool is **experimental** — not production-ready, for either
    /// (or both) of two reasons:
    ///   1. the *upstream tool itself* is experimental — pre-release, unstable,
    ///      a work in progress (e.g. Blaster);
    ///   2. its *cardano-init integration* is not yet build-green — a placeholder
    ///      template, excluded from the build-green guarantees / smoke matrix.
    ///
    /// It still generates (so the role is present and demoable), but selecting it
    /// requires explicit opt-in (`--allow-experimental`, or the interactive
    /// confirm) and is surfaced as "experimental" across every presenter, so it
    /// is never scaffolded unknowingly. Per-tool, not per-role: a future
    /// production-ready formal-methods tool would set this `false` even though
    /// today's Blaster is `true` (ROADMAP Phase 0 formal-methods deliverable).
    pub experimental: bool,
    /// Unified on-chain + off-chain template. When present (and the tool fills
    /// both roles), assigning this tool to both collapses into a single
    /// `protocol/` component built from this template instead of two folders.
    /// `Some` requires both `Role::OnChain` and `Role::OffChain` (validated at
    /// load); `protocol` is a fused component, not a `Role`.
    pub fullstack: Option<RoleConfig>,
    /// Devnet-compatibility metadata (from `[compat]`). Drives the off-chain ↔
    /// devnet compatibility gate; defaults empty (no constraints).
    pub compat: CompatConfig,
}

// ---------------------------------------------------------------------------
// Selection / RoleAssignment / Network
// ---------------------------------------------------------------------------

/// One tool assigned to one role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleAssignment {
    pub role: Role,
    pub tool_id: String,
}

/// Target Cardano network. The scaffolder always emits [`Network::Preview`];
/// `Preprod`/`Mainnet` are the other valid `CARDANO_NETWORK` values a generated
/// `.env` can be switched to at runtime (hence never constructed here).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Network {
    Preview,
    Preprod,
    Mainnet,
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

impl std::str::FromStr for Network {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "preview" => Ok(Network::Preview),
            "preprod" => Ok(Network::Preprod),
            "mainnet" => Ok(Network::Mainnet),
            _ => Err(()),
        }
    }
}

/// The complete, fully resolved user selection.
#[derive(Debug, Clone, PartialEq, Eq)]
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
        assert_eq!(Role::from_kebab("devnet").unwrap(), Role::Devnet);
        assert_eq!(
            Role::from_kebab("formal-methods").unwrap(),
            Role::FormalMethods
        );
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
        assert_eq!(Role::Devnet.dir(), "devnet");
        assert_eq!(Role::FormalMethods.dir(), "formal-methods");
    }

    #[test]
    fn role_display() {
        assert_eq!(Role::OnChain.to_string(), "On-chain");
        assert_eq!(Role::OffChain.to_string(), "Off-chain");
        assert_eq!(Role::Infrastructure.to_string(), "Infrastructure");
        assert_eq!(Role::Devnet.to_string(), "Devnet");
        assert_eq!(Role::FormalMethods.to_string(), "Formal methods");
    }

    #[test]
    fn role_all_has_five_variants() {
        assert_eq!(Role::ALL.len(), 5);
    }

    #[test]
    fn network_display_matches_env_vocabulary() {
        // These strings are the valid `CARDANO_NETWORK` values in a generated
        // `.env`; Display must render them exactly (templates match on them).
        assert_eq!(Network::Preview.to_string(), "preview");
        assert_eq!(Network::Preprod.to_string(), "preprod");
        assert_eq!(Network::Mainnet.to_string(), "mainnet");
    }
}
