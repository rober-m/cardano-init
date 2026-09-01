use std::collections::HashMap;
use std::path::PathBuf;

use minijinja::{AutoEscape, Environment, UndefinedBehavior};

use super::context::TemplateContext;
use super::planner::{FilePlan, TemplateSource};
use super::{ScaffoldError, TemplateAssets};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A file with its final content, ready to be written to disk.
#[derive(Debug)]
pub struct RenderedFile {
    /// Destination path relative to the project root.
    pub dest: PathBuf,
    /// The rendered (or pass-through) content.
    pub content: Vec<u8>,
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render all files in the plan, producing final content for each.
///
/// - Files with `render == true` are processed through MiniJinja.
/// - Files with `render == false` are passed through as-is.
/// - `Inline` sources use their embedded bytes directly.
pub fn render(
    plan: &FilePlan,
    context: &TemplateContext,
) -> Result<Vec<RenderedFile>, ScaffoldError> {
    // Pre-load all template sources into owned strings so MiniJinja can borrow them.
    let mut sources: HashMap<String, String> = HashMap::new();
    for entry in &plan.entries {
        if !entry.render {
            continue;
        }
        if let Some(key) = entry.source.asset_key() {
            if sources.contains_key(&key) {
                continue;
            }
            let data = TemplateAssets::get(&key)
                .ok_or_else(|| ScaffoldError::AssetNotFound { path: key.clone() })?;
            let text = std::str::from_utf8(&data.data)
                .expect("renderable template must be valid UTF-8")
                .to_string();
            sources.insert(key, text);
        }
    }

    // Build the MiniJinja environment with all templates loaded.
    //
    // Rendering contract (TECH_SPEC §4.3):
    // - strict undefined: a template referencing an undefined variable fails at
    //   generation time instead of silently emitting an empty string;
    // - autoescape off: output is code/config, never HTML;
    // - keep trailing newline: the authored file's final newline is preserved, so
    //   output bytes match the template source (determinism, §11).
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::Strict);
    env.set_auto_escape_callback(|_name| AutoEscape::None);
    env.set_keep_trailing_newline(true);
    for (key, text) in &sources {
        env.add_template(key, text)
            .map_err(|e| ScaffoldError::Render {
                path: key.clone(),
                source: e,
            })?;
    }

    let ctx_value = minijinja::value::Value::from_serialize(context);
    let mut rendered = Vec::with_capacity(plan.entries.len());

    for entry in &plan.entries {
        let content = match &entry.source {
            TemplateSource::Inline(bytes) => bytes.clone(),
            source => {
                let asset_key = source
                    .asset_key()
                    .expect("non-Inline source must have asset_key");

                if !entry.render {
                    let data = TemplateAssets::get(&asset_key).ok_or_else(|| {
                        ScaffoldError::AssetNotFound {
                            path: asset_key.clone(),
                        }
                    })?;
                    data.data.to_vec()
                } else {
                    let tmpl = env
                        .get_template(&asset_key)
                        .map_err(|e| ScaffoldError::Render {
                            path: asset_key.clone(),
                            source: e,
                        })?;
                    tmpl.render(&ctx_value)
                        .map_err(|e| ScaffoldError::Render {
                            path: asset_key,
                            source: e,
                        })?
                        .into_bytes()
                }
            }
        };

        rendered.push(RenderedFile {
            dest: entry.dest.clone(),
            content,
        });
    }

    Ok(rendered)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    use crate::registry::loader::Registry;
    use crate::registry::types::{Network, Role, RoleAssignment, Selection};
    use crate::scaffold::context::build_context;
    use crate::scaffold::planner;

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
    fn static_file_passes_through() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let validator = files
            .iter()
            .find(|f| f.dest.to_str().unwrap().contains("validators/giftcard.ak"))
            .expect("giftcard.ak should be in rendered files");

