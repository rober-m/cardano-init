pub mod git;
pub mod interactive;
pub mod oneshot;
pub mod output;
pub mod theme;
pub mod update;

use std::path::PathBuf;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

use crate::registry::loader::{Registry, RegistryError};
use crate::registry::types::{Role, ToolDef};
use crate::scaffold::ScaffoldError;

// ---------------------------------------------------------------------------
// CLI arguments
// ---------------------------------------------------------------------------

/// Output format. `json` implies non-interactive: it never prompts; if required
/// input is missing it errors instead (TECH_SPEC §2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Human,
    Json,
}

/// Scaffold a new Cardano protocol project.
#[derive(Parser, Debug)]
#[command(name = "cardano-init", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Output format (global): human-readable text or machine-readable JSON
    #[arg(long, value_enum, global = true, default_value = "human")]
    pub format: Format,

    #[command(flatten)]
    pub init: InitArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check that the dependencies this project needs are installed, and
    /// advise how to install any that are missing
    Doctor,

    /// List the available roles and tools (use --format json for agents)
    List(ListArgs),

    /// Add or swap a tool/role in the project in the current directory
    Add(AddArgs),

    /// Remove a role (or an infrastructure provider) from the project in the
    /// current directory
    Remove(RemoveArgs),
}

/// Arguments for `list`.
#[derive(clap::Args, Debug, Default)]
pub struct ListArgs {
    /// Show a compact matrix of tools by role (human output only; `--format
    /// json` is unaffected)
    #[arg(long)]
    pub table: bool,
}

/// Flags shared by the update commands (add / remove).
#[derive(clap::Args, Debug, Default, Clone)]
pub struct UpdateFlags {
    /// Show the change set without writing anything
    #[arg(long)]
    pub dry_run: bool,

    /// Update even when the git working tree has uncommitted changes (there is
    /// then no `git diff` to review/revert against)
    #[arg(long)]
    pub force: bool,

    /// Proceed even if the resulting off-chain ↔ provider selection is flagged
    /// incompatible (downgrades the stop to a warning)
    #[arg(long)]
    pub ignore_warning: bool,

    /// Allow experimental tools in the resulting selection
    #[arg(long)]
    pub allow_experimental: bool,
}

/// Arguments for `add`: the same role flags as init (presence assigns/replaces).
#[derive(clap::Args, Debug)]
pub struct AddArgs {
    /// On-chain tool (replaces the current on-chain tool if any)
    #[arg(long, value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (replaces the current off-chain tool if any)
    #[arg(long, value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Fullstack tool filling both on-chain and off-chain as one `protocol/`
    /// component. Not combinable with --on-chain/--off-chain.
    #[arg(long, value_name = "TOOL_ID")]
    pub fullstack: Option<String>,

    /// Infrastructure provider to add (repeatable)
    #[arg(long, value_name = "TOOL_ID")]
    pub infra: Vec<String>,

    /// Devnet tool (replaces the current devnet tool if any)
    #[arg(long, value_name = "TOOL_ID")]
    pub devnet: Option<String>,

    /// Formal-methods tool (replaces the current one if any)
    #[arg(long, value_name = "TOOL_ID")]
    pub formal_methods: Option<String>,

    #[command(flatten)]
    pub flags: UpdateFlags,
}

/// Arguments for `remove`: bare role flags drop that role; `--infra <id>` drops
/// one provider.
#[derive(clap::Args, Debug)]
pub struct RemoveArgs {
    /// Remove the on-chain component
    #[arg(long)]
    pub on_chain: bool,

    /// Remove the off-chain component
    #[arg(long)]
    pub off_chain: bool,

    /// Remove an infrastructure provider by id (repeatable)
    #[arg(long, value_name = "TOOL_ID")]
    pub infra: Vec<String>,

    /// Remove the devnet component
    #[arg(long)]
    pub devnet: bool,

    /// Remove the formal-methods component
    #[arg(long)]
    pub formal_methods: bool,

    #[command(flatten)]
    pub flags: UpdateFlags,
}

/// Arguments for the default init mode (interactive or one-shot).
#[derive(clap::Args, Debug)]
pub struct InitArgs {
    /// Project name (required in one-shot mode)
    #[arg(long)]
    pub name: Option<String>,

    /// On-chain tool (e.g., aiken, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (e.g., meshjs, scalus)
    #[arg(long, value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Fullstack tool for both on-chain and off-chain, as one `protocol/`
    /// Not combinable with --on-chain/--off-chain
    #[arg(long, value_name = "TOOL_ID")]
    pub fullstack: Option<String>,

    /// Infrastructure tool (repeatable: --infra kupo --infra ogmios)
    #[arg(long, value_name = "TOOL_ID")]
    pub infra: Vec<String>,

    /// Devnet tool (e.g., yaci)
    #[arg(long, value_name = "TOOL_ID")]
    pub devnet: Option<String>,

