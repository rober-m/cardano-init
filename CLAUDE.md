# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`cardano-init` is a Rust CLI tool that scaffolds Cardano protocol projects. Users select tools for each functional role (on-chain, off-chain, infrastructure, testing) and the CLI generates a working monorepo. Read `REQUIREMENTS.md` and `ARCHITECTURE.md` before making any significant changes — they are authoritative.

## Commands

```bash
# Build
cargo build

# Run
cargo run -- [args]

# Tests
cargo test

# Single test
cargo test <test_name>

# Lint/format
cargo fmt
cargo clippy
```

## Architecture

The codebase is a single Rust crate. The module structure is planned as follows (see `ARCHITECTURE.md` §1):

- `src/cli/` — user interaction only (dialoguer, output formatting). No logic.
- `src/registry/` — deserializes embedded TOML tool definitions into typed structs.
- `src/validation/` — pure validation over a `Selection` + `Registry`. Split into `hard.rs` (blocking errors) and `soft.rs` (warnings).
- `src/scaffold/` — four-phase pipeline: context building → planning → rendering → writing.
- `src/contract.rs` — constants for the interface contract (canonical paths, env var names, Justfile task names).

**Key invariant:** `registry/`, `validation/`, `contract`, and `scaffold/` must have zero dependency on `cli/`. They are pure logic over data.

### Data model

- **`Selection`** — fully resolved user choices (project name, role assignments, network, nix/docker flags).
- **`ToolDef`** — loaded from `registry/tools/<tool>.toml`. Each tool declares which roles it fills and which template path to use.
- **`TemplateContext`** — built from `Selection` + `Registry`; passed to MiniJinja for rendering.
- **Infrastructure role** is the only role that allows multiple tools simultaneously.

### Registry and templates

Tool definitions live in `registry/tools/<tool>.toml`. Templates live in `templates/<tool>/<role>/` with a `manifest.toml` listing files. Both are embedded into the binary at compile time via `build.rs`.

The **interface contract** (`contract.rs`) is what enables any on-chain tool to compose with any off-chain tool without per-pair logic:
- On-chain templates must produce `blueprint/plutus.json` during `build`.
- Infra templates must write standard env vars (e.g., `INDEXER_URL`) to `.env` during `dev`.
- All templates must expose `build`, `test`, `dev`, `clean` Justfile targets.

### Scaffolding pipeline

1. **Context** (`scaffold/context.rs`) — builds `TemplateContext` from selection.
2. **Plan** (`scaffold/planner.rs`) — collects all `FileEntry` items to emit; `--dry-run` exits here.
3. **Render** (`scaffold/renderer.rs`) — runs MiniJinja on each renderable file.
4. **Write** — only phase with disk side effects.

## Dependencies (planned)

- `clap` — argument parsing
- `dialoguer` — interactive prompts
- `minijinja` — template rendering
- `serde` / `toml` — registry deserialization
