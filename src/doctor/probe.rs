//! Impure system probes for the doctor (TECH_SPEC §9.3).
//!
//! This is the only doctor module that touches the system: it detects the OS,
//! which installers are on `PATH`, which dependency binaries are present, and
//! scans a generated project to identify the tool filling each role directory.
//! No execution, no version detection (v1).

use std::collections::HashSet;
use std::path::Path;
use std::str::FromStr;

use serde::Serialize;

use super::catalog::DepCatalog;
use super::installers::Installer;
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{DetectSignature, Network, Role, RoleAssignment, Selection, ToolDef};

/// Detected operating system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
#[allow(clippy::enum_variant_names)] // `MacOs` ends in "Os"; renaming would change the serialized value
pub enum Os {
    Linux,
    MacOs,
    Windows,
    Other,
}

impl Os {
    /// The OS of the running host.
    pub fn detect() -> Self {
        match std::env::consts::OS {
            "linux" => Os::Linux,
            "macos" => Os::MacOs,
            "windows" => Os::Windows,
            _ => Os::Other,
        }
    }

    /// Stable key used by platform-specific dependency recipe data.
    pub fn as_key(self) -> &'static str {
        match self {
            Os::Linux => "linux",
            Os::MacOs => "macos",
            Os::Windows => "windows",
            Os::Other => "other",
        }
    }
}

/// The probed environment the pure resolver runs against. Holds everything
/// system-derived so the resolver itself stays pure and unit-testable with a
/// synthetic value.
#[derive(Debug, Clone)]
pub struct Environment {
    pub os: Os,
    /// Installers detected as available on this host.
    pub installers: HashSet<Installer>,
    /// Dependency/installer binaries found on `PATH`.
    pub present_binaries: HashSet<String>,
}

/// Detect the environment: OS, available installers, and which catalog/installer
/// binaries are on `PATH`.
pub fn detect_environment(catalog: &DepCatalog) -> Environment {
    let os = Os::detect();
    // All binaries worth probing: every dep's general and current-OS presence
    // binaries plus every installer's detection binaries.
    let mut binaries: HashSet<String> = HashSet::new();
    for id in catalog.dep_ids() {
        if let Some(recipe) = catalog.get(id) {
            binaries.extend(recipe.binaries.iter().cloned());
            if let Some(items) = recipe.binaries_by_os.get(os.as_key()) {
                binaries.extend(items.iter().cloned());
            }
        }
    }
    for installer in Installer::ALL {
        binaries.extend(installer.detect().iter().map(|s| s.to_string()));
    }

    let present_binaries: HashSet<String> =
        binaries.into_iter().filter(|b| is_on_path(b)).collect();

    let installers: HashSet<Installer> = Installer::ALL
        .iter()
        .copied()
        .filter(|inst| inst.detect().iter().any(|b| present_binaries.contains(*b)))
        .collect();

    Environment {
        os,
        installers,
        present_binaries,
    }
}

/// True if `bin` is found in any `PATH` entry. On Windows, also tries common
/// executable extensions.
fn is_on_path(bin: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for dir in std::env::split_paths(&path) {
        if dir.join(bin).is_file() {
            return true;
        }
        if cfg!(windows) {
            for ext in ["exe", "cmd", "bat"] {
                if dir.join(format!("{bin}.{ext}")).is_file() {
                    return true;
                }
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Project scan
// ---------------------------------------------------------------------------

/// What a scanned component directory is: one of the five roles, or the fused
/// fullstack `protocol/` component (which is not a role, TECH_SPEC §3.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Role(Role),
    Protocol,
}

impl ComponentKind {
    /// The directory / kebab identifier: the role's kebab, or `"protocol"`.
    pub fn as_kebab(&self) -> &'static str {
        match self {
            ComponentKind::Role(role) => role.as_kebab(),
            ComponentKind::Protocol => contract::DIR_PROTOCOL,
        }
    }
}

impl std::fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ComponentKind::Role(role) => write!(f, "{role}"),
            ComponentKind::Protocol => write!(f, "Protocol"),
        }
    }
}