    /// Formal methods tool (e.g., blaster)
    #[arg(long, value_name = "TOOL_ID")]
    pub formal_methods: Option<String>,

    /// Generate Nix flake for dependency management
    #[arg(long)]
    pub nix: bool,

    /// Opt in to experimental tools
    #[arg(long)]
    pub allow_experimental: bool,

    /// Show what would be generated without writing to disk
    #[arg(long)]
    pub dry_run: bool,

    /// Scaffold a combination the compatibility check flags as incompatible
    #[arg(long)]
    pub ignore_warning: bool,
}

impl InitArgs {
    /// Returns true if any one-shot flags were provided.
    fn has_oneshot_flags(&self) -> bool {
        self.on_chain.is_some()
            || self.off_chain.is_some()
            || self.fullstack.is_some()
            || !self.infra.is_empty()
            || self.devnet.is_some()
            || self.formal_methods.is_some()
            || self.nix
            || self.dry_run
    }
}

// ---------------------------------------------------------------------------
// CLI errors
// ---------------------------------------------------------------------------

// `miette::Diagnostic` supplies the *human* rendering (framed title + `help:`
// remedy + code, via the fancy handler installed in `run`). The inherent
// `code()`/`context()`/`exit_code()` methods below remain the source of truth
// for the JSON contract and exit dispatch — the `code(...)` slugs here mirror
// them and must stay in sync.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
pub enum CliError {
    #[error("{0}")]
    #[diagnostic(code(cardano_init::registry_load))]
    Registry(#[from] RegistryError),

    #[error("{0}")]
    #[diagnostic(code(cardano_init::scaffold_error))]
    Scaffold(#[from] ScaffoldError),

    #[error("{0}")]
    #[diagnostic(code(cardano_init::registry_load))]
    Catalog(#[from] crate::doctor::catalog::CatalogError),

    #[error("directory '{}' already exists. Refusing to overwrite", path)]
    #[diagnostic(
        code(cardano_init::dir_exists),
        help("choose a different --name, or remove/empty the existing directory.")
    )]
    DirectoryExists { path: String },

    #[error("no tool named '{tool_id}' fills the {role} role")]
    #[diagnostic(
        code(cardano_init::unknown_tool),
        help("valid {role} tools: {}", valid_tools.join(", ")),
        url("https://github.com/input-output-hk/cardano-init#tools")
    )]
    UnknownTool {
        tool_id: String,
        role: String,
        valid_tools: Vec<String>,
    },

    #[error("tool '{tool_id}' does not support the '{role}' role")]
    #[diagnostic(
        code(cardano_init::tool_role_mismatch),
        help("'{tool_id}' fills: {}", valid_roles.join(", ")),
        url("https://github.com/input-output-hk/cardano-init#tools")
    )]
    ToolRoleMismatch {
        tool_id: String,
        role: String,
        valid_roles: Vec<String>,
    },

    #[error("no roles selected")]
    #[diagnostic(
        code(cardano_init::no_roles_selected),
        help("select at least one tool, or pass a one-shot flag like --on-chain aiken.")
    )]
    NoRolesSelected,

    #[error(
        "{} is experimental — it may be unstable or incomplete, so it's opt-in (expect rough edges and breaking changes)",
        labels.join(", ")
    )]
    #[diagnostic(
        code(cardano_init::experimental_not_allowed),
        help(
            "to scaffold it anyway, re-run with --allow-experimental:\n\n    cardano-init --name <project> {} --allow-experimental",
            example_flags.join(" ")
        )
    )]
    ExperimentalNotAllowed {
        /// Tool ids (machine-facing `context`).
        tools: Vec<String>,
        /// Human labels, `Name (id)`, for the message.
        labels: Vec<String>,
        /// The role flags that selected the experimental tool(s), e.g.
        /// `["--formal-methods", "blaster"]`, so the example is copy-pasteable.
        example_flags: Vec<String>,
    },

    #[error("tool '{tool_id}' does not support fullstack (no [fullstack] template)")]
    #[diagnostic(
        code(cardano_init::fullstack_unsupported),
        help("fullstack-capable tools: {}", valid_tools.join(", "))
    )]
    FullstackUnsupported {
        tool_id: String,
        valid_tools: Vec<String>,
    },

    #[error("--fullstack cannot be combined with --on-chain or --off-chain")]
    #[diagnostic(
        code(cardano_init::fullstack_conflict),
        help("--fullstack X already fills both roles; use it alone.")
    )]
    FullstackConflict,

    // Boxed to keep `CliError` small (this payload carries several strings).
    // `transparent` delegates both Display and the diagnostic (code + `#[help]`)
    // to the inner error.
    #[error(transparent)]
    #[diagnostic(transparent)]
    IncompatibleTools(Box<IncompatibleToolsError>),

    #[error("invalid project name '{name}' — {reason}")]
    #[diagnostic(code(cardano_init::invalid_project_name))]
    InvalidProjectName { name: String, reason: String },

    #[error("--name is required when using one-shot flags (--on-chain, --off-chain, etc.)")]
    #[diagnostic(
        code(cardano_init::name_required),
        help(
            "run without flags for interactive mode, or provide --name:\n\n    cardano-init --name my-protocol --on-chain aiken"
        )
    )]
    NameRequired,

    #[error(
        "this directory doesn't look like a cardano-init project — unrecognized: {}",
        .dirs.join(", ")
    )]
    #[diagnostic(
        code(cardano_init::project_unrecognized),
        help(
            "run add/remove inside a generated project; fix or rename any renamed/foreign component directories first."
        )
    )]
    ProjectUnrecognized { dirs: Vec<String> },

    #[error("can't add the {role} component: '{dir}/' already exists and isn't recognized")]
    #[diagnostic(
        code(cardano_init::slot_occupied),
        help("remove or rename '{dir}/' first, then re-run.")
    )]
    SlotOccupied { role: String, dir: String },

    #[error("the working tree at '{path}' has uncommitted changes")]
    #[diagnostic(
        code(cardano_init::worktree_dirty),
        help(
            "commit or stash first so the update is reviewable with `git diff`, or re-run with --force to update anyway."
        )
    )]
    WorktreeDirty { path: String },

    #[error("nothing to change — the project already matches this selection")]
    #[diagnostic(
        code(cardano_init::nothing_to_change),
        help(
            "pass a different tool/role, or use `cardano-init add`/`remove` to change the stack."
        )
    )]
    NothingToChange,

    #[error("user aborted")]
    #[diagnostic(code(cardano_init::aborted))]
    Aborted,

    #[error("prompt error: {0}")]
    #[diagnostic(code(cardano_init::prompt_error))]
    Prompt(#[from] dialoguer::Error),
}

