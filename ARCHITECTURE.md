# Cardano Protocol Scaffolder — Architecture

## 1. Crate Structure

Single crate, well-modularized. The boundary between "library logic" and "CLI concerns" is enforced by module visibility, not crate boundaries. If we later need to extract a library (e.g., for WASM), the modules are already cleanly separated.

```
cardano-init/
├── Cargo.toml
├── build.rs                    # Generates asset manifest for rust-embed
├── src/
│   ├── main.rs                 # Entry point: parse args, dispatch to mode
│   │
│   ├── cli/                    # CLI-specific: user interaction, output formatting
│   │   ├── mod.rs
│   │   ├── interactive.rs      # Guided interactive flow (dialoguer)
│   │   ├── oneshot.rs          # Flag parsing and config-file loading
│   │   └── output.rs           # Terminal output: colors, spinners
│   │
│   ├── registry/               # Tool + role definitions, loaded from embedded TOML
│   │   ├── mod.rs
│   │   ├── types.rs            # ToolDef, RoleConfig, Role, etc.
│   │   └── loader.rs           # Deserialize embedded TOML into registry (via rust-embed)
│   │
│   ├── scaffold/               # Project generation
│   │   ├── mod.rs
│   │   ├── context.rs          # Build the template context from Selection + Registry
│   │   ├── planner.rs          # Determine which files to emit (dry-run lives here)
│   │   └── renderer.rs         # Render templates via MiniJinja, write to disk
│   │
│   └── contract.rs             # Interface contract constants (paths, env vars, task names)
│
├── registry/                   # Declarative data — embedded at compile time via rust-embed
│   └── tools/
│       ├── aiken.toml
│       ├── meshjs.toml
│       └── scalus.toml
│
└── templates/                  # Template files — embedded at compile time
    ├── _base/                  # Shared across all projects
    │   ├── Justfile.jinja
    │   ├── README.md.jinja
    │   ├── gitignore
    │   └── env.jinja
    ├── _nix/                   # Optional Nix layer
    │   └── flake.nix.jinja
    ├── aiken/
    │   └── on-chain/
    │       ├── manifest.toml
    │       ├── aiken.toml.jinja
    │       ├── README.md.jinja
    │       ├── Justfile.jinja
    │       └── validators/
    │           └── example.ak
    ├── meshjs/
    │   └── off-chain/
    │       ├── manifest.toml
    │       ├── package.json.jinja
    │       ├── README.md.jinja
    │       ├── Justfile.jinja
    │       └── src/
    │           └── index.ts
    └── scalus/
        └── testing/
            ├── manifest.toml
            └── ...
```

## 2. Tool Registry Schema

Each tool is defined in a single TOML file under `registry/tools/`.

```toml
# registry/tools/aiken.toml

[tool]
id = "aiken"
name = "Aiken"
description = """
A modern smart contract language for Cardano. Aiken has its own \
purpose-built syntax inspired by Rust and Gleam, and compiles to \
UPLC (Untyped Plutus Core). It's the most popular choice for \
writing Cardano validators and produces CIP-57 blueprints natively."""
website = "https://aiken-lang.org"
languages = ["aiken"]
system_deps = ["aiken-cli"]              # What needs to be installed

[roles]
# Each key is a role this tool can fill. The value configures
# behavior in that role.

[roles.on-chain]
template = "aiken/on-chain"              # Path under templates/

# Aiken doesn't fill other roles, so no other [roles.*] sections.
```

```toml
# registry/tools/scalus.toml

[tool]
id = "scalus"
name = "Scalus"
description = """
A Scala-native toolkit for Cardano. Scalus can compile Scala code \
to UPLC for on-chain validators, build transactions for off-chain \
interaction, and run validators locally for testing — all in one \
language. Choose it for one role or several."""
website = "https://scalus.org"
languages = ["scala"]
system_deps = ["sbt", "jvm"]

[roles.testing]
template = "scalus/testing"
```

### Rust types

```rust
// src/registry/types.rs

use std::collections::HashMap;

/// A loaded tool definition.
pub struct ToolDef {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website: String,
    pub languages: Vec<String>,
    pub roles: HashMap<Role, RoleConfig>,
}

/// The roles a tool can fill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    OnChain,
    OffChain,
    Infrastructure,
    Testing,
}

/// Per-role configuration for a tool.
pub struct RoleConfig {
    pub template: String,           // path under templates/
}
```


