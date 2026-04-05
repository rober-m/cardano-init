pub mod interactive;
pub mod oneshot;
pub mod output;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser};

use crate::registry::loader::{Registry, RegistryError};
use crate::registry::types::ToolDef;
use crate::scaffold::ScaffoldError;

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

/// Scaffold a new Cardano protocol project.
#[derive(Parser, Debug)]
#[command(name = "cardano-init", version, about)]
pub struct Args {
    /// Project name (required in one-shot mode)
    #[arg(long)]
    pub name: Option<String>,

    /// On-chain tool (e.g., aiken, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (e.g., meshjs, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Infrastructure tool (repeatable: --infra kupo --infra ogmios)
    #[arg(long, value_name = "TOOL_ID")]
    pub infra: Vec<String>,

    /// Testing tool (e.g., scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub testing: Option<String>,

    /// Target network
    #[arg(long, default_value = "preview")]
    pub network: String,

    /// Generate Nix flake for dependency management
    #[arg(long)]
    pub nix: bool,

    /// Generate Docker configuration
    #[arg(long)]
    pub docker: bool,

    /// Show what would be generated without writing to disk
    #[arg(long)]
    pub dry_run: bool,
}

impl Args {
    /// Returns true if any one-shot flags were provided.
    fn has_oneshot_flags(&self) -> bool {
        self.on_chain.is_some()
            || self.off_chain.is_some()
            || !self.infra.is_empty()
            || self.testing.is_some()
            || self.nix
            || self.docker
            || self.dry_run
            || self.network != "preview"
    }
}

// ---------------------------------------------------------------------------
// CLI errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{0}")]
    Registry(#[from] RegistryError),

    #[error("{0}")]
    Scaffold(#[from] ScaffoldError),

    #[error("directory '{}' already exists — refusing to overwrite", path)]
    DirectoryExists { path: String },

    #[error("unknown tool '{}' for role {}", tool_id, role)]
    UnknownTool { tool_id: String, role: String },

    #[error("tool '{}' does not support role '{}'", tool_id, role)]
    ToolRoleMismatch { tool_id: String, role: String },

    #[error("no roles selected — at least one role must be provided")]
    NoRolesSelected,

    #[error("invalid network '{}' — expected preview, preprod, or mainnet", value)]
    InvalidNetwork { value: String },

    #[error("invalid project name '{}' — {}", name, reason)]
    InvalidProjectName { name: String, reason: String },

    #[error("--name is required when using one-shot flags (--on-chain, --off-chain, etc.)\n\n  Run without flags for interactive mode, or provide --name:\n\n    cardano-init --name my-protocol --on-chain aiken")]
    NameRequired,

    #[error("user aborted")]
    Aborted,

    #[error("prompt error: {0}")]
    Prompt(#[from] dialoguer::Error),
}

// ---------------------------------------------------------------------------
// Tool catalog for --help
// ---------------------------------------------------------------------------

/// Build the "Available tools" section appended to --help output.
fn build_tool_catalog(registry: &Registry) -> String {
    use std::fmt::Write;

    let mut out = String::from("Available tools:\n");

    for tool in registry.all_tools() {
        out.push('\n');
        format_tool(&mut out, tool);
    }

    let _ = writeln!(out, "\nExamples:");
    let _ = writeln!(out, "  cardano-init                                        # interactive mode");
    let _ = writeln!(out, "  cardano-init --name my-app --on-chain aiken         # one-shot, single role");
    let _ = writeln!(out, "  cardano-init --name my-app --on-chain aiken --off-chain meshjs --nix");

    out
}

fn format_tool(out: &mut String, tool: &ToolDef) {
    use std::fmt::Write;

    let mut roles: Vec<&str> = tool.roles.keys().map(|r| r.as_kebab()).collect();
    roles.sort();
    let _ = writeln!(out, "  {} ({})", tool.name, tool.id);
    let _ = writeln!(out, "    Roles:     {}", roles.join(", "));
    let _ = writeln!(out, "    Languages: {}", tool.languages.join(", "));
    let _ = writeln!(out, "    Website:   {}", tool.website);

    // Wrap description to ~72 chars with 4-space indent
    let _ = write!(out, "    ");
    let mut col = 4;
    for word in tool.description.split_whitespace() {
        if col + word.len() + 1 > 76 && col > 4 {
            let _ = write!(out, "\n    ");
            col = 4;
        }
        if col > 4 {
            let _ = write!(out, " ");
            col += 1;
        }
        let _ = write!(out, "{word}");
        col += word.len();
    }
    let _ = writeln!(out);
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Main CLI entry point. Parse args, dispatch to the appropriate mode,
/// and run the scaffolding pipeline.
pub fn run() -> Result<(), CliError> {
    let registry = Registry::load()?;

    // Build clap command with dynamic after_help containing tool catalog
    let catalog = build_tool_catalog(&registry);
    let cmd = Args::command().after_help(catalog);
    let matches = cmd.get_matches();
    let args = Args::from_arg_matches(&matches).expect("clap already validated");

    // If flags are provided without --name, error out
    if args.name.is_none() && args.has_oneshot_flags() {
        return Err(CliError::NameRequired);
    }

    // Decide mode: one-shot if --name provided, interactive otherwise
    let selection = if let Some(ref name) = args.name {
        oneshot::build_selection(
            name,
            args.on_chain.as_deref(),
            args.off_chain.as_deref(),
            &args.infra,
            args.testing.as_deref(),
            &args.network,
            args.nix,
            args.docker,
            &registry,
        )?
    } else {
        interactive::run_interactive(&registry)?
    };

    let root = PathBuf::from(&selection.project_name);

    // Safety: refuse to overwrite existing directory
    if root.exists() {
        return Err(CliError::DirectoryExists {
            path: selection.project_name.clone(),
        });
    }

    if args.dry_run {
        let plan = crate::scaffold::dry_run(&selection, &registry)?;
        output::print_dry_run(&selection, &registry, &plan);
        return Ok(());
    }

    output::print_summary(&selection, &registry);
    crate::scaffold::scaffold(&selection, &registry, &root)?;
    output::print_success(&selection);

    Ok(())
}