/// Payload for [`CliError::IncompatibleTools`] — boxed in the variant so
/// `CliError` stays small (an off-chain ↔ provider mismatch carries several
/// strings for both the human message and the JSON context).
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{off_chain} can't be paired with {providers} — {reason}")]
#[diagnostic(code(cardano_init::incompatible_tools))]
pub struct IncompatibleToolsError {
    /// Human label `Name (id)` for the off-chain tool.
    pub off_chain: String,
    /// Human labels `Name (id)` for the offending provider(s), comma-joined.
    pub providers: String,
    /// Why the pair can't talk (seam mismatch / self-hosting).
    pub reason: String,
    /// Pre-formatted multi-line remedy (which providers / off-chain tools fit,
    /// plus the `--ignore-warning` escape hatch); surfaced as the `help:` line.
    #[help]
    pub help: String,
    /// Machine-facing ids `[off_chain_id, provider_ids…]` (JSON context).
    pub ids: Vec<String>,
    /// Provider ids compatible with the chosen off-chain tool (JSON context).
    pub compatible_providers: Vec<String>,
    /// Off-chain ids compatible with the chosen providers (JSON context).
    pub compatible_off_chain: Vec<String>,
}

impl CliError {
    /// Process exit-code category (TECH_SPEC §2.3):
    /// - `2` — usage / validation errors (bad or missing input);
    /// - `1` — runtime errors (I/O, registry/render failure, …);
    /// - `0` — interactive abort by user choice (not an error).
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::UnknownTool { .. }
            | CliError::ToolRoleMismatch { .. }
            | CliError::NoRolesSelected
            | CliError::ExperimentalNotAllowed { .. }
            | CliError::FullstackUnsupported { .. }
            | CliError::FullstackConflict
            | CliError::IncompatibleTools(_)
            | CliError::InvalidProjectName { .. }
            | CliError::NameRequired
            | CliError::ProjectUnrecognized { .. }
            | CliError::SlotOccupied { .. }
            | CliError::NothingToChange => 2,

            CliError::Aborted => 0,

