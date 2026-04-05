/// Canonical path where on-chain builds produce the CIP-57 blueprint.
/// Relative to project root.
pub const BLUEPRINT_PATH: &str = "blueprint/plutus.json";

/// Directory names for each role. The role template is emitted into this directory.
pub const DIR_ON_CHAIN: &str = "on-chain";
pub const DIR_OFF_CHAIN: &str = "off-chain";
pub const DIR_INFRA: &str = "infra";
pub const DIR_TESTING: &str = "test";

/// Standard environment variable names for infrastructure.
/// Infra templates write these to .env; consumers read them.
pub const ENV_INDEXER_URL: &str = "INDEXER_URL";
pub const ENV_INDEXER_PORT: &str = "INDEXER_PORT";
pub const ENV_NODE_SOCKET_PATH: &str = "NODE_SOCKET_PATH";
pub const ENV_NETWORK: &str = "CARDANO_NETWORK";
