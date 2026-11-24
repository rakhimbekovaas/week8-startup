import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
} from "@solana/web3.js";
import * as fs from "fs";

const PROGRAM_ID = new PublicKey("EnnLxUs8kh7Ctuvc5DPs3sjn9wcM6k8XEVpjPs1Pi9ta");
const RAYDIUM_CLMM_DEVNET_ID = new PublicKey(
  "DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH"
);
const TOKEN_2022_PROGRAM_ID = new PublicKey(
  "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
);
const MEMO_PROGRAM_ID = new PublicKey(
  "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr"
);
const U64_MAX = new anchor.BN("18446744073709551615");

// При наличии пула в Devnet укажи здесь enabled: true и адреса аккаунтов
const RAYDIUM_DEVNET_CONFIG: {
  enabled: boolean;
  ammConfig?: PublicKey;
  poolState?: PublicKey;
  vault0?: PublicKey;
  vault1?: PublicKey;
  observationState?: PublicKey;
  remainingAccounts?: anchor.web3.AccountMeta[];
} = {
  enabled: false,
};

function loadKeypair(filePath: string): Keypair {
  const raw = JSON.parse(fs.readFileSync(filePath, "utf8"));
  return Keypair.fromSecretKey(new Uint8Array(raw));
}

async function expectErrorContains(
  promise: Promise<any>,
  expectedSubstrings: string[],
  label: string
) {
  try {
    await promise;
    throw new Error(`Ожидалась ошибка (${label}), но транзакция прошла`);
  } catch (err: any) {
    const text = JSON.stringify(err) + " " + (err?.message || "");
    const ok = expectedSubstrings.some((s) => text.includes(s));
    if (!ok) {
      throw new Error(
        `Неожиданная ошибка для "${label}": ${err?.message || text}`
      );
    }
    console.log(`OK (${label})`);
  }
}