## 3. Interface Contract

Defined as constants in `src/contract.rs`. Every template must conform to these.

```rust
// src/contract.rs

/// Canonical path where on-chain builds produce the CIP-57 blueprint.
/// Relative to project root.
pub const BLUEPRINT_PATH: &str = "blueprint/plutus.json";

/// Directory names for each role. The role template is emitted into this directory.
pub const DIR_ON_CHAIN: &str = "on-chain";
pub const DIR_OFF_CHAIN: &str = "off-chain";
pub const DIR_INFRA: &str = "infra";
pub const DIR_TESTING: &str = "test";

/// Standard environment variable names for infrastructure.
/// Infra templates write these to .env; consumers read them.
pub const ENV_INDEXER_URL: &str = "INDEXER_URL";
pub const ENV_INDEXER_PORT: &str = "INDEXER_PORT";
pub const ENV_NODE_SOCKET_PATH: &str = "NODE_SOCKET_PATH";
pub const ENV_NETWORK: &str = "CARDANO_NETWORK";
```

### Contract compliance checklist (for template authors)

An on-chain template MUST:
1. Include a `Justfile` with targets: `build`, `test`, `dev`, `clean`.
2. Produce the CIP-57 blueprint at `../blueprint/plutus.json` during `build`.
3. Work independently — `just build` succeeds with no other roles present.

An off-chain template MUST:
1. Include a `Justfile` with targets: `build`, `test`, `dev`, `clean`.
2. Read the blueprint from `../blueprint/plutus.json` if it exists.
3. Read infrastructure env vars from `../.env` if it exists.
4. Work independently — if no blueprint exists, `build` still succeeds (possibly with a warning).

An infrastructure template MUST:
1. Include a `Justfile` with targets: `build`, `test`, `dev`, `clean`.
2. Write connection details to `../.env` using the standard variable names during `dev`.
3. Work independently.

A testing template MUST:
1. Include a `Justfile` with targets: `build`, `test`, `dev`, `clean`.
2. Read the blueprint from `../blueprint/plutus.json` if needed.
3. Read infrastructure env vars from `../.env` if needed.
4. Work independently.


## 4. Template Manifests

Each template directory contains a `manifest.toml` that describes its contents
for the scaffolder.

```toml
# templates/aiken/on-chain/manifest.toml

[manifest]
# Human-readable, shown in interactive mode when this template is selected.
summary = "Aiken on-chain project with a simple always-succeeds validator"

# Files to render. Paths are relative to this template directory.
# Destination is relative to the role directory (e.g., on-chain/).
[[files]]
source = "aiken.toml.jinja"
dest = "aiken.toml"
render = true                     # Process through MiniJinja

[[files]]
source = "Justfile.jinja"
dest = "Justfile"
render = true

[[files]]
source = "README.md.jinja"
dest = "README.md"
render = true

[[files]]
source = "validators/example.ak"
dest = "validators/example.ak"
render = false                    # Copy as-is
```


## 5. Selection

### 5.1 The Selection type

A `Selection` is the fully resolved set of user choices, passed directly to the
scaffolding pipeline.

```rust
// Represents one tool assigned to one role.
pub struct RoleAssignment {
    pub role: Role,
    pub tool_id: String,
}

/// The complete user selection.
pub struct Selection {
    pub project_name: String,
    pub assignments: Vec<RoleAssignment>,  // Infra may have multiple entries
    pub network: Network,
    pub nix: bool,
}

pub enum Network {
    Preview,
    Preprod,
    Mainnet,
}
```

### 5.2 Constraint enforcement

Role uniqueness (at most one tool per role, except Infrastructure) is enforced
at the CLI level: interactive mode only allows selecting one tool per role, and
one-shot mode rejects duplicate role flags before constructing a `Selection`.
There is no separate validation module — a `Selection` that exists is valid by
construction.


## 6. Scaffolding Pipeline

The scaffolder runs in four phases. Each phase is independent and testable.

```
┌─────────────┐    ┌─────────────┐    ┌─────────────┐    ┌─────────────┐
│  1. Context  │───▶│  2. Plan    │───▶│  3. Render  │───▶│  4. Write   │
│   Building   │    │             │    │             │    │             │
│              │    │ Collect all │    │ Run Jinja   │    │ Write to    │
│ Selection +  │    │ files to    │    │ on each     │    │ disk (or    │
│ Registry ──▶ │    │ emit, in    │    │ template    │    │ dry-run     │
│ TemplateCtx  │    │ order       │    │ file        │    │ print)      │
└─────────────┘    └─────────────┘    └─────────────┘    └─────────────┘
```

