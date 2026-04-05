# Cardano Protocol Scaffolder — Requirements Specification

**Working title:** `cardano-init` (placeholder)

## 1. Purpose

A CLI tool that scaffolds a new Cardano protocol project from scratch. The user selects which tools they want for each functional role (on-chain, off-chain, infrastructure, testing), and the CLI generates a working monorepo with all the wiring already in place.

The tool targets **newcomers to Cardano development** who may not be familiar with the ecosystem's tooling landscape, while remaining useful to experienced developers who want a fast, opinionated starting point.

## 2. Domain Model

### 2.1 Roles

A role is a functional responsibility within a Cardano protocol project. The initial set of roles is:

| Role               | Description                                                                 |
|--------------------|-----------------------------------------------------------------------------|
| **On-chain**       | Smart contract / validator logic that runs on the Cardano ledger            |
| **Off-chain**      | Transaction building, submission, and interaction with on-chain logic       |
| **Infrastructure** | Indexers, node providers, chain followers, and other backend services       |
| **Testing**        | Contract and integration testing frameworks                                 |

Users may select **one or more roles** — they are not required to fill all four. A project with only an on-chain role is valid (e.g., someone just writing validators). A project with only off-chain is also valid (e.g., interacting with existing contracts).

### 2.2 Tools

A tool is a concrete framework, library, or service that fills one or more roles. Each tool has the following metadata:

- **Name** — human-readable identifier (e.g., "Aiken", "MeshJS")
- **Roles** — which roles this tool *can* fill (one or many)
- **Language(s)** — programming languages involved (e.g., Aiken lang, TypeScript, Haskell, Scala, Rust)
- **Dependencies** — system-level dependencies needed (e.g., Deno, GHC, JVM, Node.js)
- **Description** — a newcomer-friendly explanation of what the tool does and when to choose it

A tool may be selected for a **specific role** even if it supports multiple. The role assignment determines which template is used for scaffolding. For example, Scalus selected for "testing" produces a different scaffold than Scalus selected for "on-chain."

### 2.3 Tool Registry

All tool definitions live in a declarative registry (TOML files) embedded into the binary at compile time. Adding support for a new tool means:

1. Adding a tool definition file with the metadata above.
2. Adding the corresponding template(s) for each role it supports.
3. Recompiling the binary.

No changes to the CLI source code or core logic are required — only new data files.

## 3. CIP-57 Blueprint Integration

The CIP-57 Plutus blueprint is the primary integration seam between on-chain and off-chain components.

### 3.1 The Interface Contract

The central design principle is: **each tool template conforms to a shared interface contract, so any producer works with any consumer without per-pair integration logic.**

The contract defines:

- **Blueprint output path:** on-chain templates MUST produce the CIP-57 blueprint to `blueprint/plutus.json` (relative to the project root). Off-chain and testing templates that need the blueprint MUST read from this path. The CLI does not track which tools produce or consume blueprints — this is a convention that template authors follow.
- **Justfile task names:** every template MUST define its tasks under standardized names (`build`, `test`, `dev`, `clean`). The top-level Justfile composes these by delegating to `just -f on-chain/Justfile build`, etc.
- **Environment variables:** infrastructure templates MUST expose their connection details via a `.env` file at the project root using standardized variable names (e.g., `INDEXER_URL`, `NODE_SOCKET_PATH`). Off-chain and testing templates that need infrastructure MUST read from these variables.

Because every template independently conforms to this contract, composition is guaranteed: **any on-chain template + any off-chain template will interoperate correctly without ever being tested as a pair.** The CLI's integration wiring is generic — it only needs to know which roles are present, not which specific tools fill them.

### 3.2 Implications

- The CLI does NOT need per-pair integration logic. No "Aiken + MeshJS" special case.
- Adding a new tool means conforming to the contract. If the contract is satisfied, the tool works with every existing tool in other roles.
- When an on-chain tool does not natively produce a CIP-57 blueprint, the template must include a conversion or generation step that produces one at the canonical path. The contract is non-negotiable.
- When only one side of the integration is present (e.g., on-chain only), the blueprint is still generated at the canonical path but no consumer wiring is needed.
- The CLI does not track which tools produce or consume blueprints. This is a template-level concern enforced by the contract compliance tests, not by registry metadata.

