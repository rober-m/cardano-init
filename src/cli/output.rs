use console::style;

use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection};
use crate::scaffold::planner::FilePlan;

/// Print the welcome banner for interactive mode.
pub fn print_welcome() {
    println!();
    println!(
        "  {} Let's set up your Cardano protocol project.",
        style("Welcome to cardano-init!").bold()
    );
    println!();
    println!("  A Cardano protocol typically has up to four components:");
    println!(
        "  {} Smart contract logic (validators) that runs on the ledger",
        style("On-chain:").cyan().bold()
    );
    println!(
        "  {} Code that builds and submits transactions",
        style("Off-chain:").cyan().bold()
    );
    println!(
        "  {} Indexers and services that read chain data",
        style("Infrastructure:").cyan().bold()
    );
    println!(
        "  {} Frameworks for testing your contracts locally",
        style("Testing:").cyan().bold()
    );
    println!();
}

/// Print a summary of the selection before generation.
pub fn print_summary(selection: &Selection, registry: &Registry) {
    println!();
    println!("  {}", style("Summary").bold().underlined());
    println!();
    println!("  Project:  {}", style(&selection.project_name).cyan());

    for assignment in &selection.assignments {
        let role_label = match assignment.role {
            Role::OnChain => "On-chain",
            Role::OffChain => "Off-chain",
            Role::Infrastructure => "Infra",
            Role::Testing => "Testing",
        };

        let tool_info = if let Some(tool) = registry.get(&assignment.tool_id) {
            let lang = tool.languages.first().map(|s| s.as_str()).unwrap_or("?");
            format!("{} ({})", tool.name, lang)
        } else {
            assignment.tool_id.clone()
        };

        println!(
            "  {:<12}{}",
            format!("{}:", role_label),
            style(tool_info).cyan()
        );
    }

    println!("  Network:  {}", style(&selection.network).cyan());

    if selection.nix {
        println!("  Nix:      {}", style("yes").green());
    }
    println!();
}

/// Print the dry-run output: summary + nested file tree.
pub fn print_dry_run(selection: &Selection, registry: &Registry, plan: &FilePlan) {
    print_summary(selection, registry);

    println!("  {}", style(format!("{}/", selection.project_name)).bold());

    let paths: Vec<Vec<&str>> = plan
        .entries
        .iter()
        .map(|e| {
            e.dest
                .to_str()
                .expect("paths are UTF-8")
                .split('/')
                .collect()
        })
        .collect();

    print_tree(&paths, 0, 0, &mut String::new());

    println!();
    println!(
        "  {} files would be generated.",
        style(plan.entries.len()).bold()
    );
    println!();
}

/// Recursively print a directory tree from a sorted list of split paths.
///
/// `paths` contains only entries whose prefix (components 0..depth) matches
/// the current branch. `depth` is the current tree level. `indent` is the
/// prefix string built from the ancestors' box-drawing connectors.
fn print_tree(paths: &[Vec<&str>], depth: usize, _start: usize, indent: &mut String) {
    // Group entries by their component at `depth`.
    // Preserve insertion order so the tree follows the plan order.
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if depth >= path.len() {
            continue;
        }
        let key = path[depth];
        if let Some(group) = groups.iter_mut().find(|(k, _)| *k == key) {
            group.1.push(i);
        } else {
            groups.push((key, vec![i]));
        }
    }

    let total = groups.len();
    for (gi, (name, indices)) in groups.iter().enumerate() {
        let is_last = gi == total - 1;
        let connector = if is_last { "└── " } else { "├── " };

        // Check if this is a directory (has children deeper than depth+1)
        let is_dir = indices.iter().any(|&i| paths[i].len() > depth + 1);

        if is_dir {
            println!(
                "  {}{}{}",
                indent,
                style(connector).dim(),
                style(format!("{name}/")).dim()
            );
        } else {
            println!("  {}{}{}", indent, style(connector).dim(), name);
        }

        // Recurse into children that have more components
        let children: Vec<Vec<&str>> = indices
            .iter()
            .filter(|&&i| paths[i].len() > depth + 1)
            .map(|&i| paths[i].clone())
            .collect();

        if !children.is_empty() {
            let extension = if is_last { "    " } else { "│   " };
            let prev_len = indent.len();
            indent.push_str(extension);
            print_tree(&children, depth + 1, 0, indent);
            indent.truncate(prev_len);
        }
    }
}

/// Print success message after scaffolding.
pub fn print_success(selection: &Selection) {
    println!();
    println!(
        "  {} Created {}",
        style("✔").green().bold(),
        style(&selection.project_name).cyan().bold()
    );

    for assignment in &selection.assignments {
        let role_label = match assignment.role {
            Role::OnChain => "on-chain",
            Role::OffChain => "off-chain",
            Role::Infrastructure => "infrastructure",
            Role::Testing => "testing",
        };
        println!(
            "  {} Scaffolded {} ({})",
            style("✔").green().bold(),
            role_label,
            &assignment.tool_id
        );
    }

    println!();
    println!("  {}", style("Next steps:").bold());
    println!("    cd {}", selection.project_name);
    println!("    just build");
    println!();
}

/// Truncate a tool description to the first sentence for use in prompts.
pub fn first_sentence(desc: &str) -> &str {
    // Find the first period followed by whitespace or end-of-string
    if let Some(pos) = desc.find(". ") {
        &desc[..=pos]
    } else if let Some(pos) = desc.find(".\n") {
        &desc[..=pos]
    } else if desc.ends_with('.') {
        desc
    } else {
        // No sentence boundary — take first 80 chars
        let end = desc
            .char_indices()
            .nth(80)
            .map(|(i, _)| i)
            .unwrap_or(desc.len());
        &desc[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentence_with_period_space() {
        assert_eq!(
            first_sentence("Hello world. More text here."),
            "Hello world."
        );
    }

    #[test]
    fn first_sentence_no_period() {
        assert_eq!(first_sentence("No period here"), "No period here");
    }

    #[test]
    fn first_sentence_ends_with_period() {
        assert_eq!(first_sentence("Ends with period."), "Ends with period.");
    }
}