/// A component directory whose contents were recognized as a specific tool.
#[derive(Debug, Clone, Serialize)]
pub struct DetectedComponent {
    #[serde(rename = "role", serialize_with = "ser_kind")]
    pub kind: ComponentKind,
    pub tool_id: String,
}

/// A component directory that exists but whose contents matched no known tool
/// (renamed, modified, or a tool not in the registry).
#[derive(Debug, Clone, Serialize)]
pub struct UnrecognizedDir {
    #[serde(rename = "role", serialize_with = "ser_kind")]
    pub kind: ComponentKind,
    pub dir: String,
}

fn ser_kind<S: serde::Serializer>(kind: &ComponentKind, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(kind.as_kebab())
}

/// The result of scanning a project tree.
#[derive(Debug, Clone, Serialize)]
pub struct ScanResult {
    pub components: Vec<DetectedComponent>,
    pub unrecognized: Vec<UnrecognizedDir>,
}

/// True if `dir` holds the aggregated cardano-up infra driver: a `Justfile`
/// that references `cardano-up`. The infra component has no per-tool subdirs to
/// match against `detect` signatures, so it is recognized by this driver marker
/// (TECH_SPEC §9.6).
fn infra_driver_present(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("Justfile"))
        .map(|text| text.contains("cardano-up"))
        .unwrap_or(false)
}

/// True if a detect signature matches under `dir`: the file exists and, when a
/// `contains` substring is given, the file's text includes it.
fn signature_matches(dir: &Path, sig: &DetectSignature) -> bool {
    let path = dir.join(&sig.file);
    match &sig.contains {
        None => path.exists(),
        Some(needle) => std::fs::read_to_string(&path)
            .map(|text| text.contains(needle.as_str()))
            .unwrap_or(false),
    }
}

/// True if `tool` matches `dir` via a **definitive** signature: an
/// existence-only check (no `contains` needle) on a manifest file that is
/// unique to the tool (e.g. `trix.toml`, `aiken.toml`). These are stronger than
/// `contains` needles on shared files (`package.json` → `@meshsdk`), which only
/// prove that a *generic* file mentions the tool.
fn matches_definitively(dir: &Path, tool: &ToolDef) -> bool {
    tool.detect
        .iter()
        .any(|sig| sig.contains.is_none() && signature_matches(dir, sig))
}

/// Identify the single tool from `candidates` whose generated output occupies
/// `dir`. A tool matches if **any** of its `detect` signatures matches (§9.6);
/// exactly one match ⇒ identified. On an ambiguous multiple, a definitive
/// manifest match wins over tools matched only by a shared-file `contains`
/// needle — a `trix.toml` is unmistakably a Tx3 project even when its
/// `package.json` also pulls in `@meshsdk` as a library. If the definitive
/// matchers still don't narrow to exactly one, the directory is unrecognized.
fn identify_tool<'a>(dir: &Path, candidates: &[&'a ToolDef]) -> Option<&'a str> {
    let matched: Vec<&'a ToolDef> = candidates
        .iter()
        .copied()
        .filter(|tool| tool.detect.iter().any(|sig| signature_matches(dir, sig)))
        .collect();

    match matched.as_slice() {
        [tool] => Some(tool.id.as_str()),
        [] => None,
        _ => {
            let definitive: Vec<&&ToolDef> = matched
                .iter()
                .filter(|tool| matches_definitively(dir, tool))
                .collect();
            match definitive.as_slice() {
                [tool] => Some(tool.id.as_str()),
                _ => None,
            }
        }
    }
}