        let content = std::str::from_utf8(&validator.content).unwrap();
        assert!(content.contains("validator gift_card"));
        assert!(!content.contains("{{"));
    }

    #[test]
    fn jinja_template_renders_context() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let justfile = files
            .iter()
            .find(|f| f.dest == Path::new("Justfile"))
            .expect("Justfile should be in rendered files");

        let content = std::str::from_utf8(&justfile.content).unwrap();
        assert!(content.contains("test-project"));
        assert!(content.contains("build-on-chain"));
        assert!(!content.contains("{%"));
    }

    #[test]
    fn inline_source_produces_empty_content() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let gitkeep = files
            .iter()
            .find(|f| f.dest == Path::new("blueprint/.gitkeep"))
            .expect("blueprint/.gitkeep should be in rendered files");

        assert!(gitkeep.content.is_empty());
    }

    #[test]
    fn nix_flake_renders_with_packages() {
        let mut sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        sel.nix = true;
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let flake = files
            .iter()
            .find(|f| f.dest == Path::new("flake.nix"))
            .expect("flake.nix should be in rendered files");

        let content = std::str::from_utf8(&flake.content).unwrap();
        assert!(content.contains("aiken"));
        assert!(content.contains("test-project"));
        assert!(!content.contains("{%"));
    }

    // -----------------------------------------------------------------------
    // Snapshot tests (determinism guard, §11)
    //
    // Render a selection into one canonical string (each file in plan order,
    // prefixed by its dest) and compare against a committed fixture under
    // `src/scaffold/snapshots/`. Regenerate after an intentional change with:
    //
    //     UPDATE_SNAPSHOTS=1 cargo test
    //
    // and review the diff before committing.
    // -----------------------------------------------------------------------

    fn render_to_snapshot(sel: &Selection) -> String {
        let reg = registry();
        let plan = planner::plan(sel, &reg).unwrap();
        let ctx = build_context(sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let mut out = String::new();
        for file in &files {
            out.push_str("=== ");
            out.push_str(&file.dest.to_string_lossy());
            out.push_str(" ===\n");
            out.push_str(std::str::from_utf8(&file.content).expect("snapshot content is UTF-8"));
            out.push('\n');
        }
        out
    }

    fn assert_snapshot(name: &str, sel: &Selection) {
        let actual = render_to_snapshot(sel);
        let path = format!(
            "{}/src/scaffold/snapshots/{name}.snap",
            env!("CARGO_MANIFEST_DIR")
        );
        if std::env::var("UPDATE_SNAPSHOTS").is_ok() {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&path, &actual).unwrap();
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
            panic!("missing snapshot '{path}'. Run `UPDATE_SNAPSHOTS=1 cargo test` to create it.")
        });
        assert_eq!(actual, expected, "snapshot '{name}' drifted");
    }

    fn sel(assignments: Vec<RoleAssignment>, network: Network, nix: bool) -> Selection {
        Selection {
            project_name: "snap-project".to_string(),
            assignments,
            network,
            nix,
        }
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    #[test]
    fn snapshot_aiken_only() {
        assert_snapshot(
            "aiken_only",
            &sel(vec![a(Role::OnChain, "aiken")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_plinth_nix() {
        // Plinth on-chain with `--nix`: locks the component-local haskell.nix
        // flake (mirroring IntersectMBO/plinth-template) and confirms Plinth's
        // packages stay out of the aggregated top-level shell.
        assert_snapshot(
            "plinth_nix",
            &sel(vec![a(Role::OnChain, "plinth")], Network::Preview, true),
        );
    }

    #[test]
    fn snapshot_plinth_meshjs_nix() {
        // Plinth on-chain + MeshJS off-chain under `--nix`: the top-level flake
        // composes the on-chain haskell.nix shell via `inputsFrom` *and* still
        // lists MeshJS's plain nixpkgs package (nodejs) — the two coexist.
        assert_snapshot(
            "plinth_meshjs_nix",
            &sel(
                vec![a(Role::OnChain, "plinth"), a(Role::OffChain, "meshjs")],
                Network::Preview,
                true,
            ),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_nix() {
        assert_snapshot(
            "aiken_meshjs_nix",
            &sel(
                vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
                Network::Preprod,
                true,
            ),
        );
    }

    #[test]
    fn snapshot_offchain_only() {
        assert_snapshot(
            "offchain_only",
            &sel(vec![a(Role::OffChain, "meshjs")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_yaci_devnet_only() {
        assert_snapshot(
            "yaci_devnet_only",
            &sel(vec![a(Role::Devnet, "yaci")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_dingo_devnet_only() {
        assert_snapshot(
            "dingo_devnet_only",
            &sel(vec![a(Role::Devnet, "dingo")], Network::Preview, false),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_yaci() {
        // The headline integration: on-chain + off-chain + a local devnet in the
        // devnet role. Locks the connection wiring (MeshJS ↔ Yaci via .env).
        assert_snapshot(
            "aiken_meshjs_yaci",
            &sel(
                vec![
                    a(Role::OnChain, "aiken"),
                    a(Role::OffChain, "meshjs"),
                    a(Role::Devnet, "yaci"),
                ],
                Network::Preview,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_infra_kupo_ogmios() {
        // Aggregated infrastructure: two providers collapse into one infra/
        // component (the cardano-up driver). Infra-only ⇒ no blueprint dir.
        assert_snapshot(
            "infra_kupo_ogmios",
            &sel(
                vec![
                    a(Role::Infrastructure, "kupo"),
                    a(Role::Infrastructure, "ogmios"),
                ],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_infra_dolos() {
        // Dolos provides its own node socket (no cardano-node dependency), so it
        // OVERRIDES the base NODE_SOCKET_PATH default (DOLOS_SOCKET_PATH) and adds
        // DOLOS_GRPC_URL — locks the §5.4 explicit-over-default resolution.
        assert_snapshot(
            "infra_dolos",
            &sel(
                vec![a(Role::Infrastructure, "dolos")],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_aiken_meshjs_kupo_ogmios() {
        // Full stack: on-chain + off-chain + aggregated infra. Locks the .env
        // wiring (MeshJS reads INDEXER_URL/OGMIOS_URL written by the infra driver).
        assert_snapshot(
            "aiken_meshjs_kupo_ogmios",
            &sel(
                vec![
                    a(Role::OnChain, "aiken"),
                    a(Role::OffChain, "meshjs"),
                    a(Role::Infrastructure, "kupo"),
                    a(Role::Infrastructure, "ogmios"),
                ],
                Network::Preprod,
                false,
            ),
        );
    }

    #[test]
    fn snapshot_scalus_fullstack() {
        // Scalus fills both on-chain and off-chain and declares a [fullstack]
        // template, so the two collapse into a single `protocol/` component
        // (no separate on-chain/ or off-chain/). Locks the fullstack collapse.
        assert_snapshot(
            "scalus_fullstack",
            &sel(
                vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
                Network::Preview,
                false,
            ),
        );
    }

    #[test]
    fn rendered_output_has_no_crlf() {
        let s = sel(
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
            Network::Preview,
            true,
        );
        let reg = registry();
        let plan = planner::plan(&s, &reg).unwrap();
        let ctx = build_context(&s, &reg).unwrap();
        for file in render(&plan, &ctx).unwrap() {
            assert!(
                !file.content.contains(&b'\r'),
                "{:?} contains a CR byte",
                file.dest
            );
        }
    }

    #[test]
    fn rendered_justfile_keeps_trailing_newline() {
        let s = sel(vec![a(Role::OnChain, "aiken")], Network::Preview, false);
        let reg = registry();
        let plan = planner::plan(&s, &reg).unwrap();
        let ctx = build_context(&s, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();
        let justfile = files
            .iter()
            .find(|f| f.dest == Path::new("Justfile"))
            .unwrap();
        assert!(justfile.content.ends_with(b"\n"));
    }

    #[test]
    fn full_plan_renders_without_error() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
        ]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        assert_eq!(files.len(), plan.entries.len());

        for file in &files {
            if file.dest != Path::new("blueprint/.gitkeep") {
                assert!(
                    !file.content.is_empty(),
                    "file {:?} should have content",
                    file.dest
                );
            }
        }
    }
}
