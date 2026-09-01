import {
  builtinByteString,
  BuiltinByteString,
  Integer,
  List,
  mConStr0,
  mConStr1,
  outputReference,
  stringToHex,
} from "@meshsdk/common";
import {
  Asset,
  DEFAULT_V1_COST_MODEL_LIST,
  DEFAULT_V2_COST_MODEL_LIST,
  DEFAULT_V3_COST_MODEL_LIST,
  deserializeDatum,
  IEvaluator,
  IFetcher,
  ISubmitter,
  MeshTxBuilder,
  resolveScriptHash,
  serializePlutusScript,
  UTxO,
} from "@meshsdk/core";
import { applyParamsToScript } from "@meshsdk/core-cst";

import { bundledBlueprint } from "./blueprint.generated.js";

// Core, framework-agnostic bindings for the GiftCard contract. This module has
// no Node-only dependencies (no `fs`), so it is safe to import in a browser
// frontend as well as a backend. The on-chain blueprint is bundled in at build
// time from ../blueprint/plutus.json (see scripts/bundle-blueprint.mjs), so the
// contract always knows its validators — consumers never supply them.

/** Plutus language version the on-chain validators are compiled to. */
export const LANGUAGE_VERSION = "V3" as const;

/**
 * Shape of the CIP-57 blueprint produced by the on-chain `aiken build`. Only
 * the fields this library reads are named; the index signatures tolerate the
 * many other fields a real plutus.json carries (preamble, redeemer, datum, …).
 */
export type Blueprint = {
  validators: { title: string; compiledCode: string; [key: string]: unknown }[];
  [key: string]: unknown;
};

/** The two compiled validators the GiftCard flow needs. */
export type GiftCardValidators = {
  /** Compiled code of the `gift_card` minting policy. */
  giftCard: string;
  /** Compiled code of the `redeem` spending validator. */
  redeem: string;
};

/** A reference to the seed UTxO that parameterises a gift card. */
export type SeedUtxo = {
  txHash: string;
  outputIndex: number;
};

/**
 * The parameterised scripts and derived identifiers for a single gift card.
 * Useful on a frontend to display the policy id / script address without
 * building a transaction.
 */
export type GiftCardScripts = {
  giftCardScript: string;
  policyId: string;
  redeemScript: string;
  redeemAddress: string;
  /** The full asset unit (policy id + hex token name) of the gift-card token. */
  unit: string;
};

// Aiken prefixes validator titles with their module name, e.g.
// "giftcard.gift_card.mint". We match on the "<validator>.<purpose>" suffix so
// the lookup is independent of the module (file) name.
export function findValidator(blueprint: Blueprint, suffix: string): string {
  const validator = blueprint.validators.find(
    (v) => v.title === suffix || v.title.endsWith(`.${suffix}`),
  );
  if (!validator) {
    const known = blueprint.validators.map((v) => v.title).join(", ");
    throw new Error(
      `Validator "${suffix}" not found in blueprint. Available: ${known}`,
    );
  }
  return validator.compiledCode;
}

/** Pull the gift_card (mint) and redeem (spend) validators out of a blueprint. */
export function getGiftCardValidators(blueprint: Blueprint): GiftCardValidators {
  return {
    giftCard: findValidator(blueprint, "gift_card.mint"),
    redeem: findValidator(blueprint, "redeem.spend"),
  };
}

/** The blueprint inlined at build time, or null if none was bundled. */
export { bundledBlueprint };

/** Whether a blueprint was bundled into this build. */
export function hasBundledBlueprint(): boolean {
  return bundledBlueprint !== null;
}

/**
 * The GiftCard validators bundled at build time. Throws if none was bundled —
 * build the on-chain component and run the off-chain `just build`, or pass
 * `validators` to the contract explicitly.
 */
export function getBundledValidators(): GiftCardValidators {
  if (bundledBlueprint === null) {
    throw new Error(
      "No blueprint was bundled into this build. Build the on-chain component " +
        "(its `just build` writes ../blueprint/plutus.json), then run the " +
        "off-chain `just build`; or pass `validators` to GiftCardContract.",
    );
  }
  return getGiftCardValidators(bundledBlueprint);
}

/**
 * Minimal wallet surface the contract needs. Both MeshWallet (backend) and
 * BrowserWallet (frontend) satisfy this structurally, so the library is not
 * tied to a particular wallet implementation.
 */
export interface GiftCardWallet {
  getUtxos(): Promise<UTxO[]>;
  getCollateral(): Promise<UTxO[]>;
  getChangeAddress(): Promise<string>;
  signTx(unsignedTx: string, partialSign?: boolean): Promise<string>;
  submitTx(tx: string): Promise<string>;
}