## 4. Project Structure

### 4.1 Monorepo layout

All generated projects follow a monorepo structure:

```
my-protocol/
├── Justfile                    # Top-level task orchestration
├── README.md                   # Project overview, architecture explanation, getting started
├── .env                        # Infrastructure connection details (if applicable)
├── blueprint/                  # CIP-57 blueprint output (if applicable)
│   └── .gitkeep
├── on-chain/                   # Present only if on-chain role is selected
│   ├── README.md               # Explains the on-chain component and its tool
│   └── ...                     # Tool-specific project structure
├── off-chain/                  # Present only if off-chain role is selected
│   ├── README.md
│   └── ...
├── infra/                      # Present only if infrastructure role is selected
│   ├── kupo/                   # One subdirectory per infra tool
│   │   └── ...
│   ├── ogmios/                 # (if multiple infra tools are selected)
│   │   └── ...
│   └── README.md
├── test/                       # Present only if testing role is selected
│   ├── README.md
│   └── ...
└── nix/                        # Present only if Nix option is selected
    └── flake.nix
```

Only directories for selected roles are created. The top-level README always explains the full architecture, including which roles are present and how they connect.

### 4.2 Task orchestration

Every project includes a `Justfile` with standardized tasks:

- `just build` — builds all components in dependency order
- `just test` — runs all tests
- `just dev` — starts development mode (watch + rebuild as applicable)
- `just clean` — removes build artifacts
- Per-component tasks: `just build-on-chain`, `just build-off-chain`, etc.

The Justfile delegates to each component's native build system (e.g., `aiken build`, `npm run build`, `deno task build`).

### 4.3 Dependency management (optional)

If the user opts in during scaffolding:

- **Nix** — a `flake.nix` is generated that provides a dev shell with all required toolchains (e.g., Aiken CLI, Node.js, GHC). Running `nix develop` drops you into a shell where everything works.
- **Neither** — the README documents all prerequisites and how to install them manually.

## 5. Scaffolded Example Project

### 5.1 Working out of the box

Every generated project must include a **simple but complete example** that compiles, runs, and demonstrates the integration between selected components. This is not boilerplate — it is a functioning mini-protocol.

The example should:

- For on-chain: include a simple validator (e.g., an always-succeeds validator or a basic vesting contract).
- For off-chain: include a script that builds and (optionally) submits a transaction interacting with the validator.
- For infrastructure: include configuration that connects to the selected indexer/provider.
- For testing: include a passing test suite that exercises the validator logic.

If multiple roles are selected, the example must demonstrate them working **together**, not in isolation.

### 5.2 Guided documentation

Each component's README should explain:

- What this component does in the context of the protocol.
- How to modify the example to start building real logic.
- Key concepts the developer needs to understand (with links to external docs).
- Common next steps and patterns.

The top-level README should include an architecture diagram (text-based) showing how components connect.

## 6. CLI Interface

### 6.1 Interactive mode (default)

When run without arguments, the CLI enters an interactive guided flow:

1. **Welcome and explanation** — briefly explain what the CLI does and what a Cardano protocol project looks like.
2. **Role selection** — present the four roles with descriptions. User selects which ones they want (multi-select).
3. **Tool selection per role** — for each selected role, present available tools with descriptions. Highlight recommendations for the user's current selections so far.
4. **Options** — project name, Nix preference, network target (preview/preprod/mainnet).
5. **Summary** — show the full selection and the directory structure that will be created.
6. **Confirmation and generation.**

### 6.2 One-shot mode

```
cardano-init \
  --name my-protocol \
  --on-chain aiken \
  --off-chain meshjs \
  --nix
```

All options specified via flags.

### 6.3 Utility commands

- `cardano-init --dry-run` — show what would be generated without writing to disk.

## 7. Template System

### 7.1 Structure

Templates are organized by tool and role:

