import { getBundledValidators, hasBundledBlueprint } from "./contract.js";
import { createGiftCardContractFromEnv, loadEnv, topupOnDevnet } from "./node.js";

// Runnable entry point for `just dev` / `npm start` and the `create` / `redeem`
// commands. It submits real transactions when the blueprint is bundled and the
// required environment variables are present, and degrades to guidance
// otherwise. The blueprint is bundled from ../blueprint/plutus.json before this
// runs (the npm "pre" hooks call scripts/bundle-blueprint.mjs).
//
//   npm start                       # status: blueprint + env readiness
//   npx tsx src/cli.ts create <name> <lovelace>
//   npx tsx src/cli.ts redeem <createTxHash>

function printStatus(): void {
  if (!hasBundledBlueprint()) {
    console.log("No blueprint bundled.");
    console.log(
      "Build the on-chain component first:  just -f ../on-chain/Justfile build",
    );
    return;
  }

  const { giftCard, redeem } = getBundledValidators();
  console.log("Bundled on-chain blueprint:");
  console.log(`  gift_card (mint):  ${giftCard.length} hex chars`);
  console.log(`  redeem    (spend): ${redeem.length} hex chars`);
  console.log("");

  const env = loadEnv();
  if (env.ok) {
    console.log("Environment ready.");
    console.log("Run a transaction:");
    console.log("  npx tsx src/cli.ts create <tokenName> <lovelace>");
    console.log("  npx tsx src/cli.ts redeem <createTxHash>");
  } else {
    console.log("To submit transactions, set the following (see .env.example):");
    for (const key of env.missing) console.log(`  - ${key}`);
  }
}

function requireEnv() {
  if (!hasBundledBlueprint()) {
    throw new Error(
      "No blueprint bundled. Build on-chain first: " +
        "just -f ../on-chain/Justfile build",
    );
  }
  const env = loadEnv();
  if (!env.ok) {
    throw new Error(
      `Missing required configuration: ${env.missing.join(", ")}. ` +
        "See .env.example.",
    );
  }
  return env.env;
}

async function create(tokenName: string, lovelace: string): Promise<void> {
  const env = requireEnv();
  const { contract, wallet, provider } = await createGiftCardContractFromEnv({ env });

  // On a local devnet, top the wallet up from the faucet so a fresh wallet has
  // funds + collateral. Amount is in ADA. No-op (and not needed) on Blockfrost.
  const address = await wallet.getChangeAddress();
  const topped = await topupOnDevnet(provider, address, "10000"); // 10,000 ADA
  if (topped) console.log(`Funded ${address.slice(0, 20)}… from the devnet faucet.`);

  const unsignedTx = await contract.createGiftCard(tokenName, [
    { unit: "lovelace", quantity: lovelace },
  ]);
  const txHash = await contract.signAndSubmit(unsignedTx);

  console.log(`Gift card "${tokenName}" created.`);
  console.log(`  tx: ${txHash}`);
  console.log("Once it is confirmed on chain, redeem it with:");
  console.log(`  npx tsx src/cli.ts redeem ${txHash}`);
}

async function redeem(createTxHash: string): Promise<void> {
  const env = requireEnv();
  const { contract } = await createGiftCardContractFromEnv({ env });

  const giftCardUtxo = await contract.getGiftCardUtxo(createTxHash);
  if (giftCardUtxo === undefined) {
    throw new Error(
      `No gift-card UTxO (output with an inline datum) found in tx ${createTxHash}.`,
    );
  }

  const unsignedTx = await contract.redeemGiftCard(giftCardUtxo);
  const txHash = await contract.signAndSubmit(unsignedTx);

  console.log("Gift card redeemed; assets released to your wallet.");
  console.log(`  tx: ${txHash}`);
}

async function main(): Promise<void> {
  const [command, ...args] = process.argv.slice(2);

  switch (command) {
    case undefined:
    case "status":
      printStatus();
      break;
    case "create":
      if (args.length < 2) {
        throw new Error("usage: create <tokenName> <lovelace>");
      }
      await create(args[0]!, args[1]!);
      break;
    case "redeem":
      if (args.length < 1) {
        throw new Error("usage: redeem <createTxHash>");
      }
      await redeem(args[0]!);
      break;
    default:
      throw new Error(`Unknown command "${command}". Use: status | create | redeem`);
  }
}

main().catch((err) => {
  console.error(err instanceof Error ? err.message : err);
  process.exit(1);
});