export type GiftCardContractInput = {
  /** Reads chain state (UTxOs). A provider such as BlockfrostProvider works. */
  fetcher: IFetcher;
  /** Submits signed transactions. A provider such as BlockfrostProvider works. */
  submitter: ISubmitter;
  /**
   * Computes Plutus script execution units for the transaction. Required for
   * the on-chain scripts to balance correctly — providers such as
   * BlockfrostProvider and YaciProvider implement this. Defaults to the
   * `submitter` when it also implements `IEvaluator` (both providers do).
   */
  evaluator?: IEvaluator;
  /** The wallet that funds, signs, and submits. */
  wallet: GiftCardWallet;
  /** 0 for testnets (preview/preprod), 1 for mainnet. */
  networkId: number;
};

export class GiftCardContract {
  private readonly fetcher: IFetcher;
  private readonly submitter: ISubmitter;
  private readonly evaluator?: IEvaluator;
  private readonly wallet: GiftCardWallet;
  private readonly networkId: number;
  private cachedCostModels?: number[][];
  private readonly giftCardCompiledCode: string;
  private readonly redeemCompiledCode: string;

  constructor(input: GiftCardContractInput) {
    this.fetcher = input.fetcher;
    this.submitter = input.submitter;
    // Fall back to the submitter as evaluator (BlockfrostProvider / YaciProvider
    // implement IEvaluator), so Plutus ex-units are computed without extra wiring.
    this.evaluator =
      input.evaluator ??
      (input.submitter != null &&
      typeof (input.submitter as { evaluateTx?: unknown }).evaluateTx === "function"
        ? (input.submitter as unknown as IEvaluator)
        : undefined);
    this.wallet = input.wallet;
    this.networkId = input.networkId;
    const validators = getBundledValidators();
    this.giftCardCompiledCode = validators.giftCard;
    this.redeemCompiledCode = validators.redeem;
  }

  /** A fresh transaction builder. One builder builds one transaction. */
  private async newTxBuilder(): Promise<MeshTxBuilder> {
    const builder = new MeshTxBuilder({
      fetcher: this.fetcher,
      submitter: this.submitter,
      evaluator: this.evaluator,
    });
    builder.setNetwork(await this.costModels());
    return builder;
  }

  /**
   * The Plutus cost models the script-data hash is computed from.
   *
   * These must be the ones the chain actually runs. The node recomputes the
   * hash and rejects a mismatch outright ("script data hash mismatch",
   * Conway UTxO rule 17), so a pinned constant list is only safe while it
   * agrees with the chain. It often does not: a devnet can run a different
   * protocol version than the constants were cut for, and the lists grow as
   * Plutus gains builtins.
   *
   * So ask the provider first, and keep the shipped constants as a fallback
   * for one that cannot answer — MeshJS's YaciProvider throws "Method not
   * implemented" — where pinning at least keeps tx building deterministic.
   * Cached because it is per-chain, not per-transaction.
   */
  private async costModels(): Promise<number[][]> {
    if (this.cachedCostModels !== undefined) {
      return this.cachedCostModels;
    }
    const fetchCostModels = (
      this.fetcher as { fetchCostModels?: () => Promise<number[][]> }
    ).fetchCostModels;
    if (typeof fetchCostModels === "function") {
      try {
        const fetched = await fetchCostModels.call(this.fetcher);
        if (Array.isArray(fetched) && fetched.length >= 3) {
          this.cachedCostModels = fetched;
          return fetched;
        }
      } catch {
        // Provider cannot supply them; fall through to the constants.
      }
    }
    this.cachedCostModels = [
      DEFAULT_V1_COST_MODEL_LIST,
      DEFAULT_V2_COST_MODEL_LIST,
      DEFAULT_V3_COST_MODEL_LIST,
    ];
    return this.cachedCostModels;
  }

  /** Apply the (token name, seed UTxO) parameters to the one-shot mint policy. */
  private giftCardCbor(
    tokenNameHex: string,
    utxoTxHash: string,
    utxoTxId: number,
  ): string {
    return applyParamsToScript(
      this.giftCardCompiledCode,
      [builtinByteString(tokenNameHex), outputReference(utxoTxHash, utxoTxId)],
      "JSON",
    );
  }

  /** Apply the (token name, policy id) parameters to the redeem spend script. */
  private redeemCbor(tokenNameHex: string, policyId: string): string {
    return applyParamsToScript(this.redeemCompiledCode, [tokenNameHex, policyId]);
  }

  private getScriptAddress(scriptCbor: string): string {
    return serializePlutusScript(
      { code: scriptCbor, version: LANGUAGE_VERSION },
      undefined,
      this.networkId,
    ).address;
  }

  /**
   * Derive the parameterised scripts, policy id, and redeem address for a gift
   * card seeded by `seedUtxo`. Pure: no wallet or network access, so it is safe
   * to call from a frontend to preview an address.
   */
  getScripts(tokenName: string, seedUtxo: SeedUtxo): GiftCardScripts {
    const tokenNameHex = stringToHex(tokenName);
    const giftCardScript = this.giftCardCbor(
      tokenNameHex,
      seedUtxo.txHash,
      seedUtxo.outputIndex,
    );
    const policyId = resolveScriptHash(giftCardScript, LANGUAGE_VERSION);
    const redeemScript = this.redeemCbor(tokenNameHex, policyId);
    return {
      giftCardScript,
      policyId,
      redeemScript,
      redeemAddress: this.getScriptAddress(redeemScript),
      unit: policyId + tokenNameHex,
    };
  }