/// Scan a project root for role directories and identify the tool in each.
///
/// For each contract role directory present, the candidate tools are exactly
/// those that declare that role; a tool matches if any of its `detect`
/// signature files exists under the directory. Exactly one match ⇒ detected;
/// zero (or an ambiguous multiple) ⇒ unrecognized.
pub fn scan_project(root: &Path, registry: &Registry) -> ScanResult {
    let mut components = Vec::new();
    let mut unrecognized = Vec::new();

    for &role in Role::ALL {
        let dir = root.join(role.dir());
        if !dir.is_dir() {
            continue;
        }

        // Infrastructure aggregates into a single cardano-up-driven component
        // (no per-tool subdirs), so it is recognized by the driver marker rather
        // than per-tool `detect` signatures.
        if role == Role::Infrastructure {
            if infra_driver_present(&dir) {
                components.push(DetectedComponent {
                    kind: ComponentKind::Role(role),
                    tool_id: super::INFRA_DRIVER_ID.to_string(),
                });
            } else {
                unrecognized.push(UnrecognizedDir {
                    kind: ComponentKind::Role(role),
                    dir: role.dir().to_string(),
                });
            }
            continue;
        }

        let candidates = registry.tools_for_role(role);
        if let Some(tool_id) = identify_tool(&dir, &candidates) {
            components.push(DetectedComponent {
                kind: ComponentKind::Role(role),
                tool_id: tool_id.to_string(),
            });
        } else {
            unrecognized.push(UnrecognizedDir {
                kind: ComponentKind::Role(role),
                dir: role.dir().to_string(),
            });
        }
    }

    // Fullstack `protocol/` is not a role directory, so it is scanned separately:
    // matched against the `detect` signatures of tools that declare a `[fullstack]`
    // template. The identified tool is a real registry tool, so its `system_deps`
    // feed the required set through the normal path (TECH_SPEC §9.6).
    let protocol_dir = root.join(contract::DIR_PROTOCOL);
    if protocol_dir.is_dir() {
        let candidates: Vec<&ToolDef> = registry
            .all_tools()
            .iter()
            .filter(|tool| tool.fullstack.is_some())
            .collect();

        if let Some(tool_id) = identify_tool(&protocol_dir, &candidates) {
            components.push(DetectedComponent {
                kind: ComponentKind::Protocol,
                tool_id: tool_id.to_string(),
            });
        } else {
            unrecognized.push(UnrecognizedDir {
                kind: ComponentKind::Protocol,
                dir: contract::DIR_PROTOCOL.to_string(),
            });
        }
    }

    ScanResult {
        components,
        unrecognized,
    }
}

// ---------------------------------------------------------------------------
// Selection reconstruction
// ---------------------------------------------------------------------------

/// A `Selection` reconstructed from an existing project tree, plus the caveats
/// the reconstruction hit. Detection is the source of truth (no persisted
/// manifest — matching the doctor's "structure is the source of truth", §9.6),
/// so the update path *confirms* this with the user before acting on it rather
/// than trusting it silently.
#[derive(Debug, Clone)]
pub struct Reconstructed {
    /// Best-effort recovered selection.
    pub selection: Selection,
    /// Component directories that exist but matched no known tool (renamed,
    /// hand-edited, or foreign) — carried straight from [`scan_project`].
    pub unrecognized: Vec<UnrecognizedDir>,
    /// Scalar fields that had to be guessed (e.g. `"network"` when `.env` is
    /// missing/edited), so the confirm step can flag them.
    pub low_confidence: Vec<&'static str>,
    /// `cardano-up` packages found in `infra/Justfile` that map to no registry
    /// tool (surfaced, never silently dropped).
    pub unknown_infra: Vec<String>,
}

