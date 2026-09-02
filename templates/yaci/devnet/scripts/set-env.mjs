// Publishes (or clears) the Yaci devnet connection details in the project's
// shared ../.env, so off-chain/devnet consumers find them at the standard
// keys defined by the cardano-init interface contract.
//
//   node scripts/set-env.mjs            # write the devnet connection details
//   node scripts/set-env.mjs --clear    # blank them out again (used by `clean`)
//
// The write is idempotent: existing keys are replaced in place (no duplicate
// lines accumulate across repeated `just dev` runs); every other line in the
// .env — CARDANO_NETWORK, comments, blanks — is preserved untouched.
import { spawnSync } from "node:child_process";
import {
  closeSync,
  ftruncateSync,
  openSync,
  readFileSync,
  writeSync,
} from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

// This script lives at test/scripts/; the project root is two levels up.
const scriptPath = fileURLToPath(import.meta.url);
const projectRoot = resolve(dirname(scriptPath), "..", "..");
const envPath = resolve(projectRoot, ".env");
const lockPath = `${envPath}.lock`;
const lockHeldArgument = "--cardano-init-env-lock-held";

if (!process.argv.includes(lockHeldArgument)) {
  runWithEnvLock();
}

const clear = process.argv.includes("--clear");

// Standard local Yaci DevKit endpoints (see ./README.md and the devkit docs).
// INDEXER_URL / INDEXER_PORT are the contract's connection keys; YACI_ADMIN_URL
// is an extra key Yaci adds for its faucet/topup admin API.
const SET = {
  INDEXER_URL: "http://localhost:8080/api/v1/",
  INDEXER_PORT: "8080",
  YACI_ADMIN_URL: "http://localhost:10000",
};

// On clear: blank the contract keys; drop the Yaci-specific admin key entirely.
const updates = clear
  ? { INDEXER_URL: "", INDEXER_PORT: "" }
  : SET;
const removeKeys = clear ? new Set(["YACI_ADMIN_URL"]) : new Set();

function hasCode(error, code) {
  return error !== null && typeof error === "object" && error.code === code;
}

function openEnv(path) {
  try {
    return openSync(path, "r+");
  } catch (error) {
    if (!hasCode(error, "ENOENT")) throw error;
    try {
      return openSync(path, "wx+", 0o600);
    } catch (createError) {
      if (!hasCode(createError, "EEXIST")) throw createError;
      return openSync(path, "r+");
    }
  }
}

function writeAll(fd, text, path) {
  const buffer = Buffer.from(text, "utf-8");
  let offset = 0;
  while (offset < buffer.length) {
    const written = writeSync(fd, buffer, offset, buffer.length - offset, offset);
    if (written === 0) throw new Error(`Unable to make progress writing ${path}`);
    offset += written;
  }
}

function runWithEnvLock() {
  const childArgs = [
    lockPath,
    process.execPath,
    scriptPath,
    lockHeldArgument,
    ...process.argv.slice(2),
  ];
  let command;
  let args;
  if (process.platform === "linux") {
    command = "flock";
    args = ["-w", "30", ...childArgs];
  } else if (process.platform === "darwin") {
    command = "lockf";
    args = ["-k", "-t", "30", ...childArgs];
  } else {
    throw new Error(`Yaci DevKit is unsupported on ${process.platform}`);
  }

  const result = spawnSync(command, args, { stdio: "inherit" });
  if (result.error) throw result.error;
  process.exit(result.status ?? 1);
}

const envFd = openEnv(envPath);
try {
  const original = readFileSync(envFd, "utf-8");
  const lines = original.length > 0 ? original.split("\n") : [];

  const pending = new Set(Object.keys(updates));
  const out = [];
  for (const line of lines) {
    const match = /^([A-Za-z_][A-Za-z0-9_]*)=/.exec(line);
    const key = match?.[1];
    if (key && removeKeys.has(key)) continue; // drop this line
    if (key && pending.has(key)) {
      out.push(`${key}=${updates[key]}`);
      pending.delete(key);
    } else {
      out.push(line);
    }
  }
  // Append any keys that weren't already present.
  for (const key of pending) out.push(`${key}=${updates[key]}`);

  // Normalize to a single trailing newline (LF).
  let text = out.join("\n").replace(/\n+$/, "");
  if (text.length > 0) text += "\n";
  ftruncateSync(envFd, 0);
  writeAll(envFd, text, envPath);
} finally {
  closeSync(envFd);
}

if (clear) {
  console.log(`Cleared Yaci connection details from ${envPath}`);
} else {
  console.log(`Wrote Yaci connection details to ${envPath}:`);
  for (const [k, v] of Object.entries(SET)) console.log(`  ${k}=${v}`);
}
