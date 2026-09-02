//! Loads the embedded `registry/deps.toml` into a `DepCatalog` (TECH_SPEC §9.2).
//!
//! Recipes are **data**; the installer vocabulary they reference is **code**
//! (`installers.rs`). Installer keys are validated against the `Installer` enum
//! here, at load — an unknown installer is a load error, exactly like an unknown
//! `Role` in a tool file.

use std::collections::HashMap;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::installers::Installer;

#[derive(RustEmbed)]
#[folder = "registry/"]
#[include = "deps.toml"]
struct DepsAsset;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum CatalogError {
    #[error("registry/deps.toml is missing or not valid UTF-8")]
    Missing,

    #[error("failed to parse registry/deps.toml: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("dep '{dep}' has a malformed install entry (expected a single {{ installer = arg }})")]
    MalformedInstall { dep: String },

    #[error("unknown installer '{installer}' in dep '{dep}'")]
    UnknownInstaller { dep: String, installer: String },

    #[error("unknown OS '{os}' in dep '{dep}' (expected linux, macos, windows, or other)")]
    UnknownOs { dep: String, os: String },
}

// ---------------------------------------------------------------------------
// Domain types
// ---------------------------------------------------------------------------

/// A single install method: which installer, and the arg to pass it.
#[derive(Debug, Clone)]
pub struct InstallMethod {
    pub installer: Installer,
    pub arg: String,
}

/// A per-dependency recipe.
#[derive(Debug, Clone)]
pub struct DepRecipe {
    /// Presence check: the dep is present if any of these is on `PATH`.
    pub binaries: Vec<String>,
    /// Additional presence binaries for a specific operating system.
    pub binaries_by_os: HashMap<String, Vec<String>>,
    /// Universal fallback when the resolver can't produce a plan.
    pub docs: String,
    /// Ordered install methods (order = preference).
    pub install: Vec<InstallMethod>,
}

/// All dep recipes, keyed by dep id.
#[derive(Debug, Clone)]
pub struct DepCatalog {
    recipes: HashMap<String, DepRecipe>,
}

// ---------------------------------------------------------------------------
// TOML intermediate types (private)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct DepRecipeToml {
    binaries: Vec<String>,
    #[serde(default)]
    binaries_by_os: HashMap<String, Vec<String>>,
    docs: String,
    /// Each entry is a single-key table: `{ brew = "node" }`.
    install: Vec<HashMap<String, String>>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

impl DepCatalog {
    /// Load and validate the embedded `registry/deps.toml`.
    pub fn load() -> Result<Self, CatalogError> {
        let data = DepsAsset::get("deps.toml").ok_or(CatalogError::Missing)?;
        let text = std::str::from_utf8(&data.data).map_err(|_| CatalogError::Missing)?;
        Self::from_str(text)
    }

    /// Parse and validate from a TOML string (also the test entry point).
    pub fn from_str(text: &str) -> Result<Self, CatalogError> {
        let raw: HashMap<String, DepRecipeToml> = toml::from_str(text)?;
        let mut recipes = HashMap::with_capacity(raw.len());

        for (dep, recipe) in raw {
            for os in recipe.binaries_by_os.keys() {
                if !matches!(os.as_str(), "linux" | "macos" | "windows" | "other") {
                    return Err(CatalogError::UnknownOs {
                        dep: dep.clone(),
                        os: os.clone(),
                    });
                }
            }
            let mut install = Vec::with_capacity(recipe.install.len());
            for entry in recipe.install {
                // Exactly one { installer = arg } per entry.
                if entry.len() != 1 {
                    return Err(CatalogError::MalformedInstall { dep: dep.clone() });
                }
                let (key, arg) = entry.into_iter().next().expect("len checked above");
                let installer =
                    Installer::from_key(&key).ok_or_else(|| CatalogError::UnknownInstaller {
                        dep: dep.clone(),
                        installer: key,
                    })?;
                install.push(InstallMethod { installer, arg });
            }
            recipes.insert(
                dep,
                DepRecipe {
                    binaries: recipe.binaries,
                    binaries_by_os: recipe.binaries_by_os,
                    docs: recipe.docs,
                    install,
                },
            );
        }

        Ok(DepCatalog { recipes })
    }

    /// Look up a recipe by dep id.
    pub fn get(&self, dep_id: &str) -> Option<&DepRecipe> {
        self.recipes.get(dep_id)
    }

    /// All dep ids (unspecified order).
    pub fn dep_ids(&self) -> impl Iterator<Item = &str> {
        self.recipes.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_loads() {
        let cat = DepCatalog::load().expect("registry/deps.toml should load");
        // Base dep + a couple of tool deps must exist.
        assert!(cat.get("just").is_some());
        assert!(cat.get("node").is_some());
        assert!(cat.get("aiken").is_some());
    }

    #[test]
    fn aiken_recipe_shape() {
        let cat = DepCatalog::load().unwrap();
        let aiken = cat.get("aiken").unwrap();
        assert_eq!(aiken.binaries, vec!["aiken".to_string()]);
        // First method is aikup, second is nix (order = preference).
        assert_eq!(aiken.install[0].installer, Installer::Aikup);
        // Empty arg ⇒ `aikup install` (latest); "latest" is not a valid tag.
        assert_eq!(aiken.install[0].arg, "");
        assert_eq!(aiken.install[1].installer, Installer::Nix);
    }

    #[test]
    fn env_lock_recipe_has_platform_alternatives() {
        let cat = DepCatalog::load().unwrap();
        let lock = cat.get("env-lock").unwrap();
        assert!(lock.binaries.is_empty());
        assert_eq!(lock.binaries_by_os["linux"], ["flock"]);
        assert_eq!(lock.binaries_by_os["macos"], ["lockf"]);
        assert_eq!(
            lock.binaries_by_os["windows"],
            ["powershell.exe", "pwsh.exe"]
        );
        assert!(lock.install.is_empty());
    }

    #[test]
    fn unknown_installer_is_load_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ snap = "foo" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::UnknownInstaller { .. }));
    }

    #[test]
    fn unknown_os_is_load_error() {
        let toml = r#"
[foo]
binaries = []
binaries_by_os = { plan9 = ["foo"] }
docs = "https://example.com"
install = []
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::UnknownOs { .. }));
    }

    #[test]
    fn multi_key_install_entry_is_error() {
        let toml = r#"
[foo]
binaries = ["foo"]
docs = "https://example.com"
install = [{ brew = "foo", apt = "foo" }]
"#;
        let err = DepCatalog::from_str(toml).unwrap_err();
        assert!(matches!(err, CatalogError::MalformedInstall { .. }));
    }
}
