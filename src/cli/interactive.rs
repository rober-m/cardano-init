use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use super::CliError;
use super::oneshot::validate_project_name;
use super::output;
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

/// Run the full interactive wizard, returning a validated Selection.
pub fn run_interactive(registry: &Registry) -> Result<Selection, CliError> {
    let theme = ColorfulTheme::default();

    // Step 1: Welcome
    output::print_welcome();

    // Step 2: Role selection
    let roles = select_roles(&theme)?;
    if roles.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    // Step 3: Tool selection per role
    let assignments = select_tools(&theme, &roles, registry)?;

    // Step 4: Options
    let project_name = prompt_project_name(&theme)?;
    let network = prompt_network(&theme)?;
    let nix = Confirm::with_theme(&theme)
        .with_prompt("Set up Nix for dependency management?")
        .default(false)
        .interact()?;
    let selection = Selection {
        project_name,
        assignments,
        network,
        nix,
    };

    // Step 5: Summary + confirmation
    output::print_summary(&selection, registry);

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt("Generate project?")
        .default(true)
        .interact()?;

    if !confirmed {
        return Err(CliError::Aborted);
    }

    Ok(selection)
}

fn select_roles(theme: &ColorfulTheme) -> Result<Vec<Role>, CliError> {
    let descriptions = [
        "On-chain        — Smart contract logic (validators) on the ledger",
        "Off-chain       — Code that builds and submits transactions",
        "Infrastructure  — Indexers and services that read chain data",
        "Testing         — Frameworks for testing contracts locally",
        "Formal methods  — Specification and automated verification tools",
    ];

    let selections = MultiSelect::with_theme(theme)
        .with_prompt("Which components do you need? (space to select, enter to confirm)")
        .items(&descriptions)
        .interact()?;

    Ok(selections.iter().map(|&i| Role::ALL[i]).collect())
}

fn select_tools(
    theme: &ColorfulTheme,
    roles: &[Role],
    registry: &Registry,
) -> Result<Vec<RoleAssignment>, CliError> {
    let mut assignments = Vec::new();

    for &role in roles {
        let tools = registry.tools_for_role(role);
        if tools.is_empty() {
            println!(
                "  {} No tools available for {} yet, skipping.",
                console::style("⚠").yellow(),
                role
            );
            continue;
        }

        let items: Vec<String> = tools
            .iter()
            .map(|t| {
                let desc = output::first_sentence(&t.description);
                format!("{} — {}", t.name, desc)
            })
            .collect();

        if role == Role::Infrastructure {
            let selections = MultiSelect::with_theme(theme)
                .with_prompt(format!("Choose tools for {} (space to select):", role))
                .items(&items)
                .interact()?;

            for &idx in &selections {
                assignments.push(RoleAssignment {
                    role,
                    tool_id: tools[idx].id.clone(),
                });
            }
        } else {
            let idx = Select::with_theme(theme)
                .with_prompt(format!("Choose a tool for {}:", role))
                .items(&items)
                .default(0)
                .interact()?;

            assignments.push(RoleAssignment {
                role,
                tool_id: tools[idx].id.clone(),
            });
        }
    }

    Ok(assignments)
}

fn prompt_project_name(theme: &ColorfulTheme) -> Result<String, CliError> {
    let name: String = Input::with_theme(theme)
        .with_prompt("Project name")
        .default("my-protocol".to_string())
        .validate_with(|input: &String| -> Result<(), String> {
            validate_project_name(input).map_err(|e| e.to_string())
        })
        .interact_text()?;
    Ok(name)
}

fn prompt_network(theme: &ColorfulTheme) -> Result<Network, CliError> {
    let items = ["preview", "preprod", "mainnet"];
    let idx = Select::with_theme(theme)
        .with_prompt("Target network")
        .items(&items)
        .default(0)
        .interact()?;
    Ok(Network::from_str(items[idx]).expect("hardcoded network values are valid"))
}
