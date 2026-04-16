# Adding a Tool to cardano-init

This guide is for contributors who want to integrate their own tooling into `cardano-init`. If your tool fills one of the supported roles — **on-chain**, **off-chain**, **infrastructure**, **testing**, or **formal-methods** — you can add it by providing two things:

1. A registry entry (`registry/tools/<your-tool>.toml`)
2. A template directory (`templates/<your-tool>/<role>/`)

No changes to Rust source code are required. Because all templates and registry files are embedded into the binary at compile time via `rust-embed`, you will need to recompile (`cargo build`) after adding your files.

---

## Step 1: Create the registry entry

Create `registry/tools/<your-tool>.toml`. Use the tool's id as the filename.

```toml
[tool]
id          = "mytool"                  # Unique identifier, used in CLI flags
name        = "MyTool"                  # Human-readable name shown in the UI
description = """\
One to three sentences. What does this tool do, and when should \
someone choose it over alternatives?"""
website     = "https://mytool.dev"
languages   = ["typescript"]           # Languages the generated project uses
system_deps = ["mytool-cli"]           # What the user needs installed on their machine
nix_packages = ["mytool"]              # Nix package name(s), if available (omit if none)

[roles.off-chain]                      # The role this tool fills
template = "mytool/off-chain"          # Path under templates/ for this role
```

A tool can fill multiple roles. Add one `[roles.<role>]` section per role:

```toml
[roles.on-chain]
template = "mytool/on-chain"

[roles.testing]
template = "mytool/testing"
```

### Valid role names

| Role key | Description |
|---|---|
| `on-chain` | Smart contract / validator logic |
| `off-chain` | Transaction building and submission |
| `infrastructure` | Indexers, chain followers, node providers |
| `testing` | Contract and integration testing frameworks |
| `formal-methods` | Specification and automated verification |

---

## Step 2: Create the template directory

Create a directory at the path you declared in the registry entry, e.g. `templates/mytool/off-chain/`. It must contain a `manifest.toml` plus all the files it declares.

### The manifest

`manifest.toml` lists every file your template emits.

```toml
[manifest]
summary = "MyTool off-chain project with a hello-world transaction"

[[files]]
source = "Justfile.jinja"      # Path relative to this template directory
dest   = "Justfile"            # Destination relative to the role directory

[[files]]
source = "src/index.ts"
dest   = "src/index.ts"

[[files]]
source = "package.json.jinja"
dest   = "package.json"
```

Whether a file is rendered through MiniJinja is determined solely by its source filename: files ending in `.jinja` are rendered, all others are copied verbatim. Name a file `foo.jinja` when it contains template variables or conditional blocks; leave it without the extension when it should be copied byte-for-byte (source code, lock files, binary assets, or any file whose content contains `{{` or `{%` as literal syntax).

### Directory layout

```
templates/
└── mytool/
    └── off-chain/
        ├── manifest.toml
        ├── Justfile.jinja
        ├── package.json.jinja
        └── src/
            └── index.ts
```

---

## Step 3: Implement the interface contract

Every template, regardless of role, must conform to the interface contract defined in `src/contract.rs`. This is what allows any on-chain tool to compose with any off-chain tool without per-pair integration logic.

### Mandatory Justfile targets

Every template **must** include a `Justfile` that exposes these four targets:

| Target | Purpose |
|---|---|
| `build` | Compile / package the component |
| `test` | Run the component's tests |
| `dev` | Start development mode (watch, REPL, local daemon) |
| `clean` | Remove build artifacts |

The top-level project Justfile delegates to these by calling `just -f <role>/Justfile build`, so the names are non-negotiable.

### Role-specific requirements

**On-chain tools** must produce the CIP-57 Plutus blueprint during `build`:

```justfile
build:
    mytool build
    cp -f output/plutus.json ../blueprint/plutus.json
```

The off-chain and testing templates read from `../blueprint/plutus.json`. If your tool outputs to a different path, the `build` target must copy it to the canonical location. This is the primary integration seam between on-chain and off-chain.

**Off-chain tools** must handle the case where no blueprint exists yet:

```justfile
build:
    @test -f ../blueprint/plutus.json || echo "Warning: no blueprint found, skipping type generation"
    npm run build
```

Off-chain tools may also read infrastructure connection details from `../.env`:

```typescript
import * as dotenv from "dotenv";
dotenv.config({ path: "../.env" });

const indexerUrl = process.env.INDEXER_URL;
```