async function main() {
  const connection = new anchor.web3.Connection(
    process.env.ANCHOR_PROVIDER_URL || "https://api.devnet.solana.com",
    "confirmed"
  );
  const walletPath =
    process.env.ANCHOR_WALLET || `${process.env.HOME}/.config/solana/id.json`;
  const adminKeypair = loadKeypair(walletPath);
  const provider = new anchor.AnchorProvider(
    connection,
    new anchor.Wallet(adminKeypair),
    { commitment: "confirmed" }
  );
  anchor.setProvider(provider);

  const idl = JSON.parse(
    fs.readFileSync("target/idl/simple_vault.json", "utf8")
  ) as anchor.Idl;
  const program = new Program(idl, provider);
  const methods: any = program.methods;
  const accountsNs: any = program.account;

  const userKeypair = Keypair.generate();
  const solTx = await connection.sendTransaction(
    new anchor.web3.Transaction().add(
      SystemProgram.transfer({
        fromPubkey: adminKeypair.publicKey,
        toPubkey: userKeypair.publicKey,
        lamports: 0.05 * anchor.web3.LAMPORTS_PER_SOL,
      })
    ),
    [adminKeypair]
  );
  await connection.confirmTransaction(solTx, "confirmed");

  const tokenMintA = await createMint(
    connection,
    adminKeypair,
    adminKeypair.publicKey,
    null,
    6
  );
  const tokenMintB = await createMint(
    connection,
    adminKeypair,
    adminKeypair.publicKey,
    null,
    6
  );

  const userTokenAccountA = await getOrCreateAssociatedTokenAccount(
    connection,
    adminKeypair,
    tokenMintA,
    userKeypair.publicKey
  );
  await mintTo(
    connection,
    adminKeypair,
    tokenMintA,
    userTokenAccountA.address,
    adminKeypair,
    1_000_000_000
  );

  const [vaultStatePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), tokenMintA.toBuffer()],
    PROGRAM_ID
  );
  const [vaultTokenAccountAPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault_tokens"), tokenMintA.toBuffer()],
    PROGRAM_ID
  );
  const [shareMintPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("share_mint"), tokenMintA.toBuffer()],
    PROGRAM_ID
  );
  const [vaultTokenAccountBPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault_tokens_b"), tokenMintA.toBuffer()],
    PROGRAM_ID
  );

  await methods
    .initializeVault()
    .accounts({
      admin: adminKeypair.publicKey,
      tokenMint: tokenMintA,
      vaultTokenAccount: vaultTokenAccountAPda,
      shareMint: shareMintPda,
      vaultState: vaultStatePda,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .rpc();
  console.log("1. initialize_vault ок");

  const nonAdminProvider = new anchor.AnchorProvider(
    connection,
    new anchor.Wallet(userKeypair),
    { commitment: "confirmed" }
  );
  const nonAdminMethods: any = new Program(idl, nonAdminProvider).methods;

  await expectErrorContains(
    nonAdminMethods
      .addPairToken()
      .accounts({
        admin: userKeypair.publicKey,
        vaultState: vaultStatePda,
        tokenMintB,
        vaultTokenAccountB: vaultTokenAccountBPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc(),
    ["Unauthorized"],
    "add_pair_token от не-админа"
  );

  await methods
    .addPairToken()
    .accounts({
      admin: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      tokenMintB,
      vaultTokenAccountB: vaultTokenAccountBPda,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .rpc();
  console.log("2. add_pair_token ок");

  const tokenMintB2 = await createMint(
    connection,
    adminKeypair,
    adminKeypair.publicKey,
    null,
    6
  );
  await expectErrorContains(
    methods
      .addPairToken()
      .accounts({
        admin: adminKeypair.publicKey,
        vaultState: vaultStatePda,
        tokenMintB: tokenMintB2,
        vaultTokenAccountB: vaultTokenAccountBPda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
        rent: SYSVAR_RENT_PUBKEY,
      })
      .rpc(),
    ["PairAlreadySet", "already in use", "0x0"],
    "повторный add_pair_token"
  );

  const [userCardPda] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("user_card"),
      vaultStatePda.toBuffer(),
      userKeypair.publicKey.toBuffer(),
    ],
    PROGRAM_ID
  );
  await nonAdminMethods
    .createUserCard()
    .accounts({
      user: userKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      systemProgram: SystemProgram.programId,
    })
    .rpc();

  const userShareAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    adminKeypair,
    shareMintPda,
    userKeypair.publicKey
  );

  await nonAdminMethods
    .deposit(new anchor.BN(1_000_000_000))
    .accounts({
      user: userKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      vaultTokenAccount: vaultTokenAccountAPda,
      shareMint: shareMintPda,
      userTokenAccount: userTokenAccountA.address,
      userShareAccount: userShareAccount.address,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();
  console.log("3. deposit 1000 A ок");

  if (!RAYDIUM_DEVNET_CONFIG.enabled) {
    console.log("Пропуск CPI Raydium: включи RAYDIUM_DEVNET_CONFIG.enabled после создания пула в Devnet.");
    return;
  }

  await expectErrorContains(
    methods
      .swap(new anchor.BN(600_000_000), new anchor.BN(0), true)
      .accounts({
        admin: adminKeypair.publicKey,
        vaultState: vaultStatePda,
        vaultTokenAccount: vaultTokenAccountAPda,
        vaultTokenAccountB: vaultTokenAccountBPda,
        ammConfig: RAYDIUM_DEVNET_CONFIG.ammConfig,
        poolState: RAYDIUM_DEVNET_CONFIG.poolState,
        inputVault: RAYDIUM_DEVNET_CONFIG.vault0,
        outputVault: RAYDIUM_DEVNET_CONFIG.vault1,
        observationState: RAYDIUM_DEVNET_CONFIG.observationState,
        inputVaultMint: tokenMintA,
        outputVaultMint: tokenMintB,
        raydiumProgram: RAYDIUM_CLMM_DEVNET_ID,
        tokenProgram: TOKEN_PROGRAM_ID,
        tokenProgram2022: TOKEN_2022_PROGRAM_ID,
        memoProgram: MEMO_PROGRAM_ID,
      })
      .remainingAccounts(RAYDIUM_DEVNET_CONFIG.remainingAccounts || [])
      .rpc(),
    ["SwapExceedsHalfVaultA"],
    "swap > 50% TOKEN_A"
  );

  await methods
    .swap(new anchor.BN(300_000_000), new anchor.BN(0), true)
    .accounts({
      admin: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      vaultTokenAccount: vaultTokenAccountAPda,
      vaultTokenAccountB: vaultTokenAccountBPda,
      ammConfig: RAYDIUM_DEVNET_CONFIG.ammConfig,
      poolState: RAYDIUM_DEVNET_CONFIG.poolState,
      inputVault: RAYDIUM_DEVNET_CONFIG.vault0,
      outputVault: RAYDIUM_DEVNET_CONFIG.vault1,
      observationState: RAYDIUM_DEVNET_CONFIG.observationState,
      inputVaultMint: tokenMintA,
      outputVaultMint: tokenMintB,
      raydiumProgram: RAYDIUM_CLMM_DEVNET_ID,
      tokenProgram: TOKEN_PROGRAM_ID,
      tokenProgram2022: TOKEN_2022_PROGRAM_ID,
      memoProgram: MEMO_PROGRAM_ID,
    })
    .remainingAccounts(RAYDIUM_DEVNET_CONFIG.remainingAccounts || [])
    .rpc();
  console.log("4. swap 300 A -> B ок");

  await expectErrorContains(
    nonAdminMethods
      .withdraw(new anchor.BN(100_000_000))
      .accounts({
        user: userKeypair.publicKey,
        vaultState: vaultStatePda,
        userCard: userCardPda,
        vaultTokenAccount: vaultTokenAccountAPda,
        shareMint: shareMintPda,
        userTokenAccount: userTokenAccountA.address,
        userShareAccount: userShareAccount.address,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc(),
    ["InMarket"],
    "withdraw при in_market = true"
  );

  const syncIx = await methods
    .syncVault()
    .accounts({
      admin: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      vaultTokenAccount: vaultTokenAccountAPda,
      vaultTokenAccountB: vaultTokenAccountBPda,
    })
    .instruction();

  await methods
    .swap(U64_MAX, new anchor.BN(0), false)
    .accounts({
      admin: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      vaultTokenAccount: vaultTokenAccountAPda,
      vaultTokenAccountB: vaultTokenAccountBPda,
      ammConfig: RAYDIUM_DEVNET_CONFIG.ammConfig,
      poolState: RAYDIUM_DEVNET_CONFIG.poolState,
      inputVault: RAYDIUM_DEVNET_CONFIG.vault1,
      outputVault: RAYDIUM_DEVNET_CONFIG.vault0,
      observationState: RAYDIUM_DEVNET_CONFIG.observationState,
      inputVaultMint: tokenMintB,
      outputVaultMint: tokenMintA,
      raydiumProgram: RAYDIUM_CLMM_DEVNET_ID,
      tokenProgram: TOKEN_PROGRAM_ID,
      tokenProgram2022: TOKEN_2022_PROGRAM_ID,
      memoProgram: MEMO_PROGRAM_ID,
    })
    .remainingAccounts(RAYDIUM_DEVNET_CONFIG.remainingAccounts || [])
    .postInstructions([syncIx])
    .rpc();
  console.log("8. swap(весь B -> A) + sync_vault ок");

  const vaultStateAfterSync: any = await accountsNs.vaultState.fetch(vaultStatePda);
  console.log("inMarket:", vaultStateAfterSync.inMarket, "totalTokens:", vaultStateAfterSync.totalTokens.toString());

  const cardData: any = await accountsNs.userCard.fetch(userCardPda);
  await nonAdminMethods
    .withdraw(cardData.shares)
    .accounts({
      user: userKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      vaultTokenAccount: vaultTokenAccountAPda,
      shareMint: shareMintPda,
      userTokenAccount: userTokenAccountA.address,
      userShareAccount: userShareAccount.address,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();

  const finalUserTokenAcc = await getAccount(connection, userTokenAccountA.address);
  console.log("9. withdraw всех shares ок, баланс A у user:", finalUserTokenAcc.amount.toString());
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});