            CliError::Registry(_)
            | CliError::Scaffold(_)
            | CliError::Catalog(_)
            | CliError::DirectoryExists { .. }
            | CliError::WorktreeDirty { .. }
            | CliError::Prompt(_) => 1,
        }
    }

    /// Stable, machine-readable error code (TECH_SPEC §2.5). Part of the JSON
    /// contract; never changes for a given error kind.
    pub fn code(&self) -> &'static str {
        match self {
            CliError::Registry(_) | CliError::Catalog(_) => "registry_load",
            CliError::Scaffold(_) => "scaffold_error",
            CliError::DirectoryExists { .. } => "dir_exists",
            CliError::UnknownTool { .. } => "unknown_tool",
            CliError::ToolRoleMismatch { .. } => "tool_role_mismatch",
            CliError::NoRolesSelected => "no_roles_selected",
            CliError::ExperimentalNotAllowed { .. } => "experimental_not_allowed",
            CliError::FullstackUnsupported { .. } => "fullstack_unsupported",
            CliError::FullstackConflict => "fullstack_conflict",
            CliError::IncompatibleTools(_) => "incompatible_tools",
            CliError::InvalidProjectName { .. } => "invalid_project_name",
            CliError::NameRequired => "name_required",
            CliError::ProjectUnrecognized { .. } => "project_unrecognized",
            CliError::SlotOccupied { .. } => "slot_occupied",
            CliError::WorktreeDirty { .. } => "worktree_dirty",
            CliError::NothingToChange => "nothing_to_change",
            CliError::Aborted => "aborted",
            CliError::Prompt(_) => "prompt_error",
        }
    }

    /// Structured, agent-facing context: the offending input plus valid
    /// alternatives where applicable (TECH_SPEC §2.5, PRD FR-15).
    pub fn context(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            CliError::Registry(e) => json!({ "detail": e.to_string() }),
            CliError::Catalog(e) => json!({ "detail": e.to_string() }),
            CliError::Scaffold(e) => json!({ "detail": e.to_string() }),
            CliError::DirectoryExists { path } => json!({ "path": path }),
            CliError::UnknownTool {
                tool_id,
                role,
                valid_tools,
            } => json!({ "tool_id": tool_id, "role": role, "valid_tools": valid_tools }),
            CliError::ToolRoleMismatch {
                tool_id,
                role,
                valid_roles,
            } => json!({ "tool_id": tool_id, "role": role, "valid_roles": valid_roles }),
            CliError::FullstackUnsupported {
                tool_id,
                valid_tools,
            } => json!({ "tool_id": tool_id, "valid_tools": valid_tools }),
            CliError::InvalidProjectName { name, reason } => {
                json!({ "name": name, "reason": reason })
            }
            CliError::ExperimentalNotAllowed { tools, .. } => {
                json!({ "tools": tools, "remedy": "--allow-experimental" })
            }
            CliError::IncompatibleTools(e) => json!({
                "tools": e.ids,
                "reason": e.reason,
                "compatible_providers": e.compatible_providers,
                "compatible_off_chain": e.compatible_off_chain,
                "remedy": "--ignore-warning",
            }),
            CliError::ProjectUnrecognized { dirs } => json!({ "dirs": dirs }),
            CliError::SlotOccupied { role, dir } => json!({ "role": role, "dir": dir }),
            CliError::WorktreeDirty { path } => json!({ "path": path }),
            CliError::NoRolesSelected
            | CliError::FullstackConflict
            | CliError::NameRequired
            | CliError::NothingToChange
            | CliError::Aborted
            | CliError::Prompt(_) => json!({}),
        }
    }
}

// ---------------------------------------------------------------------------
// Tool catalog for --help
// ---------------------------------------------------------------------------

/// Build the section appended to --help: a pointer to `list` for the tool
/// catalog (kept out of --help to avoid a wall of text), plus usage examples.
fn build_help_footer() -> String {
    use std::fmt::Write;

    let mut out =
        String::from("Run 'cardano-init list' to see the tools available for each role.\n\n");

    let _ = writeln!(out, "Examples:");
    let _ = writeln!(
        out,
        "  cardano-init                                        # interactive mode"
    );
    let _ = writeln!(
        out,
        "  cardano-init --name my-app --on-chain aiken         # one-shot, single role"
    );
    let _ = writeln!(
        out,
        "  cardano-init --name my-app --on-chain aiken --off-chain meshjs --nix"
    );

    out
}