### Phase 1: Context Building (`scaffold/context.rs`)

Builds the template context — a structured object available to all templates.

```rust
pub struct TemplateContext {
    pub project_name: String,
    pub network: String,

    // Which roles are active (for conditional sections in base templates)
    pub has_on_chain: bool,
    pub has_off_chain: bool,
    pub has_infra: bool,
    pub has_testing: bool,

    // Per-role info (tool name, language, etc.)
    pub on_chain: Option<RoleContext>,
    pub off_chain: Option<RoleContext>,
    pub infra_tools: Vec<RoleContext>,   // Infra allows multiple tools
    pub testing: Option<RoleContext>,

    // Contract constants (so templates don't hardcode paths)
    pub blueprint_path: String,         // "blueprint/plutus.json"
    pub env_vars: HashMap<String, String>,

    // Options
    pub nix: bool,
}

pub struct RoleContext {
    pub tool_id: String,
    pub tool_name: String,
    pub language: String,
    pub dir: String,                    // "on-chain", "off-chain", etc.
}
```

### Phase 2: Planning (`scaffold/planner.rs`)

Collects the full list of files to emit, in order. This is where `--dry-run`
short-circuits — it prints the plan and exits.

```rust
pub struct FilePlan {
    pub entries: Vec<FileEntry>,
}

pub struct FileEntry {
    pub dest: PathBuf,              // Relative to project root
    pub source: TemplateSource,     // Where the content comes from
    pub render: bool,               // Whether to run through MiniJinja
}

pub enum TemplateSource {
    Base(String),                   // From templates/_base/
    Role(String),                   // From templates/<tool>/<role>/
    Optional(String),               // From templates/_nix/
}

pub fn plan(selection: &Selection, registry: &Registry) -> FilePlan {
    let mut entries = vec![];

    // 1. Base layer: Justfile, README, .gitignore, .env, blueprint/.gitkeep
    // 2. For each role assignment: read manifest.toml, add all files
    // 3. If nix: add _nix/ files

    FilePlan { entries }
}
```

### Phase 3: Rendering (`scaffold/renderer.rs`)

Runs MiniJinja on each file that needs rendering.

```rust
pub fn render(plan: &FilePlan, context: &TemplateContext) -> Result<Vec<RenderedFile>> {
    let env = minijinja::Environment::new();
    // Load all template sources into the environment.
    // For each entry where render == true, render with context.
    // For each entry where render == false, pass through as-is.
}
```

### Phase 4: Write

Writes rendered files to disk. Simple — create directories, write files,
make Justfiles executable. This phase is the only one with side effects.


## 7. CLI Flow

### 7.1 Interactive mode (`cli/interactive.rs`)

```
$ cardano-init

  Welcome to cardano-init! Let's set up your Cardano protocol project.

  A Cardano protocol typically has up to four components:
  • On-chain:       Smart contract logic (validators) that runs on the ledger
  • Off-chain:      Code that builds and submits transactions
  • Infrastructure: Indexers and services that read chain data
  • Testing:        Frameworks for testing your contracts locally

? Which components do you need? (space to select, enter to confirm)
  ▸ [x] On-chain
    [x] Off-chain
    [ ] Infrastructure
    [x] Testing

? Choose a tool for on-chain:
    Aiken — Modern smart contract language, Rust/Gleam-inspired syntax
  ▸ Plinth — Write validators in Haskell using the Plutus libraries
    Scalus — Write validators in Scala

? Choose a tool for off-chain:
  ▸ MeshJS — TypeScript SDK, works with any JS runtime
    Lucid Evolution — Lightweight TypeScript transaction builder
    Tx3 — Declarative transaction builder
    Scalus — Scala-based transaction building

? Choose a tool for testing:
  ▸ Scalus — Property-based testing with local validator execution
    ...

? Project name: my-protocol
? Target network: preview
? Set up Nix for dependency management? No

  ┌─────────────────────────────────────────┐
  │  Summary                                │
  │                                         │
  │  Project:  my-protocol                  │
  │  On-chain: Plinth (Haskell)             │
  │  Off-chain: MeshJS (TypeScript)         │
  │  Testing:  Scalus (Scala)               │
  │  Network:  preview                      │
  │                                         │
  │  Files to create:                       │
  │  my-protocol/                           │
  │  ├── Justfile                           │
  │  ├── README.md                          │
  │  ├── .env                               │
  │  ├── blueprint/                         │
  │  ├── on-chain/   (Plinth)              │
  │  ├── off-chain/  (MeshJS)             │
  │  └── test/       (Scalus)             │
  └─────────────────────────────────────────┘

? Generate project? Yes

  ✔ Created my-protocol/
  ✔ Scaffolded on-chain (Plinth)
  ✔ Scaffolded off-chain (MeshJS)
  ✔ Scaffolded testing (Scalus)
  ✔ Done!

  Next steps:
    cd my-protocol
    just build
```