/// Reconstruct the [`Selection`] that would (re)generate the project at `root`.
///
/// Roles and tools come from [`scan_project`]; the infra provider set is
/// recovered by parsing `infra/Justfile` (the aggregated component collapses to
/// a single driver marker in the scan); `network`/`nix`/`project_name` are read
/// from `.env`, `flake.nix`, and the directory name. Anything not cleanly
/// recoverable is reported in `low_confidence`/`unknown_infra` for confirmation.
pub fn reconstruct(root: &Path, registry: &Registry) -> Reconstructed {
    let scan = scan_project(root, registry);

    let mut assignments = Vec::new();
    let mut low_confidence = Vec::new();
    let mut unknown_infra = Vec::new();

    for component in &scan.components {
        match component.kind {
            // Infrastructure collapses to one driver marker in the scan; recover
            // the individual providers from the generated Justfile.
            ComponentKind::Role(Role::Infrastructure) => {
                let (tool_ids, unknown) = parse_infra_providers(root, registry);
                if tool_ids.is_empty() {
                    low_confidence.push("infra");
                }
                for tool_id in tool_ids {
                    assignments.push(RoleAssignment {
                        role: Role::Infrastructure,
                        tool_id,
                    });
                }
                unknown_infra.extend(unknown);
            }
            ComponentKind::Role(role) => assignments.push(RoleAssignment {
                role,
                tool_id: component.tool_id.clone(),
            }),
            // A fused `protocol/` is one tool filling both roles; re-expand it to
            // the two assignments the planner re-collapses (TECH_SPEC §3.4).
            ComponentKind::Protocol => {
                assignments.push(RoleAssignment {
                    role: Role::OnChain,
                    tool_id: component.tool_id.clone(),
                });
                assignments.push(RoleAssignment {
                    role: Role::OffChain,
                    tool_id: component.tool_id.clone(),
                });
            }
        }
    }

    if !unknown_infra.is_empty() && !low_confidence.contains(&"infra") {
        low_confidence.push("infra");
    }

    let network = read_network(root).unwrap_or_else(|| {
        low_confidence.push("network");
        Network::Preview
    });
    let nix = root.join("flake.nix").is_file();
    let project_name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();

    Reconstructed {
        selection: Selection {
            project_name,
            assignments,
            network,
            nix,
        },
        unrecognized: scan.unrecognized,
        low_confidence,
        unknown_infra,
    }
}

/// Recover the infra provider set from `infra/Justfile` by scanning its
/// `cardano-up install <package> …` lines (see
/// `templates/_infra/cardano-up/Justfile.jinja`) and reverse-mapping each
/// `<package>` to a registry tool. Returns `(recovered tool ids, unknown
/// packages)`; a missing/unreadable Justfile yields empty vecs.
fn parse_infra_providers(root: &Path, registry: &Registry) -> (Vec<String>, Vec<String>) {
    let text = match std::fs::read_to_string(root.join(contract::DIR_INFRA).join("Justfile")) {
        Ok(t) => t,
        Err(_) => return (Vec::new(), Vec::new()),
    };

    let mut tool_ids = Vec::new();
    let mut unknown = Vec::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        for window in tokens.windows(3) {
            if window[0] == "cardano-up" && window[1] == "install" {
                let pkg = window[2];
                match tool_for_package(pkg, registry) {
                    Some(id) => {
                        if !tool_ids.contains(&id) {
                            tool_ids.push(id);
                        }
                    }
                    None => {
                        if !unknown.iter().any(|u| u == pkg) {
                            unknown.push(pkg.to_string());
                        }
                    }
                }
            }
        }
    }
    (tool_ids, unknown)
}

/// The infra tool whose `cardano_up_package` equals `pkg`, if any. No reverse
/// index exists in the registry, so iterate the infra tools (matching the
/// existing `required_deps` style in the CLI).
fn tool_for_package(pkg: &str, registry: &Registry) -> Option<String> {
    registry
        .tools_for_role(Role::Infrastructure)
        .into_iter()
        .find(|t| {
            t.infra
                .as_ref()
                .is_some_and(|i| i.cardano_up_package == pkg)
        })
        .map(|t| t.id.clone())
}

