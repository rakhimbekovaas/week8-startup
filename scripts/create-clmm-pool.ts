import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import Decimal from "decimal.js";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
} from "@solana/spl-token";
import { Raydium, TxVersion } from "@raydium-io/raydium-sdk-v2";

const RAYDIUM_CLMM_DEVNET = new PublicKey(
  "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH"
);
const DECIMALS = 6;
const MINT_AMOUNT = 1_000_000 * 10 ** DECIMALS;
const TARGET_TICK_SPACING = 60;
const TARGET_FEE_RATE = 2500; // 0.25%
const INITIAL_PRICE = new Decimal(1);

function loadKeypair(): Keypair {
  const walletPath =
    process.env.ANCHOR_WALLET ||
    path.join(os.homedir(), ".config", "solana", "id.json");
  const resolved = walletPath.startsWith("~")
    ? path.join(os.homedir(), walletPath.slice(1))
    : walletPath;
  const secret = JSON.parse(fs.readFileSync(resolved, "utf-8"));
  return Keypair.fromSecretKey(Uint8Array.from(secret));
}

async function main() {
  const rpcUrl = process.env.DEVNET_RPC || "https://api.devnet.solana.com";
  const connection = new Connection(rpcUrl, "confirmed");
  const owner = loadKeypair();

  console.log("[1/3] Создаю тестовые токены...");
  const tokenAMint = await createMint(
    connection,
    owner,
    owner.publicKey,
    null,
    DECIMALS
  );
  const tokenBMint = await createMint(
    connection,
    owner,
    owner.publicKey,
    null,
    DECIMALS
  );

  const ataA = await getOrCreateAssociatedTokenAccount(
    connection,
    owner,
    tokenAMint,
    owner.publicKey
  );
  const ataB = await getOrCreateAssociatedTokenAccount(
    connection,
    owner,
    tokenBMint,
    owner.publicKey
  );

  await mintTo(connection, owner, tokenAMint, ataA.address, owner, MINT_AMOUNT);
  await mintTo(connection, owner, tokenBMint, ataB.address, owner, MINT_AMOUNT);

  console.log(`  TOKEN_A: ${tokenAMint.toBase58()}`);
  console.log(`  TOKEN_B: ${tokenBMint.toBase58()}`);

  console.log("[2/3] Создаю CLMM-пул на Raydium devnet...");
  const raydium = await Raydium.load({
    owner,
    connection,
    cluster: "devnet",
    disableFeatureCheck: true,
    disableLoadToken: true,
    blockhashCommitment: "finalized",
  });

  const configs = await raydium.api.getClmmConfigs();
  const matchedConfig =
    configs.find(
      (c: any) =>
        c.tickSpacing === TARGET_TICK_SPACING &&
        (c.tradeFeeRate === TARGET_FEE_RATE || c.feeRate === TARGET_FEE_RATE)
    ) || configs.find((c: any) => c.tickSpacing === TARGET_TICK_SPACING);

  if (!matchedConfig) {
    throw new Error(
      `Не найден CLMM конфиг с tickSpacing=${TARGET_TICK_SPACING}`
    );
  }

  const [mint0, mint1] =
    tokenAMint.toBuffer().compare(tokenBMint.toBuffer()) < 0
      ? [tokenAMint, tokenBMint]
      : [tokenBMint, tokenAMint];

  const mint0Info = await raydium.token.getTokenInfo(mint0.toBase58());
  const mint1Info = await raydium.token.getTokenInfo(mint1.toBase58());

  const ammConfig: any = {
    ...matchedConfig,
    id: new PublicKey(matchedConfig.id),
    fundOwner: "",
  };

  const { execute, extInfo } = await raydium.clmm.createPool({
    programId: RAYDIUM_CLMM_DEVNET,
    mint1: mint0Info,
    mint2: mint1Info,
    ammConfig,
    initialPrice: INITIAL_PRICE,
    txVersion: TxVersion.V0,
  });

  const { txId } = await execute({ sendAndConfirm: true });
  const extAny = extInfo as any;
  const rawPool = extAny?.id ?? extAny?.poolId ?? extAny?.address ?? "";
  const poolId =
    typeof rawPool === "string"
      ? rawPool
      : rawPool?.toBase58?.() ?? rawPool?.toString?.() ?? "";

  console.log(`  Pool: ${poolId}`);
  console.log(`  Tx  : https://explorer.solana.com/tx/${txId}?cluster=devnet`);

  console.log("[3/3] Сохраняю devnet-pool.json");
  const out = {
    cluster: "devnet",
    programId: RAYDIUM_CLMM_DEVNET.toBase58(),
    wallet: owner.publicKey.toBase58(),
    tokenA: tokenAMint.toBase58(),
    tokenB: tokenBMint.toBase58(),
    mint0: mint0.toBase58(),
    mint1: mint1.toBase58(),
    poolId,
    ammConfigId: matchedConfig.id,
    tickSpacing: matchedConfig.tickSpacing,
    initialPrice: INITIAL_PRICE.toString(),
    txId,
  };

  fs.writeFileSync("devnet-pool.json", JSON.stringify(out, null, 2) + "\n");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});