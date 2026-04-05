mod contract;
mod registry;
mod scaffold;

use registry::types::{Network, Role, RoleAssignment, Selection};

fn main() {
    let registry = registry::Registry::load().expect("failed to load registry");

    // Temporary demo: scaffold a project with Aiken + MeshJS
    let selection = Selection {
        project_name: "my-protocol".to_string(),
        assignments: vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
            RoleAssignment { role: Role::OffChain, tool_id: "meshjs".into() },
        ],
        network: Network::Preview,
        nix: false,
        docker: false,
    };

    // Show what would be generated (dry run)
    let plan = scaffold::dry_run(&selection, &registry).expect("planning failed");
    println!("Files to generate:");
    for entry in &plan.entries {
        println!("  {}", entry.dest.display());
    }

    // Actually scaffold into ./output/
    let output = std::path::Path::new("output");
    let root = output.join(&selection.project_name);
    scaffold::scaffold(&selection, &registry, &root).expect("scaffolding failed");
    println!("\nScaffolded project at: {}", root.display());
}