**Infrastructure tools** must write their connection details to `../.env` during `dev`. Use the standard variable names:

| Variable | Meaning |
|---|---|
| `INDEXER_URL` | Base URL of the chain indexer |
| `INDEXER_PORT` | Port of the indexer |
| `NODE_SOCKET_PATH` | Path to the local node socket |
| `CARDANO_NETWORK` | Target network (`preview`, `preprod`, or `mainnet`) |

```justfile
dev:
    mytool start &
    echo "INDEXER_URL=http://localhost" >> ../.env
    echo "INDEXER_PORT=1442" >> ../.env
```

**Testing tools** should read both the blueprint and the `.env` if they are present, but must work if neither exists.

**Formal-methods tools** have no extra contract beyond the four Justfile targets.

---

## Template variables

Any file with `render = true` can reference the following variables from the template context:

```jinja
{{ project_name }}          {# e.g. "my-protocol" #}
{{ network }}               {# "preview", "preprod", or "mainnet" #}
{{ blueprint_path }}        {# "blueprint/plutus.json" #}
{{ nix }}                   {# true or false #}

{# Flags for conditional sections #}
{{ has_on_chain }}
{{ has_off_chain }}
{{ has_infra }}
{{ has_testing }}
{{ has_formal_methods }}

{# Per-role context (only safe to access when the corresponding has_* is true) #}
{{ on_chain.tool_id }}      {# e.g. "aiken" #}
{{ on_chain.tool_name }}    {# e.g. "Aiken" #}
{{ on_chain.language }}     {# first entry from the tool's languages list #}
{{ on_chain.dir }}          {# "on-chain" #}

{{ off_chain.tool_id }}
{{ off_chain.tool_name }}
{{ off_chain.language }}
{{ off_chain.dir }}         {# "off-chain" #}

{{ testing.tool_id }}
{{ testing.dir }}           {# "test" #}

{{ formal_methods.tool_id }}
{{ formal_methods.dir }}    {# "formal-methods" #}

{# Infrastructure allows multiple tools simultaneously #}
{% for t in infra_tools %}
{{ t.tool_id }}
{{ t.dir }}                 {# "infra" #}
{% endfor %}

{# Nix packages from all selected tools, deduplicated #}
{% for pkg in nix_packages %}{{ pkg }}{% endfor %}
```

Your template files only need to reference the variables relevant to them. An off-chain template does not need to reference `on_chain.*` at all — the integration is handled by the base-level Justfile template, not by individual role templates.

---

## Worked example: a minimal off-chain tool

`registry/tools/mytool.toml`:

```toml
[tool]
id          = "mytool"
name        = "MyTool"
description = "A TypeScript SDK for building Cardano transactions."
website     = "https://mytool.dev"
languages   = ["typescript"]
system_deps = ["node"]
nix_packages = ["nodejs_20"]

[roles.off-chain]
template = "mytool/off-chain"
```

`templates/mytool/off-chain/manifest.toml`:

```toml
[manifest]
summary = "MyTool off-chain project"

[[files]]
source = "Justfile.jinja"
dest   = "Justfile"

[[files]]
source = "src/index.ts"
dest   = "src/index.ts"
```

`templates/mytool/off-chain/Justfile.jinja`:

```justfile
# Off-chain component (MyTool)
# Part of {{ project_name }}

build:
    npm install
    npm run build

test:
    npm test

dev:
    npm run dev

clean:
    rm -rf dist/ node_modules/
```

`templates/mytool/off-chain/src/index.ts`:

```typescript
// Off-chain entry point — replace with your transaction logic
console.log("Hello from MyTool");
```

After adding these files, run `cargo build` to embed them into the binary, then verify with a dry run:

```bash
cargo run -- --name test-project --off-chain mytool --dry-run
```

---

## Testing your integration

**Dry run** — check the file plan without writing anything:

```bash
cargo run -- --name my-project --off-chain mytool --dry-run
```

**Full scaffold** — generate a real project and inspect it:

```bash
cargo run -- --name my-project --off-chain mytool
ls my-project/off-chain/
```

**Unit tests** — the registry loader test `all_fields_populated` will automatically pick up your tool and verify all required fields are present:

```bash
cargo test
```

For a thorough validation, also scaffold a project with your tool combined with tools from other roles and confirm the top-level `just build` and `just test` targets wire up correctly.