### 7.2 One-shot mode

```
$ cardano-init \
    --name my-protocol \
    --on-chain aiken \
    --off-chain meshjs \
    --infra kupo \
    --nix

✔ Created my-protocol/ with: Aiken (on-chain), MeshJS (off-chain), Kupo (infra), Nix
```


## 8. Base Template Examples

### Top-level Justfile

```jinja
{# templates/_base/Justfile.jinja #}

# {{ project_name }} — Cardano protocol project
# Generated by cardano-init. Edit freely.

default:
    @just --list

{% if has_on_chain %}
# --- On-chain ({{ on_chain.tool_name }}) ---
build-on-chain:
    just -f {{ on_chain.dir }}/Justfile build

test-on-chain:
    just -f {{ on_chain.dir }}/Justfile test
{% endif %}

{% if has_off_chain %}
# --- Off-chain ({{ off_chain.tool_name }}) ---
build-off-chain:
    just -f {{ off_chain.dir }}/Justfile build

test-off-chain:
    just -f {{ off_chain.dir }}/Justfile test
{% endif %}

{% if has_infra %}
# --- Infrastructure ---
{% for infra_tool in infra_tools %}
dev-{{ infra_tool.tool_id }}:
    just -f {{ infra_tool.dir }}/Justfile dev
{% endfor %}

dev-infra:{% for infra_tool in infra_tools %} dev-{{ infra_tool.tool_id }}{% endfor %}
{% endif %}

{% if has_testing %}
# --- Testing ({{ testing.tool_name }}) ---
test-integration:
    just -f {{ testing.dir }}/Justfile test
{% endif %}

# --- Aggregate tasks ---
build:{% if has_on_chain %} build-on-chain{% endif %}{% if has_off_chain %} build-off-chain{% endif %}
    @echo "Build complete."

test:{% if has_on_chain %} test-on-chain{% endif %}{% if has_off_chain %} test-off-chain{% endif %}{% if has_testing %} test-integration{% endif %}
    @echo "All tests passed."

clean:
{% if has_on_chain %}    just -f {{ on_chain.dir }}/Justfile clean
{% endif %}
{% if has_off_chain %}    just -f {{ off_chain.dir }}/Justfile clean
{% endif %}
{% if has_testing %}    just -f {{ testing.dir }}/Justfile clean
{% endif %}
    rm -f {{ blueprint_path }}
    @echo "Clean complete."
```


## 9. Dependency Graph

The module dependency graph flows strictly downward. No circular dependencies.

```
main.rs
  │
  └── cli/          ──▶  scaffold/   ──▶  registry/
                                      ──▶  contract
```

Key rule: `registry/`, `contract`, and `scaffold/` have **zero dependency on
`cli/`**. They are pure logic over data. This is what makes future extraction
(for WASM, testing, or library use) straightforward.


## 10. Testing Strategy

### Unit tests
- **Registry loading:** parse each TOML file, verify all fields present.
- **Planning:** given a selection, verify the file plan contains exactly the
  expected files in the expected order.
- **Rendering:** given a context and a template, verify output matches expected.

### Integration tests
- **Per-template smoke test:** for each tool-role template, scaffold a project
  with only that role selected. Verify the output directory structure. If CI has
  the tool installed (or via Nix), run `just build` and verify it succeeds.
- **Contract compliance:** for each template, verify it includes a Justfile with
  all required tasks. For on-chain templates, verify `just build` produces
  `blueprint/plutus.json`. These tests enforce the interface contract
  mechanically.
- **Dry-run snapshot tests:** run `--dry-run` for a set of known selections and
  compare output against committed snapshots.