```
templates/
├── _base/                      # Shared across all projects
│   ├── Justfile.hbs
│   ├── README.md.hbs
│   └── gitignore.hbs
├── aiken/
│   └── on-chain/               # Aiken in the on-chain role
│       ├── template.toml       # Template metadata
│       ├── aiken.toml.hbs
│       ├── validators/
│       │   └── example.ak
│       └── README.md.hbs
├── meshjs/
│   └── off-chain/
│       ├── template.toml
│       ├── package.json.hbs
│       ├── src/
│       │   └── index.ts
│       └── README.md.hbs
├── scalus/
│   ├── on-chain/               # Scalus can fill multiple roles
│   │   └── ...
│   ├── off-chain/
│   │   └── ...
│   └── testing/
│       └── ...
└── ...
```

### 7.2 Composition

The scaffolder works in phases:

1. **Base layer** — emit the shared skeleton (Justfile, top-level README, gitignore, blueprint directory, optional .env).
2. **Role layers** — for each selected role, emit the tool's template into the corresponding subdirectory.
3. **Assembly** — the base layer templates are parametric: the Justfile, top-level README, and .env are generated based on which roles are present. Because all templates conform to the interface contract (§3.1), this step is generic — it doesn't need to know which specific tools were selected, only which roles exist.
4. **Optional layers** — emit Nix configuration if requested.

Because composition relies on the interface contract rather than per-pair logic, **no tool-specific integration wiring is needed.** The base Justfile template simply iterates over active roles and delegates to their standardized tasks.

Templates use a simple templating engine (e.g., Handlebars, Tera, or MiniJinja) with access to the full project context: project name, selected tools, roles, network, options, etc.

## 8. Extensibility

### 8.1 Adding a new tool

Requires no changes to CLI source code or core logic. Because registry and templates are embedded via rust-embed, the binary must be recompiled after adding new files. Author provides:

1. A tool definition file (`registry/tools/newtool.toml`) with all metadata fields from §2.2.
2. One or more template directories (`templates/newtool/<role>/`), each conforming to the interface contract (§3.1).

Because all templates conform to the same interface contract, **no combinatorial testing is required.** A new on-chain tool only needs to be tested in isolation — if it produces a valid blueprint at the canonical path and exposes the standard Justfile tasks, it is guaranteed to compose correctly with every existing off-chain and testing tool.

### 8.2 Plugin hooks (future)

Reserve the possibility for tools to define lifecycle hooks (e.g., "after scaffolding, run `aiken new`"). Not required for v1 but the architecture should not preclude it.

## 9. Non-Functional Requirements

- **Language:** Rust, using `clap` for argument parsing, `dialoguer` for interactive prompts, `minijinja` for template rendering, and `rust-embed` for bundling registry and templates into the binary.
- **Distribution:** single statically-linked binary. Zero runtime dependencies. Distributed via GitHub releases, cargo install, and optionally Nix flake.
- **Offline capable:** all templates and tool definitions bundled into the binary at compile time. No network calls required for generation (network only needed for optional dependency installation).
- **Test coverage:** the CLI itself should be tested, particularly template composition. Integration tests that scaffold a project and verify it builds for each supported tool template.
- **CI-friendly:** one-shot and config-file modes must work in non-interactive environments.

## 10. Web UI (Future)

A browser-based project configurator (similar to Spring Initializr or TanStack Builder) that provides a visual interface for the same scaffolding workflow. The web UI is a **thin visual layer** — it does not generate projects itself.

### 10.1 Architecture

The web UI reads the tool registry (the same TOML/YAML files used by the CLI) to render the configuration interface and validate selections. Its sole output is a CLI command string that the user copies and runs locally.

No generation logic is duplicated. The Rust CLI is the single source of truth for project scaffolding.

### 10.2 Requirements

- Users configure their project visually: select roles, pick tools, set options.
- The UI shows live validation derived from the tool registry metadata.
- Users can preview the generated file tree (directory names and structure, derived from which roles are selected — not actual file contents).
- The output is a copyable CLI command: `cardano-init --name my-protocol --on-chain aiken --off-chain meshjs --nix`.
- The tool registry is shared between CLI and web UI — adding a new tool definition makes it available in both without any web UI code changes.
