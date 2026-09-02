use std::fs;
use std::path::Path;

use super::ScaffoldError;
use super::renderer::RenderedFile;
use super::update::{SlotOp, UpdatePlan};

/// Write all rendered files to disk under `root`.
///
/// Creates directories as needed. This is the only phase with side effects.
pub fn write(files: &[RenderedFile], root: &Path) -> Result<(), ScaffoldError> {
    for file in files {
        write_file(root, &file.dest, &file.content)?;
    }

    Ok(())
}

/// Write one file under `root`, creating parent directories as needed.
fn write_file(root: &Path, dest: &Path, content: &[u8]) -> Result<(), ScaffoldError> {
    let path = root.join(dest);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| ScaffoldError::Io {
            path: parent.display().to_string(),
            source: e,
        })?;
    }

    fs::write(&path, content).map_err(|e| ScaffoldError::Io {
        path: path.display().to_string(),
        source: e,
    })
}

/// Apply an [`UpdatePlan`] to an existing project at `root`.
///
/// Component directories that are unchanged (KEEP) are never in the plan, so the
/// user's code in them is never touched. The one exception is the generated
/// `.env` writer scripts, which the plan carries in `shared_files`: they
/// implement a mutual-exclusion protocol over one file and are only correct
/// while every writer in the project speaks the same version of it, so they are
/// refreshed even inside a KEPT directory. See `update::is_env_writer`.
///
/// The caller is responsible for the `Create`-target precondition (the dir must
/// not already exist) and the git-clean safety gate; this function is the disk
/// side effect only.
///
/// Ordering: directories that go away (`Remove`) or are swapped (`Replace`) are
/// deleted **first**, so a `Replace` removes the old tool's files before the new
/// tool's are written into the same directory. Kept components are outside the
/// plan entirely, so this never risks losing user work in them.
pub fn apply_update(plan: &UpdatePlan, root: &Path) -> Result<(), ScaffoldError> {
    // 1. Remove departing / replaced component directories.
    for op in &plan.slot_ops {
        if let SlotOp::Remove(dir) | SlotOp::Replace(dir) = op {
            let path = root.join(dir);
            if path.exists() {
                fs::remove_dir_all(&path).map_err(|e| ScaffoldError::Io {
                    path: path.display().to_string(),
                    source: e,
                })?;
            }
        }
    }

    // 2. Write component files for created / replaced / re-rendered directories.
    for file in &plan.creates {
        write_file(root, &file.dest, &file.content)?;
    }

    // 3. Overwrite shared files, skipping byte-identical ones so an unaffected
    //    file (e.g. `.gitignore`, or an env writer already on this version) is
    //    left alone.
    for file in &plan.shared_files {
        let path = root.join(&file.dest);
        let unchanged = fs::read(&path)
            .map(|cur| cur == file.content)
            .unwrap_or(false);
        if !unchanged {
            write_file(root, &file.dest, &file.content)?;
        }
    }

    // 4. Remove shared files that no longer apply.
    for dest in &plan.shared_removals {
        let path = root.join(dest);
        if path.exists() {
            fs::remove_file(&path).map_err(|e| ScaffoldError::Io {
                path: path.display().to_string(),
                source: e,
            })?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn writes_files_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![
            RenderedFile {
                dest: PathBuf::from("hello.txt"),
                content: b"hello world".to_vec(),
            },
            RenderedFile {
                dest: PathBuf::from("sub/dir/nested.txt"),
                content: b"nested content".to_vec(),
            },
            RenderedFile {
                dest: PathBuf::from("empty/.gitkeep"),
                content: Vec::new(),
            },
        ];

        write(&files, dir.path()).unwrap();

        assert_eq!(
            fs::read_to_string(dir.path().join("hello.txt")).unwrap(),
            "hello world"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("sub/dir/nested.txt")).unwrap(),
            "nested content"
        );
        assert!(dir.path().join("empty/.gitkeep").exists());
        assert_eq!(
            fs::read(dir.path().join("empty/.gitkeep")).unwrap().len(),
            0
        );
    }

    #[test]
    fn creates_directory_structure() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![RenderedFile {
            dest: PathBuf::from("a/b/c/deep.txt"),
            content: b"deep".to_vec(),
        }];

        write(&files, dir.path()).unwrap();

        assert!(dir.path().join("a/b/c").is_dir());
        assert!(dir.path().join("a/b/c/deep.txt").is_file());
    }

    // -----------------------------------------------------------------------
    // apply_update: end-to-end against a real scaffolded tree
    // -----------------------------------------------------------------------

    use crate::registry::loader::Registry;
    use crate::registry::types::{Network, Role, RoleAssignment, Selection};
    use crate::scaffold::update::{Mutation, apply, apply_all, plan_update};

    fn reg() -> Registry {
        Registry::load().expect("registry loads")
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    fn sel(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "proj".into(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
    }

    /// Scaffold `s` into a fresh temp dir and return (tempdir, root path).
    fn scaffolded(s: &Selection) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        crate::scaffold::scaffold(s, &reg(), &root).unwrap();
        (tmp, root)
    }

    #[test]
    fn add_offchain_wires_the_tree() {
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let (_tmp, root) = scaffolded(&old);

        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "meshjs")));
        let plan = plan_update(&old, &new, &reg()).unwrap();
        apply_update(&plan, &root).unwrap();

        assert!(root.join("off-chain/package.json").is_file());
        assert!(root.join("on-chain/aiken.toml").is_file()); // on-chain untouched
        let justfile = fs::read_to_string(root.join("Justfile")).unwrap();
        assert!(justfile.contains("build-off-chain"));
    }

    #[test]
    fn remove_offchain_preserves_onchain_user_edits() {
        // The scenario from the design discussion: edits to a KEPT component
        // survive a removal of a different component.
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let (_tmp, root) = scaffolded(&old);

        // Simulate user work in the on-chain component.
        fs::write(root.join("on-chain/USER_NOTES.md"), b"my hard work").unwrap();
        let validator_before = fs::read(root.join("on-chain/validators/giftcard.ak")).unwrap();

        let new = apply(&old, &Mutation::Remove(Role::OffChain));
        let plan = plan_update(&old, &new, &reg()).unwrap();
        apply_update(&plan, &root).unwrap();

        assert!(!root.join("off-chain").exists()); // removed
        // KEEP guarantee: on-chain is byte-for-byte intact, user file included.
        assert_eq!(
            fs::read_to_string(root.join("on-chain/USER_NOTES.md")).unwrap(),
            "my hard work"
        );
        assert_eq!(
            fs::read(root.join("on-chain/validators/giftcard.ak")).unwrap(),
            validator_before
        );
        let justfile = fs::read_to_string(root.join("Justfile")).unwrap();
        assert!(!justfile.contains("build-off-chain"));
    }

    #[test]
    fn swap_offchain_replaces_the_dir_and_matches_fresh_scaffold() {
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let (_tmp, root) = scaffolded(&old);
        fs::write(root.join("off-chain/USER.txt"), b"gone after swap").unwrap();

        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "tx3")));
        let plan = plan_update(&old, &new, &reg()).unwrap();
        apply_update(&plan, &root).unwrap();

        // The replaced dir is wiped (user file gone) and now matches a fresh
        // scaffold of the new selection.
        assert!(!root.join("off-chain/USER.txt").exists());
        let (_tmp2, fresh) = scaffolded(&new);
        assert_eq!(
            read_tree(&root.join("off-chain")),
            read_tree(&fresh.join("off-chain"))
        );
        // on-chain still intact.
        assert!(root.join("on-chain/aiken.toml").is_file());
    }

    #[test]
    fn fusion_swap_replaces_two_dirs_with_protocol() {
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let (_tmp, root) = scaffolded(&old);

        let new = apply_all(
            &old,
            &[
                Mutation::Add(a(Role::OnChain, "scalus")),
                Mutation::Add(a(Role::OffChain, "scalus")),
            ],
        );
        let plan = plan_update(&old, &new, &reg()).unwrap();
        apply_update(&plan, &root).unwrap();

        assert!(!root.join("on-chain").exists());
        assert!(!root.join("off-chain").exists());
        assert!(root.join("protocol").is_dir());
    }

    /// Recursively read a directory into a sorted map of relative-path → bytes.
    fn read_tree(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut out = std::collections::BTreeMap::new();
        fn walk(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, Vec<u8>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(base, &path, out);
                } else {
                    let rel = path
                        .strip_prefix(base)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned();
                    out.insert(rel, fs::read(&path).unwrap());
                }
            }
        }
        if root.is_dir() {
            walk(root, root, &mut out);
        }
        out
    }
}