pub(super) fn format_tool(out: &mut String, tool: &ToolDef) {
    use std::fmt::Write;

    let mut roles: Vec<&str> = tool.roles.keys().map(|r| r.as_kebab()).collect();
    roles.sort();
    let experimental_tag = if tool.experimental {
        "  [experimental]"
    } else {
        ""
    };
    let _ = writeln!(out, "  {} ({}){}", tool.name, tool.id, experimental_tag);
    if tool.experimental {
        let _ = writeln!(
            out,
            "    Status:    experimental — may be unstable or incomplete; needs --allow-experimental"
        );
    }
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

/// Main CLI entry point. Parse args, dispatch, and present the result (or a
/// machine-readable error). Returns the process exit code.
pub fn run() -> i32 {
    let registry = match Registry::load() {
        Ok(r) => r,
        Err(e) => {
            // Registry load happens before we know the requested format; this
            // is a packaging bug (embedded data), so default to human output.
            let err = CliError::from(e);
            let code = err.exit_code();
            output::print_error(err, Format::Human);
            return code;
        }
    };

    // Append usage examples + a pointer to `list` (the tool catalog lives there,
    // not in --help).
    let cmd = Cli::command().after_help(build_help_footer());
    let matches = cmd.get_matches();
    let cli = Cli::from_arg_matches(&matches).expect("clap already validated");
    let format = cli.format;

    let result = match cli.command {
        Some(Command::Doctor) => run_doctor(&registry, format),
        Some(Command::List(args)) => {
            output::print_list(&registry, format, args.table);
            Ok(())
        }
        Some(Command::Add(args)) => update::run_add(args, &registry, format),
        Some(Command::Remove(args)) => update::run_remove(args, &registry, format),
        None => run_init(cli.init, &registry, format),
    };

    match result {
        Ok(()) => 0,
        Err(e) => {
            let code = e.exit_code();
            output::print_error(e, format);
            code
        }
    }
}

fn required_deps_for_assignments<'a>(
    assignments: impl Iterator<Item = (&'a str, crate::registry::types::Role)>,
    registry: &Registry,
) -> Vec<String> {
    let mut deps = vec![crate::doctor::BASE_DEP.to_string()];
    for (id, role) in assignments {
        if id == crate::doctor::INFRA_DRIVER_ID {
            for tool in registry.tools_for_role(crate::registry::types::Role::Infrastructure) {
                deps.extend(tool.system_deps.iter().cloned());
            }
        } else if let Some(tool) = registry.get(id) {
            let role_deps = tool
                .roles
                .get(&role)
                .and_then(|config| config.system_deps.as_ref())
                .unwrap_or(&tool.system_deps);
            deps.extend(role_deps.iter().cloned());
        }
    }
    deps
}

fn required_deps_for_components(
    components: &[crate::doctor::probe::DetectedComponent],
    registry: &Registry,
) -> Vec<String> {
    let mut deps = required_deps_for_assignments(
        components
            .iter()
            .filter_map(|component| match component.kind {
                crate::doctor::probe::ComponentKind::Role(role) => {
                    Some((component.tool_id.as_str(), role))
                }
                crate::doctor::probe::ComponentKind::Protocol => None,
            }),
        registry,
    );
    for component in components {
        if matches!(
            component.kind,
            crate::doctor::probe::ComponentKind::Protocol
        ) && let Some(tool) = registry.get(&component.tool_id)
        {
            deps.extend(tool.system_deps.iter().cloned());
        }
    }
    deps
}

/// Turn a detected [`Incompatibility`](crate::registry::compat::Incompatibility)
/// into a stop-generation `CliError`, pre-formatting the "which tools do fit"
/// remedy (with the self-hosting case — no compatible provider — phrased as
/// "omit the provider").
fn incompatible_tools_error(inc: crate::registry::compat::Incompatibility) -> CliError {
    let provider_labels: Vec<String> = inc
        .providers
        .iter()
        .map(|p| format!("{} ({})", p.name, p.id))
        .collect();
    let providers_text = provider_labels.join(", ");

    // These lines feed the diagnostic's `help:` field; miette owns the
    // indentation, so they carry none of their own.
    let providers_line = if !inc.compatible_providers.is_empty() {
        format!(
            "Providers that work with {}: {}",
            inc.off_chain_name,
            inc.compatible_providers.join(", ")
        )
    } else if inc.self_hosted {
        format!(
            "{} provides its own devnet — omit the provider selection.",
            inc.off_chain_name
        )
    } else {
        format!(
            "No bundled provider serves {} — point it at a public provider (see its README).",
            inc.off_chain_name
        )
    };
    let off_chain_line = if inc.compatible_off_chain.is_empty() {
        String::new()
    } else {
        format!(
            "\nOff-chain tools that work with {}: {}",
            providers_text,
            inc.compatible_off_chain.join(", ")
        )
    };

    let mut ids = vec![inc.off_chain_id.clone()];
    ids.extend(inc.providers.iter().map(|p| p.id.clone()));

    CliError::IncompatibleTools(Box::new(IncompatibleToolsError {
        off_chain: format!("{} ({})", inc.off_chain_name, inc.off_chain_id),
        providers: providers_text,
        reason: inc.reason,
        help: format!(
            "{providers_line}{off_chain_line}\n\nTo scaffold this combination anyway, re-run with --ignore-warning."
        ),
        ids,
        compatible_providers: inc.compatible_providers,
        compatible_off_chain: inc.compatible_off_chain,
    }))
}

/// Run `work` while showing a transient spinner labelled `message`, clearing it
/// before returning. The spinner is shown only for **human, attended** (TTY)
/// output — under `--format json` or when piped it would corrupt the stream, so
/// `work` runs plainly. indicatif draws to stderr, keeping stdout clean either
/// way. Scoped to genuinely latency-bearing work (system probing / dep
/// resolution), not near-instant steps (clig.dev: no fake progress).
fn with_spinner<T>(message: &str, format: Format, work: impl FnOnce() -> T) -> T {
    use indicatif::{ProgressBar, ProgressStyle};

    if format != Format::Human || !console::user_attended() {
        return work();
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .expect("static spinner template is valid"),
    );
    pb.set_message(message.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    let result = work();
    pb.finish_and_clear();
    result
}

/// Run the standalone `doctor`: scan the current directory for generated
/// components, then report dependency status + install plans.
fn run_doctor(registry: &Registry, format: Format) -> Result<(), CliError> {
    use crate::doctor::{self, catalog::DepCatalog, probe};

    let catalog = DepCatalog::load()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let scan = probe::scan_project(&cwd, registry);

    let required = required_deps_for_components(&scan.components, registry);
    let (env, report) = with_spinner("Probing environment…", format, || {
        let env = probe::detect_environment(&catalog);
        let report = doctor::resolve_all(&required, &catalog, &env);
        (env, report)
    });

    output::print_doctor(&scan, &report, &env, registry, format);
    Ok(())
}

/// Experimental gate: fail if `selection` includes a not-yet-build-green tool.
/// Shared by `init` (one-shot) and the update commands; the caller skips it when
/// `--allow-experimental` was given.
fn experimental_gate(
    selection: &crate::registry::types::Selection,
    registry: &Registry,
) -> Result<(), CliError> {
    let experimental = output::selected_experimental_tools(selection, registry);
    if experimental.is_empty() {
        return Ok(());
    }
    let tools: Vec<String> = experimental.iter().map(|t| t.id.clone()).collect();
    let labels: Vec<String> = experimental
        .iter()
        .map(|t| format!("{} ({})", t.name, t.id))
        .collect();
    // The exact role flags that pulled in the experimental tool(s), so the
    // suggested command is copy-pasteable.
    let example_flags: Vec<String> = selection
        .assignments
        .iter()
        .filter(|a| tools.iter().any(|id| id == &a.tool_id))
        .flat_map(|a| [role_flag(a.role).to_string(), a.tool_id.clone()])
        .collect();
    Err(CliError::ExperimentalNotAllowed {
        tools,
        labels,
        example_flags,
    })
}

/// The one-shot flag that assigns a tool to a given role, e.g. `--on-chain`.
/// Used to build a copy-pasteable example command in error messages.
fn role_flag(role: Role) -> &'static str {
    match role {
        Role::OnChain => "--on-chain",
        Role::OffChain => "--off-chain",
        Role::Infrastructure => "--infra",
        Role::Devnet => "--devnet",
        Role::FormalMethods => "--formal-methods",
    }
}

/// Run the default init mode (interactive or one-shot).
fn run_init(args: InitArgs, registry: &Registry, format: Format) -> Result<(), CliError> {
    // `--name` is required for one-shot flags, and always in JSON mode (which
    // is non-interactive and must never prompt — TECH_SPEC §2.1).
    if args.name.is_none() && (args.has_oneshot_flags() || format == Format::Json) {
        return Err(CliError::NameRequired);
    }

    // `--fullstack X` is sugar for `--on-chain X --off-chain X`; combining it with
    // an explicit on-chain/off-chain flag is ambiguous (TECH_SPEC §2.2).
    if args.fullstack.is_some() && (args.on_chain.is_some() || args.off_chain.is_some()) {
        return Err(CliError::FullstackConflict);
    }

    // Decide mode: one-shot if --name provided, interactive otherwise
    let selection = if let Some(ref name) = args.name {
        let selection = oneshot::build_selection(
            name,
            args.on_chain.as_deref(),
            args.off_chain.as_deref(),
            args.fullstack.as_deref(),
            &args.infra,
            args.devnet.as_deref(),
            args.formal_methods.as_deref(),
            args.nix,
            registry,
        )?;
        // Experimental gate (one-shot / JSON): selecting a not-yet-build-green
        // tool requires explicit opt-in. Non-interactive can't prompt, so this
        // is a hard usage error unless --allow-experimental was passed.
        if !args.allow_experimental {
            experimental_gate(&selection, registry)?;
        }
        selection
    } else {
        interactive::run_interactive(registry, args.allow_experimental, args.ignore_warning)?
    };

    // Off-chain ↔ devnet compatibility gate. Interactive mode already filters
    // incompatible options at selection time, so this primarily guards one-shot;
    // with --ignore-warning it degrades to a warning and proceeds anyway.
    if let Some(inc) = crate::registry::compat::check(&selection.assignments, registry) {
        if args.ignore_warning {
            if format == Format::Human {
                output::print_incompatibility_warning(&inc);
            }
        } else {
            return Err(incompatible_tools_error(inc));
        }
    }

    let root = PathBuf::from(&selection.project_name);

    // Safety: refuse to write into an existing, non-empty directory (never
    // overwrite user files). A missing or empty target dir is fine (§6.4).
    if root.exists()
        && std::fs::read_dir(&root)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(CliError::DirectoryExists {
            path: selection.project_name.clone(),
        });
    }

    if args.dry_run {
        let plan = crate::scaffold::dry_run(&selection, registry)?;
        output::print_dry_run(&selection, registry, &plan, format);
        return Ok(());
    }

    // No pre-generation summary here: interactive mode already showed the panel
    // at its confirm step, and the one-shot path goes straight to the success
    // panel (same rows + a CREATED badge), so a separate preview would just
    // duplicate it.
    with_spinner("Scaffolding project…", format, || {
        crate::scaffold::scaffold(&selection, registry, &root)
    })?;

    // Give the new project a git repo + initial commit, so `add`/`remove`
    // have a clean tree to diff against out of the box. Best-effort: skips when
    // git is absent or we're already inside a repo, never fails generation.
    let git = git::init_project_repo(&root);

    // Check-and-advise: resolve the deps this selection needs (TECH_SPEC §9).
    let report = with_spinner("Checking dependencies…", format, || {
        resolve_selection_deps(&selection, registry)
    })?;
    output::print_success(&selection, registry, &report, git, format);

    Ok(())
}

