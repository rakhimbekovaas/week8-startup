import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
  createMint,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { Keypair, PublicKey, SystemProgram, SYSVAR_RENT_PUBKEY } from "@solana/web3.js";
import * as fs from "fs";

const PROGRAM_ID = new PublicKey("EnnLxUs8kh7Ctuvc5DPs3sjn9wcM6k8XEVpjPs1Pi9ta");

async function main() {
  const connection = new anchor.web3.Connection(
    "https://api.devnet.solana.com",
    "confirmed"
  );
  const walletPath =
    process.env.ANCHOR_WALLET || `${process.env.HOME}/.config/solana/id.json`;
  const secretKey = JSON.parse(fs.readFileSync(walletPath, "utf8"));
  const adminKeypair = Keypair.fromSecretKey(new Uint8Array(secretKey));
  const wallet = new anchor.Wallet(adminKeypair);

  const provider = new anchor.AnchorProvider(connection, wallet, {
    commitment: "confirmed",
  });
  anchor.setProvider(provider);

  const idlPath = "target/idl/simple_vault.json";
  if (!fs.existsSync(idlPath)) {
    throw new Error(
      `Не найден ${idlPath}. Сначала сгенерируй IDL или проверь путь к файлу IDL.`
    );
  }
  const idl = JSON.parse(fs.readFileSync(idlPath, "utf8")) as anchor.Idl;
  const program = new Program(idl, provider);
  const methods: any = program.methods;

  console.log("Admin / User:", adminKeypair.publicKey.toBase58());
  console.log("Program ID:", PROGRAM_ID.toBase58());

  const tokenMint = await createMint(
    connection,
    adminKeypair,
    adminKeypair.publicKey,
    null,
    6
  );
  console.log("Token Mint A:", tokenMint.toBase58());

  const tokenMintB = await createMint(
    connection,
    adminKeypair,
    adminKeypair.publicKey,
    null,
    6
  );
  console.log("Token Mint B:", tokenMintB.toBase58());

  const userTokenAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    adminKeypair,
    tokenMint,
    adminKeypair.publicKey
  );
  await mintTo(
    connection,
    adminKeypair,
    tokenMint,
    userTokenAccount.address,
    adminKeypair,
    1_000_000_000
  );
  console.log("Minted 1000 tokens to:", userTokenAccount.address.toBase58());

  const [vaultStatePda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault"), tokenMint.toBuffer()],
    PROGRAM_ID
  );
  const [vaultTokenAccountPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("vault_tokens"), tokenMint.toBuffer()],
    PROGRAM_ID
  );
  const [shareMintPda] = PublicKey.findProgramAddressSync(
    [Buffer.from("share_mint"), tokenMint.toBuffer()],
    PROGRAM_ID
  );

  const initTx = await methods
    .initializeVault()
    .accounts({
      admin: adminKeypair.publicKey,
      tokenMint,
      vaultState: vaultStatePda,
      vaultTokenAccount: vaultTokenAccountPda,
      shareMint: shareMintPda,
      tokenProgram: TOKEN_PROGRAM_ID,
      systemProgram: SystemProgram.programId,
      rent: SYSVAR_RENT_PUBKEY,
    })
    .rpc();
  console.log("initialize_vault tx:", initTx);

  if (
    typeof methods.addPairToken === "function" ||
    typeof methods.add_pair_token === "function"
  ) {
    const addPairFn = methods.addPairToken || methods.add_pair_token;
    const [vaultTokenAccountBPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault_tokens_b"), tokenMint.toBuffer()],
      PROGRAM_ID
    );
    const pairTx = await addPairFn()
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
    console.log("add_pair_token tx:", pairTx);
  } else {
    console.log("add_pair_token отсутствует в IDL, пропускаем этот шаг.");
  }

  const [userCardPda] = PublicKey.findProgramAddressSync(
    [
      Buffer.from("user_card"),
      vaultStatePda.toBuffer(),
      adminKeypair.publicKey.toBuffer(),
    ],
    PROGRAM_ID
  );

  const cardTx = await methods
    .createUserCard()
    .accounts({
      user: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      systemProgram: SystemProgram.programId,
    })
    .rpc();
  console.log("create_user_card tx:", cardTx);

  const userShareAccount = await getOrCreateAssociatedTokenAccount(
    connection,
    adminKeypair,
    shareMintPda,
    adminKeypair.publicKey
  );

  const depositAmount = new anchor.BN(250_000_000);
  const depositTx = await methods
    .deposit(depositAmount)
    .accounts({
      user: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      vaultTokenAccount: vaultTokenAccountPda,
      shareMint: shareMintPda,
      userTokenAccount: userTokenAccount.address,
      userShareAccount: userShareAccount.address,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();
  console.log("deposit tx:", depositTx);

  const vaultAccAfterDeposit: any = await (program.account as any).vaultState.fetch(
    vaultStatePda
  );
  const cardAccAfterDeposit: any = await (program.account as any).userCard.fetch(
    userCardPda
  );
  const vaultTokenBalAfterDeposit = await getAccount(
    connection,
    vaultTokenAccountPda
  );

  console.log("Vault state after deposit:", vaultAccAfterDeposit);
  console.log(
    "Vault token account balance after deposit:",
    vaultTokenBalAfterDeposit.amount.toString()
  );
  console.log("UserCard after deposit:", cardAccAfterDeposit);

  const withdrawShares = new anchor.BN(100_000_000);
  const withdrawTx = await methods
    .withdraw(withdrawShares)
    .accounts({
      user: adminKeypair.publicKey,
      vaultState: vaultStatePda,
      userCard: userCardPda,
      vaultTokenAccount: vaultTokenAccountPda,
      shareMint: shareMintPda,
      userTokenAccount: userTokenAccount.address,
      userShareAccount: userShareAccount.address,
      tokenProgram: TOKEN_PROGRAM_ID,
    })
    .rpc();
  console.log("withdraw tx:", withdrawTx);

  const vaultAccAfterWithdraw: any = await (program.account as any).vaultState.fetch(
    vaultStatePda
  );
  const vaultTokenBalAfterWithdraw = await getAccount(
    connection,
    vaultTokenAccountPda
  );
  const userTokenBal = await getAccount(connection, userTokenAccount.address);

  console.log("Vault state after withdraw:", vaultAccAfterWithdraw);
  console.log(
    "Vault token account balance after withdraw:",
    vaultTokenBalAfterWithdraw.amount.toString()
  );
  console.log(
    "User token balance after withdraw:",
    userTokenBal.amount.toString()
  );
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});