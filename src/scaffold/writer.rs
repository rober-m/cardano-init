use std::fs;
use std::path::Path;

use super::renderer::RenderedFile;
use super::ScaffoldError;

/// Write all rendered files to disk under `root`.
///
/// Creates directories as needed. This is the only phase with side effects.
pub fn write(files: &[RenderedFile], root: &Path) -> Result<(), ScaffoldError> {
    for file in files {
        let dest = root.join(&file.dest);

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| ScaffoldError::Io {
                path: parent.display().to_string(),
                source: e,
            })?;
        }

        fs::write(&dest, &file.content).map_err(|e| ScaffoldError::Io {
            path: dest.display().to_string(),
            source: e,
        })?;
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
        let files = vec![
            RenderedFile {
                dest: PathBuf::from("a/b/c/deep.txt"),
                content: b"deep".to_vec(),
            },
        ];

        write(&files, dir.path()).unwrap();

        assert!(dir.path().join("a/b/c").is_dir());
        assert!(dir.path().join("a/b/c/deep.txt").is_file());
    }
}
