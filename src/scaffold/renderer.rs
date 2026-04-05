use std::collections::HashMap;
use std::path::PathBuf;

use minijinja::Environment;

use super::planner::{FilePlan, TemplateSource};
use super::context::TemplateContext;
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
pub fn render(plan: &FilePlan, context: &TemplateContext) -> Result<Vec<RenderedFile>, ScaffoldError> {
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
            let data = TemplateAssets::get(&key).ok_or_else(|| {
                ScaffoldError::AssetNotFound { path: key.clone() }
            })?;
            let text = std::str::from_utf8(&data.data)
                .expect("renderable template must be valid UTF-8")
                .to_string();
            sources.insert(key, text);
        }
    }

    // Build the MiniJinja environment with all templates loaded.
    let mut env = Environment::new();
    for (key, text) in &sources {
        env.add_template(key, text).map_err(|e| {
            ScaffoldError::Render {
                path: key.clone(),
                source: e,
            }
        })?;
    }

    let ctx_value = minijinja::value::Value::from_serialize(context);
    let mut rendered = Vec::with_capacity(plan.entries.len());

    for entry in &plan.entries {
        let content = match &entry.source {
            TemplateSource::Inline(bytes) => bytes.clone(),
            source => {
                let asset_key = source.asset_key().expect("non-Inline source must have asset_key");

                if !entry.render {
                    let data = TemplateAssets::get(&asset_key).ok_or_else(|| {
                        ScaffoldError::AssetNotFound { path: asset_key.clone() }
                    })?;
                    data.data.to_vec()
                } else {
                    let tmpl = env.get_template(&asset_key).map_err(|e| {
                        ScaffoldError::Render {
                            path: asset_key.clone(),
                            source: e,
                        }
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
            docker: false,
        }
    }

    #[test]
    fn static_file_passes_through() {
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
        ]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let example = files.iter().find(|f| {
            f.dest.to_str().unwrap().contains("validators/example.ak")
        }).expect("example.ak should be in rendered files");

        let content = std::str::from_utf8(&example.content).unwrap();
        assert!(content.contains("validator example"));
        assert!(!content.contains("{{"));
    }

    #[test]
    fn jinja_template_renders_context() {
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
        ]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let justfile = files.iter().find(|f| {
            f.dest == PathBuf::from("Justfile")
        }).expect("Justfile should be in rendered files");

        let content = std::str::from_utf8(&justfile.content).unwrap();
        assert!(content.contains("test-project"));
        assert!(content.contains("build-on-chain"));
        assert!(!content.contains("{%"));
    }

    #[test]
    fn inline_source_produces_empty_content() {
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
        ]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        let gitkeep = files.iter().find(|f| {
            f.dest == PathBuf::from("blueprint/.gitkeep")
        }).expect("blueprint/.gitkeep should be in rendered files");

        assert!(gitkeep.content.is_empty());
    }

    #[test]
    fn full_plan_renders_without_error() {
        let sel = selection(vec![
            RoleAssignment { role: Role::OnChain, tool_id: "aiken".into() },
            RoleAssignment { role: Role::OffChain, tool_id: "meshjs".into() },
        ]);
        let reg = registry();
        let plan = planner::plan(&sel, &reg).unwrap();
        let ctx = build_context(&sel, &reg).unwrap();
        let files = render(&plan, &ctx).unwrap();

        assert_eq!(files.len(), plan.entries.len());

        for file in &files {
            if file.dest != PathBuf::from("blueprint/.gitkeep") {
                assert!(
                    !file.content.is_empty(),
                    "file {:?} should have content",
                    file.dest
                );
            }
        }
    }
}