/// Resolve the dependency report for a generated selection (check-and-advise).
fn resolve_selection_deps(
    selection: &crate::registry::types::Selection,
    registry: &Registry,
) -> Result<crate::doctor::Report, CliError> {
    use crate::doctor::{self, catalog::DepCatalog, probe};

    let catalog = DepCatalog::load()?;
    let required = required_deps_for_assignments(
        selection
            .assignments
            .iter()
            .map(|a| (a.tool_id.as_str(), a.role)),
        registry,
    );
    let env = probe::detect_environment(&catalog);
    Ok(doctor::resolve_all(&required, &catalog, &env))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_tool_error_code_and_context() {
        let registry = Registry::load().unwrap();
        // `bogus` is not a tool; one-shot validation should surface it with the
        // stable code + the valid alternatives for the role.
        let err = oneshot::build_selection(
            "demo",
            Some("bogus"),
            None,
            None,
            &[],
            None,
            None,
            false,
            &registry,
        )
        .unwrap_err();

        assert_eq!(err.code(), "unknown_tool");
        let ctx = err.context();
        assert_eq!(ctx["tool_id"], "bogus");
        assert_eq!(ctx["role"], "On-chain");
        // on-chain is fillable by aiken + plinth + scalus.
        let valid: Vec<&str> = ctx["valid_tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(valid, vec!["aiken", "plinth", "scalus"]);
    }

    #[test]
    fn tool_role_mismatch_lists_valid_roles() {
        let registry = Registry::load().unwrap();
        // aiken is on-chain only; asking it to fill off-chain is a mismatch.
        let err = oneshot::build_selection(
            "demo",
            None,
            Some("aiken"),
            None,
            &[],
            None,
            None,
            false,
            &registry,
        )
        .unwrap_err();
        assert_eq!(err.code(), "tool_role_mismatch");
        let ctx = err.context();
        assert_eq!(ctx["valid_roles"], serde_json::json!(["on-chain"]));
    }

    #[test]
    fn fullstack_conflict_with_explicit_role_flag() {
        // --fullstack combined with --on-chain is a usage error; caught in
        // run_init before any generation happens.
        let registry = Registry::load().unwrap();
        let args = InitArgs {
            name: Some("demo".to_string()),
            on_chain: Some("aiken".to_string()),
            off_chain: None,
            fullstack: Some("scalus".to_string()),
            infra: vec![],
            devnet: None,
            formal_methods: None,
            nix: false,
            allow_experimental: false,
            dry_run: true,
            ignore_warning: false,
        };
        let err = run_init(args, &registry, Format::Json).unwrap_err();
        assert_eq!(err.code(), "fullstack_conflict");
        assert_eq!(err.exit_code(), 2);
    }

    /// Build InitArgs for a one-shot run with just the fields a test varies.
    fn init_args(formal_methods: Option<&str>, allow_experimental: bool) -> InitArgs {
        InitArgs {
            name: Some("exp-gate-demo".to_string()),
            on_chain: Some("aiken".to_string()),
            off_chain: None,
            fullstack: None,
            infra: vec![],
            devnet: None,
            formal_methods: formal_methods.map(str::to_string),
            nix: false,
            allow_experimental,
            dry_run: true,
            ignore_warning: false,
        }
    }

    #[test]
    fn experimental_tool_gated_without_flag() {
        // One-shot selecting blaster (experimental) without --allow-experimental
        // is a usage error before any generation happens.
        let registry = Registry::load().unwrap();
        let err = run_init(init_args(Some("blaster"), false), &registry, Format::Json).unwrap_err();
        assert_eq!(err.code(), "experimental_not_allowed");
        assert_eq!(err.exit_code(), 2);
        let ctx = err.context();
        let tools: Vec<&str> = ctx["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(tools, vec!["blaster"]);
        assert_eq!(ctx["remedy"], "--allow-experimental");

        // The human message names the tool (name + id); the copy-pasteable opt-in
        // command (with the exact role flag) is the diagnostic's `help:` line.
        let msg = err.to_string();
        assert!(msg.contains("Blaster (blaster)"), "message: {msg}");
        let help = miette::Diagnostic::help(&err)
            .map(|h| h.to_string())
            .unwrap_or_default();
        assert!(
            help.contains("--formal-methods blaster --allow-experimental"),
            "help: {help}"
        );
    }

    #[test]
    fn diagnostic_code_mirrors_inherent_code() {
        // The miette `#[diagnostic(code(...))]` slug (drives the human display)
        // must stay in sync with the inherent `code()` (the JSON contract): the
        // derive slug is the inherent slug under the `cardano_init::` namespace.
        let err = CliError::NameRequired;
        let diag = miette::Diagnostic::code(&err)
            .map(|c| c.to_string())
            .expect("variant declares a diagnostic code");
        assert_eq!(diag, format!("cardano_init::{}", err.code()));
    }

    #[test]
    fn experimental_tool_allowed_with_flag() {
        // With --allow-experimental the same selection passes the gate (dry-run
        // → Ok, no disk write).
        let registry = Registry::load().unwrap();
        let res = run_init(init_args(Some("blaster"), true), &registry, Format::Json);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    #[test]
    fn non_experimental_selection_needs_no_flag() {
        // A selection with no experimental tool is unaffected by the gate.
        let registry = Registry::load().unwrap();
        let res = run_init(init_args(None, false), &registry, Format::Json);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    /// InitArgs for a one-shot tx3 + devnet run (tx3 is experimental, so
    /// allow_experimental is set); `ignore_warning` toggles the compat gate.
    fn tx3_devnet_args(devnet: &str, ignore_warning: bool) -> InitArgs {
        InitArgs {
            name: Some("compat-demo".to_string()),
            on_chain: None,
            off_chain: Some("tx3".to_string()),
            fullstack: None,
            infra: vec![],
            devnet: Some(devnet.to_string()),
            formal_methods: None,
            nix: false,
            allow_experimental: true,
            dry_run: true,
            ignore_warning,
        }
    }

    #[test]
    fn incompatible_offchain_devnet_is_gated() {
        // tx3 (TRP, self-hosted devnet) + yaci (Blockfrost) don't share a seam,
        // so generation stops before any disk write.
        let registry = Registry::load().unwrap();
        let err = run_init(tx3_devnet_args("yaci", false), &registry, Format::Json).unwrap_err();
        assert_eq!(err.code(), "incompatible_tools");
        assert_eq!(err.exit_code(), 2);
        let ctx = err.context();
        assert_eq!(ctx["remedy"], "--ignore-warning");
        let tools: Vec<&str> = ctx["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(tools, vec!["tx3", "yaci"]);
    }

    #[test]
    fn incompatible_offchain_devnet_allowed_with_ignore_warning() {
        // --ignore-warning downgrades the stop to a warning and proceeds.
        let registry = Registry::load().unwrap();
        let res = run_init(tx3_devnet_args("yaci", true), &registry, Format::Json);
        assert!(res.is_ok(), "expected Ok, got {res:?}");
    }

    #[test]
    fn required_deps_unions_just_with_tool_deps() {
        let registry = Registry::load().unwrap();
        // aiken → ["aiken"], meshjs → ["node"]; plus the base dep "just".
        let deps = required_deps_for_assignments(
            [
                ("aiken", crate::registry::types::Role::OnChain),
                ("meshjs", crate::registry::types::Role::OffChain),
            ]
            .into_iter(),
            &registry,
        );
        assert!(deps.contains(&"just".to_string()));
        assert!(deps.contains(&"aiken".to_string()));
        assert!(deps.contains(&"node".to_string()));
    }

    #[test]
    fn required_deps_resolves_infra_driver_to_union() {
        let registry = Registry::load().unwrap();
        // The aggregated infra component (reported under INFRA_DRIVER_ID by the
        // scan) contributes the union of infra tools' system_deps.
        let deps = required_deps_for_assignments(
            [(
                crate::doctor::INFRA_DRIVER_ID,
                crate::registry::types::Role::Infrastructure,
            )]
            .into_iter(),
            &registry,
        );
        assert!(deps.contains(&"just".to_string()));
        assert!(deps.contains(&"docker".to_string()));
        assert!(deps.contains(&"cardano-up".to_string()));
    }

    #[test]
    fn required_deps_uses_role_specific_deps() {
        let registry = Registry::load().unwrap();
        let deps = required_deps_for_assignments(
            [("dingo", crate::registry::types::Role::Devnet)].into_iter(),
            &registry,
        );
        assert!(deps.contains(&"just".to_string()));
        assert!(deps.contains(&"docker".to_string()));
        assert!(!deps.contains(&"cardano-up".to_string()));
    }
}