  private async getWalletInfoForTx(): Promise<{
    utxos: UTxO[];
    walletAddress: string;
    collateral: UTxO;
  }> {
    const utxos = await this.wallet.getUtxos();
    const collateral = (await this.wallet.getCollateral())[0];
    const walletAddress = await this.wallet.getChangeAddress();
    if (utxos.length === 0) throw new Error("No UTxOs found in wallet");
    if (collateral === undefined)
      throw new Error("No collateral UTxO found in wallet");
    return { utxos, walletAddress, collateral };
  }

  /**
   * Build a transaction that mints a unique gift-card token and locks
   * `giftValue` at the redeem script address. Returns the UNSIGNED transaction
   * hex — sign and submit it with `signAndSubmit`, a browser wallet, or your
   * own signer.
   */
  createGiftCard = async (
    tokenName: string,
    giftValue: Asset[],
  ): Promise<string> => {
    const { utxos, walletAddress, collateral } = await this.getWalletInfoForTx();
    const tokenNameHex = stringToHex(tokenName);
    const firstUtxo = utxos[0];
    if (firstUtxo === undefined) throw new Error("No UTxOs available");
    const remainingUtxos = utxos.slice(1);

    const { giftCardScript, policyId, redeemAddress, unit } = this.getScripts(
      tokenName,
      firstUtxo.input,
    );

    const tx = await (await this.newTxBuilder())
      .txIn(
        firstUtxo.input.txHash,
        firstUtxo.input.outputIndex,
        firstUtxo.output.amount,
        firstUtxo.output.address,
      )
      .mintPlutusScript(LANGUAGE_VERSION)
      .mint("1", policyId, tokenNameHex)
      .mintingScript(giftCardScript)
      .mintRedeemerValue(mConStr0([]))
      .txOut(redeemAddress, [...giftValue, { unit, quantity: "1" }])
      .txOutInlineDatumValue([
        firstUtxo.input.txHash,
        firstUtxo.input.outputIndex,
        tokenNameHex,
      ])
      .changeAddress(walletAddress)
      .txInCollateral(
        collateral.input.txHash,
        collateral.input.outputIndex,
        collateral.output.amount,
        collateral.output.address,
      )
      .selectUtxosFrom(remainingUtxos)
      .complete();

    return tx;
  };

  /**
   * Build a transaction that burns a gift-card token and releases the locked
   * assets to the redeemer. `giftCardUtxo` is the UTxO sitting at the redeem
   * script address (see `getGiftCardUtxo`). Returns the UNSIGNED transaction hex.
   */
  redeemGiftCard = async (giftCardUtxo: UTxO): Promise<string> => {
    const { utxos, walletAddress, collateral } = await this.getWalletInfoForTx();

    const inlineDatum = deserializeDatum<List>(
      giftCardUtxo.output.plutusData!,
    ).list;
    const paramTxHash = (inlineDatum[0] as BuiltinByteString).bytes;
    const paramTxId = (inlineDatum[1] as Integer).int as number;
    const tokenNameHex = (inlineDatum[2] as BuiltinByteString).bytes;

    const giftCardScript = this.giftCardCbor(tokenNameHex, paramTxHash, paramTxId);
    const policyId = resolveScriptHash(giftCardScript, LANGUAGE_VERSION);
    const redeemScript = this.redeemCbor(tokenNameHex, policyId);

    const tx = await (await this.newTxBuilder())
      .spendingPlutusScript(LANGUAGE_VERSION)
      .txIn(
        giftCardUtxo.input.txHash,
        giftCardUtxo.input.outputIndex,
        giftCardUtxo.output.amount,
        giftCardUtxo.output.address,
      )
      .spendingReferenceTxInInlineDatumPresent()
      .spendingReferenceTxInRedeemerValue("")
      .txInScript(redeemScript)
      .mintPlutusScript(LANGUAGE_VERSION)
      .mint("-1", policyId, tokenNameHex)
      .mintingScript(giftCardScript)
      .mintRedeemerValue(mConStr1([]))
      .changeAddress(walletAddress)
      .txInCollateral(
        collateral.input.txHash,
        collateral.input.outputIndex,
        collateral.output.amount,
        collateral.output.address,
      )
      .selectUtxosFrom(utxos)
      .complete();

    return tx;
  };

  /**
   * Find the gift-card UTxO produced by a `createGiftCard` transaction: the
   * output carrying an inline datum (the one locked at the redeem address).
   */
  getGiftCardUtxo = async (txHash: string): Promise<UTxO | undefined> => {
    const utxos = await this.fetcher.fetchUTxOs(txHash);
    return utxos.find((u) => u.output.plutusData !== undefined);
  };

  /** Sign an unsigned transaction with the wallet and submit it. Returns the tx hash. */
  signAndSubmit = async (unsignedTx: string): Promise<string> => {
    const signedTx = await this.wallet.signTx(unsignedTx);
    return this.submitter.submitTx(signedTx);
  };
}
