// Integration smoke test for the local Yaci DevKit devnet.
//
// Standalone by design: no npm install, no dependencies — just Node's built-in
// fetch (Node 18+). It exercises the seam the rest of the project relies on:
// the Blockfrost-compatible API that off-chain components talk to.
//
// Behaviour (per the interface contract's graceful-degradation rule for devnet
// components — it must work even when neither the devnet nor a blueprint exists):
//
//   • No INDEXER_URL in ../.env, or the devnet is unreachable
//        → print how to start it and PASS (exit 0). Keeps `just test` green in
//          CI that has no Docker/devnet.
//   • Devnet reachable but serving wrong/blank data
//        → FAIL (exit 1). That is a real, actionable problem.
//   • Devnet reachable and healthy
//        → assert protocol parameters look sane, report the bundled blueprint
//          (if on-chain has been built), and PASS.
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// This file lives at test/; the project root is one level up.
const projectRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const envPath = resolve(projectRoot, ".env");
const blueprintPath = resolve(projectRoot, "blueprint", "plutus.json");

/** Minimal .env reader: `KEY=value` lines, last wins, no interpolation. */
function readEnv(path) {
  const env = {};
  if (!existsSync(path)) return env;
  for (const line of readFileSync(path, "utf-8").split("\n")) {
    const match = /^([A-Za-z_][A-Za-z0-9_]*)=(.*)$/.exec(line);
    if (match) env[match[1]] = match[2].trim();
  }
  return env;
}

function pass(msg) {
  console.log(`✓ ${msg}`);
}

function skip(msg) {
  console.log(`• ${msg}`);
}

function fail(msg) {
  console.error(`✗ ${msg}`);
  process.exit(1);
}

// `just test` (scripts/devnet-test.sh) starts an ephemeral devnet and passes
// INDEXER_URL via the environment; otherwise we read the shared ../.env (e.g. a
// persistent devnet started with `just dev`). The environment wins.
const fileEnv = readEnv(envPath);
const indexerUrl = (process.env.INDEXER_URL ?? fileEnv.INDEXER_URL ?? "").trim();

// When set, a devnet is mandatory: every graceful skip below becomes a hard
// failure. The end-to-end CI smoke test sets this so a pass proves the seam
// actually ran (see scripts/devnet-test.sh).
const requireDevnet = (process.env.CARDANO_INIT_REQUIRE_DEVNET ?? "").trim() !== "";

if (!indexerUrl) {
  if (requireDevnet) {
    fail("No INDEXER_URL, but CARDANO_INIT_REQUIRE_DEVNET is set — a live devnet was required.");
  }
  skip("No INDEXER_URL in ../.env — no local devnet configured.");
  console.log("  Start one with:  just dev   (writes INDEXER_URL into ../.env)");
  console.log("  Skipping the devnet integration test.");
  process.exit(0);
}

// The Blockfrost-compatible base URL is expected to end in a slash, e.g.
// http://localhost:8080/api/v1/ . Tolerate a missing one.
const base = indexerUrl.endsWith("/") ? indexerUrl : `${indexerUrl}/`;

async function getJson(path) {
  const url = `${base}${path}`;
  // Retry a few times: just after a devnet starts, the port can open a moment
  // before the API serves data.
  let lastErr;
  for (let attempt = 1; attempt <= 3; attempt++) {
    try {
      const res = await fetch(url, { signal: AbortSignal.timeout(10_000) });
      if (!res.ok) fail(`GET ${path} returned HTTP ${res.status}`);
      return res.json();
    } catch (err) {
      lastErr = err;
      if (attempt < 3) await new Promise((r) => setTimeout(r, 3_000));
    }
  }
  // Still unreachable: the URL is likely stale (devnet not running). Degrade.
  if (requireDevnet) {
    fail(`Devnet required (CARDANO_INIT_REQUIRE_DEVNET) but ${path} is unreachable (${lastErr?.code ?? lastErr?.name ?? "error"}).`);
  }
  skip(`Devnet path ${path} is not reachable (${lastErr?.code ?? lastErr?.name ?? "error"}).`);
  console.log("  `just test` starts a devnet automatically; or run one with `just dev`.");
  console.log("  Skipping the devnet integration test.");
  process.exit(0);
}

async function main() {
  console.log("Testing Yaci devnet.");

  // 1. Latest block — served from genesis onward (unlike epoch/parameters
  //    endpoints, which 404 until the first epoch completes), so it's the
  //    reliable liveness + correctness check on a fresh devnet. A well-formed
  //    response proves the Yaci Store indexer is serving the chain.
  const block = await getJson("blocks/latest");
  if (typeof block.hash !== "string" || typeof block.slot !== "number") {
    fail(`Latest block missing expected fields: ${JSON.stringify(block).slice(0, 200)}`);
  }
  pass(`Indexer serving the chain (latest block: hash=${block.hash}, height=${block.height}, slot=${block.slot}).`);

  // 2. Blueprint — produced by the on-chain `just build`. Optional: a project
  //    may not have built (or may not have) an on-chain component yet.
  if (existsSync(blueprintPath)) {
    let blueprint;
    try {
      blueprint = JSON.parse(readFileSync(blueprintPath, "utf-8"));
    } catch (err) {
      fail(`../blueprint/plutus.json is not valid JSON: ${err.message}`);
    }
    const validators = Array.isArray(blueprint.validators) ? blueprint.validators : [];
    if (validators.length === 0) {
      fail("../blueprint/plutus.json has no validators.");
    }
    pass(`Blueprint loaded: ${validators.length} validator(s) ready to deploy against the devnet.`);
  } else {
    skip("No ../blueprint/plutus.json yet — build the on-chain component to deploy against the devnet.");
  }

  console.log("\nDevnet integration smoke test passed.");
}

main().catch((err) => {
  fail(err instanceof Error ? err.message : String(err));
});