/// Read `CARDANO_NETWORK` from the project's `.env`, if present and valid.
fn read_network(root: &Path) -> Option<Network> {
    let text = std::fs::read_to_string(root.join(".env")).ok()?;
    let prefix = format!("{}=", contract::ENV_NETWORK);
    for line in text.lines() {
        if let Some(value) = line.trim().strip_prefix(&prefix) {
            return Network::from_str(value.trim()).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn registry() -> Registry {
        Registry::load().expect("registry loads")
    }

    #[test]
    fn os_detect_is_known_on_ci() {
        // Just exercise the path; the value depends on the host.
        let _ = Os::detect();
    }

    #[test]
    fn scan_identifies_aiken_and_meshjs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // on-chain/ with aiken signature
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/aiken.toml"), "").unwrap();

        // off-chain/ with meshjs signature (package.json referencing @meshsdk)
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "@meshsdk/core": "^1.9.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());

        assert_eq!(result.components.len(), 2);
        let onchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OnChain))
            .unwrap();
        assert_eq!(onchain.tool_id, "aiken");
        let offchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OffChain))
            .unwrap();
        assert_eq!(offchain.tool_id, "meshjs");
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn foreign_package_json_is_unrecognized_not_meshjs() {
        // A from-scratch JS project (e.g. Next.js) has a package.json but no
        // @meshsdk dependency — content-aware detection must NOT call it MeshJS.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "next": "^14.0.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::OffChain)
        );
    }

    #[test]
    fn scan_identifies_tx3_despite_meshsdk_dependency() {
        // The Tx3 off-chain template legitimately depends on @meshsdk/core (it
        // imports it in blueprint.ts), so its package.json trips MeshJS's
        // `contains = "@meshsdk"` needle. Both tools match the dir — but a
        // trix.toml is a definitive, Tx3-unique manifest, so it must win over
        // the shared-file MeshJS match rather than reading as ambiguous.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("off-chain")).unwrap();
        fs::write(root.join("off-chain/trix.toml"), "[protocol]\n").unwrap();
        fs::write(
            root.join("off-chain/package.json"),
            r#"{ "dependencies": { "@meshsdk/core": "^1.9.0", "tx3-sdk": "^0.15.0" } }"#,
        )
        .unwrap();

        let result = scan_project(root, &registry());

        assert!(result.unrecognized.is_empty());
        let offchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OffChain))
            .expect("off-chain component identified");
        assert_eq!(offchain.tool_id, "tx3");
    }

    #[test]
    fn scan_flags_unrecognized_role_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // on-chain/ exists but contains no recognizable signature (renamed files).
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/renamed.txt"), "").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::OnChain)
        );
    }

    #[test]
    fn scan_distinguishes_scalus_on_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(
            root.join("on-chain/build.sbt"),
            "\"org.scalus\" %% \"scalus\" % scalusVersion\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let onchain = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::OnChain))
            .unwrap();
        assert_eq!(onchain.tool_id, "scalus");
    }

    #[test]
    fn scan_recognizes_aggregated_infra_by_driver_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // Aggregated infra/: a single Justfile referencing cardano-up, no subdirs.
        fs::create_dir_all(root.join("infra")).unwrap();
        fs::write(
            root.join("infra/Justfile"),
            "dev:\n    cardano-up up --context demo\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let infra = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Role(Role::Infrastructure))
            .expect("infra should be detected via the driver marker");
        assert_eq!(infra.tool_id, crate::doctor::INFRA_DRIVER_ID);
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn scan_infra_without_marker_is_unrecognized() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("infra")).unwrap();
        fs::write(root.join("infra/Justfile"), "dev:\n    echo nope\n").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(
            result.unrecognized[0].kind,
            ComponentKind::Role(Role::Infrastructure)
        );
    }

    #[test]
    fn scan_detects_fullstack_protocol_component() {
        // A protocol/ dir with scalus's signatures is identified as the scalus
        // fullstack component (its real system_deps then feed the required set).
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("protocol")).unwrap();
        fs::write(
            root.join("protocol/build.sbt"),
            "\"org.scalus\" %% \"scalus\" % scalusVersion\n",
        )
        .unwrap();

        let result = scan_project(root, &registry());
        let protocol = result
            .components
            .iter()
            .find(|c| c.kind == ComponentKind::Protocol)
            .expect("protocol component should be detected");
        assert_eq!(protocol.tool_id, "scalus");
        assert!(result.unrecognized.is_empty());
    }

    #[test]
    fn scan_protocol_without_signatures_is_unrecognized() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("protocol")).unwrap();
        fs::write(root.join("protocol/whatever.txt"), "").unwrap();

        let result = scan_project(root, &registry());
        assert!(result.components.is_empty());
        assert_eq!(result.unrecognized.len(), 1);
        assert_eq!(result.unrecognized[0].kind, ComponentKind::Protocol);
    }

    #[test]
    fn scan_empty_project_finds_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let result = scan_project(tmp.path(), &registry());
        assert!(result.components.is_empty());
        assert!(result.unrecognized.is_empty());
    }

    // -----------------------------------------------------------------------
    // Reconstruction round-trips: scaffold a real project, then reconstruct its
    // Selection and assert it matches what generated it.
    // -----------------------------------------------------------------------

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    /// Scaffold `selection` into `<tmp>/<name>` and reconstruct it back. The
    /// project is generated into a dir named after the project so the recovered
    /// `project_name` (derived from the dir) round-trips too.
    fn roundtrip(assignments: Vec<RoleAssignment>, network: Network, nix: bool) -> Reconstructed {
        let reg = registry();
        let sel = Selection {
            project_name: "my-protocol".to_string(),
            assignments,
            network,
            nix,
        };
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join(&sel.project_name);
        crate::scaffold::scaffold(&sel, &reg, &root).unwrap();
        // reconstruct reads eagerly, so it's safe to let `tmp` drop afterwards.
        reconstruct(&root, &reg)
    }

    /// Assignments compared as a set (planner order is not canonical in a
    /// `Selection`; the plan sorts at generation time).
    fn assert_same_assignments(mut got: Vec<RoleAssignment>, mut want: Vec<RoleAssignment>) {
        let key = |r: &RoleAssignment| (r.role.as_kebab(), r.tool_id.clone());
        got.sort_by_key(&key);
        want.sort_by_key(&key);
        assert_eq!(got, want);
    }

    #[test]
    fn reconstruct_aiken_and_meshjs() {
        let r = roundtrip(
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
            Network::Preview,
            false,
        );
        assert!(r.unrecognized.is_empty());
        assert!(r.low_confidence.is_empty(), "{:?}", r.low_confidence);
        assert_eq!(r.selection.project_name, "my-protocol");
        assert_eq!(r.selection.network, Network::Preview);
        assert!(!r.selection.nix);
        assert_same_assignments(
            r.selection.assignments,
            vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")],
        );
    }

    #[test]
    fn reconstruct_recovers_infra_provider_set() {
        // The scan collapses infra to one driver marker; reconstruction must
        // recover the individual providers from infra/Justfile.
        let r = roundtrip(
            vec![
                a(Role::OnChain, "aiken"),
                a(Role::Infrastructure, "kupo"),
                a(Role::Infrastructure, "ogmios"),
            ],
            Network::Preview,
            false,
        );
        assert!(r.unknown_infra.is_empty());
        assert_same_assignments(
            r.selection.assignments,
            vec![
                a(Role::OnChain, "aiken"),
                a(Role::Infrastructure, "kupo"),
                a(Role::Infrastructure, "ogmios"),
            ],
        );
    }

    #[test]
    fn reconstruct_expands_fullstack_protocol() {
        // scalus fills both roles via [fullstack]; the fused protocol/ must
        // round-trip to the two assignments the planner re-collapses.
        let r = roundtrip(
            vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
            Network::Preview,
            false,
        );
        assert!(r.unrecognized.is_empty());
        assert_same_assignments(
            r.selection.assignments,
            vec![a(Role::OnChain, "scalus"), a(Role::OffChain, "scalus")],
        );
    }

    #[test]
    fn reconstruct_recovers_network_and_nix() {
        let r = roundtrip(vec![a(Role::OnChain, "aiken")], Network::Preprod, true);
        assert_eq!(r.selection.network, Network::Preprod);
        assert!(r.selection.nix);
        assert!(r.low_confidence.is_empty(), "{:?}", r.low_confidence);
    }

    #[test]
    fn reconstruct_missing_env_flags_network_low_confidence() {
        // A project with a role dir but no .env: network can't be recovered.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join("on-chain")).unwrap();
        fs::write(root.join("on-chain/aiken.toml"), "").unwrap();

        let r = reconstruct(root, &registry());
        assert_eq!(r.selection.network, Network::Preview);
        assert!(r.low_confidence.contains(&"network"));
    }
}
