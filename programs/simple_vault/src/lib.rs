use anchor_lang::prelude::*;
use anchor_spl::token::{
    self, Approve, Burn, Mint, MintTo, Revoke, Token, TokenAccount, Transfer,
};
use raydium_clmm_cpi::cpi;
use raydium_clmm_cpi::cpi::accounts::{
    ClosePosition, DecreaseLiquidityV2, IncreaseLiquidityV2, OpenPositionV2, SwapSingleV2,
};
use raydium_clmm_cpi::program::RaydiumClmm;
use raydium_clmm_cpi::states::{AmmConfig, PoolState};

declare_id!("EnnLxUs8kh7Ctuvc5DPs3sjn9wcM6k8XEVpjPs1Pi9ta");

pub const RAYDIUM_CLMM_DEVNET_ID: Pubkey =
    pubkey!("DRayAUgENGQBKVaX8owNhgzkEDyoHTGVEGHVJT1E9pfH");

#[inline(never)]
fn read_pool_mints(pool_info: &AccountInfo) -> Result<(Pubkey, Pubkey)> {
    let data = pool_info.try_borrow_data()?;
    require!(data.len() >= 137, VaultError::WrongPool);
    let mint_0 = Pubkey::new_from_array(
        data[73..105]
            .try_into()
            .map_err(|_| VaultError::WrongPool)?,
    );
    let mint_1 = Pubkey::new_from_array(
        data[105..137]
            .try_into()
            .map_err(|_| VaultError::WrongPool)?,
    );
    Ok((mint_0, mint_1))
}

#[inline(never)]
fn read_position_liquidity(pos_info: &AccountInfo) -> Result<u128> {
    let data = pos_info.try_borrow_data()?;
    if data.len() < 153 {
        return Ok(0);
    }
    let liq = u128::from_le_bytes(
        data[137..153]
            .try_into()
            .map_err(|_| VaultError::MathOverflow)?,
    );
    Ok(liq)
}

#[inline(never)]
fn set_delegate_amount<'info>(
    token_program: AccountInfo<'info>,
    to: AccountInfo<'info>,
    delegate: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    amount: u64,
) -> Result<()> {
    if amount > 0 {
        token::approve(
            CpiContext::new_with_signer(
                token_program,
                Approve {
                    to,
                    delegate,
                    authority,
                },
                signer_seeds,
            ),
            amount,
        )?;
    }
    Ok(())
}

#[inline(never)]
fn revoke_delegate<'info>(
    token_program: AccountInfo<'info>,
    source: AccountInfo<'info>,
    authority: AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    should_revoke: bool,
) -> Result<()> {
    if should_revoke {
        token::revoke(CpiContext::new_with_signer(
            token_program,
            Revoke { source, authority },
            signer_seeds,
        ))?;
    }
    Ok(())
}

#[inline(never)]
fn run_open_position_cpi<'info>(
    ctx: &Context<'_, '_, '_, 'info, OpenPosition<'info>>,
    vault_0_acc: AccountInfo<'info>,
    vault_1_acc: AccountInfo<'info>,
    signer_seeds: &[&[&[u8]]],
    tick_lower: i32,
    tick_upper: i32,
    tick_array_lower_start: i32,
    tick_array_upper_start: i32,
    amount_0_max: u64,
    amount_1_max: u64,
    base_0: bool,
) -> Result<()> {
    let liquidity: u128 = if base_0 {
        amount_0_max as u128
    } else {
        amount_1_max as u128
    };

    let cpi_accounts = OpenPositionV2 {
        payer: ctx.accounts.admin.to_account_info(),
        position_nft_owner: ctx.accounts.vault_state.to_account_info(),
        position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
        position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
        metadata_account: ctx.accounts.metadata_account.to_account_info(),
        pool_state: ctx.accounts.pool_state.to_account_info(),
        protocol_position: ctx.accounts.protocol_position.to_account_info(),
        tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
        tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
        personal_position: ctx.accounts.personal_position.to_account_info(),
        token_account_0: vault_0_acc,
        token_account_1: vault_1_acc,
        token_vault_0: ctx.accounts.token_vault_0.to_account_info(),
        token_vault_1: ctx.accounts.token_vault_1.to_account_info(),
        rent: ctx.accounts.rent.to_account_info(),
        system_program: ctx.accounts.system_program.to_account_info(),
        token_program: ctx.accounts.token_program.to_account_info(),
        associated_token_program: ctx.accounts.associated_token_program.to_account_info(),
        token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
        vault_0_mint: ctx.accounts.vault_0_mint.to_account_info(),
        vault_1_mint: ctx.accounts.vault_1_mint.to_account_info(),
        metadata_program: ctx.accounts.metadata_program.to_account_info(),
    };

    let cpi_ctx = CpiContext::new_with_signer(
        ctx.accounts.raydium_program.to_account_info(),
        cpi_accounts,
        signer_seeds,
    )
    .with_remaining_accounts(ctx.remaining_accounts.to_vec());

    cpi::open_position_v2(
        cpi_ctx,
        tick_upper,
        tick_array_lower_start,
        tick_array_upper_start,
        tick_lower,
        liquidity,
        amount_0_max,
        amount_1_max,
        true,
        None,
    )
}

#[inline(never)]
fn refund_excess_rent_to_admin(vault_info: &AccountInfo, admin_info: &AccountInfo) -> Result<()> {
    let min_rent = Rent::get()?.minimum_balance(8 + VaultState::INIT_SPACE);
    let current_lamports = vault_info.lamports();
    if current_lamports > min_rent {
        let refund = current_lamports - min_rent;
        **vault_info.try_borrow_mut_lamports()? -= refund;
        **admin_info.try_borrow_mut_lamports()? += refund;
    }
    Ok(())
}

#[program]
pub mod simple_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.admin = ctx.accounts.admin.key();
        vault_state.token_mint = ctx.accounts.token_mint.key();
        vault_state.vault_token_account = ctx.accounts.vault_token_account.key();
        vault_state.share_mint = ctx.accounts.share_mint.key();
        vault_state.token_mint_b = Pubkey::default();
        vault_state.vault_token_account_b = Pubkey::default();
        vault_state.in_market = false;
        vault_state.has_open_position = false;
        vault_state.position_nft_mint = Pubkey::default();
        vault_state.total_fees_a = 0;
        vault_state.total_fees_b = 0;
        vault_state.total_tokens = 0;
        vault_state.total_shares = 0;
        vault_state.bump = ctx.bumps.vault_state;
        Ok(())
    }

    pub fn add_pair_token(ctx: Context<AddPairToken>) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require_keys_eq!(
            ctx.accounts.vault_state.token_mint_b,
            Pubkey::default(),
            VaultError::PairAlreadySet
        );
        require_keys_neq!(
            ctx.accounts.token_mint_b.key(),
            ctx.accounts.vault_state.token_mint,
            VaultError::InvalidPair
        );

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.token_mint_b = ctx.accounts.token_mint_b.key();
        vault_state.vault_token_account_b = ctx.accounts.vault_token_account_b.key();
        Ok(())
    }

    pub fn create_user_card(ctx: Context<CreateUserCard>) -> Result<()> {
        let user_card = &mut ctx.accounts.user_card;
        user_card.vault = ctx.accounts.vault_state.key();
        user_card.owner = ctx.accounts.user.key();
        user_card.deposited = 0;
        user_card.shares = 0;
        user_card.bump = ctx.bumps.user_card;
        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        require!(!ctx.accounts.vault_state.in_market, VaultError::InMarket);
        require!(amount > 0, VaultError::ZeroAmount);

        let vault_state = &ctx.accounts.vault_state;
        let shares = if vault_state.total_shares == 0 || vault_state.total_tokens == 0 {
            amount
        } else {
            (amount as u128)
                .checked_mul(vault_state.total_shares as u128)
                .ok_or(VaultError::MathOverflow)?
                .checked_div(vault_state.total_tokens as u128)
                .ok_or(VaultError::MathOverflow)? as u64
        };
        require!(shares > 0, VaultError::ZeroAmount);

        token::transfer(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user_token_account.to_account_info(),
                    to: ctx.accounts.vault_token_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            amount,
        )?;

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        token::mint_to(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                MintTo {
                    mint: ctx.accounts.share_mint.to_account_info(),
                    to: ctx.accounts.user_share_account.to_account_info(),
                    authority: ctx.accounts.vault_state.to_account_info(),
                },
                signer_seeds,
            ),
            shares,
        )?;

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.total_tokens = vault_state
            .total_tokens
            .checked_add(amount)
            .ok_or(VaultError::MathOverflow)?;
        vault_state.total_shares = vault_state
            .total_shares
            .checked_add(shares)
            .ok_or(VaultError::MathOverflow)?;

        let user_card = &mut ctx.accounts.user_card;
        user_card.deposited = user_card
            .deposited
            .checked_add(amount)
            .ok_or(VaultError::MathOverflow)?;
        user_card.shares = user_card
            .shares
            .checked_add(shares)
            .ok_or(VaultError::MathOverflow)?;

        Ok(())
    }

    pub fn withdraw(ctx: Context<Withdraw>, shares_amount: u64) -> Result<()> {
        require!(!ctx.accounts.vault_state.in_market, VaultError::InMarket);
        require!(shares_amount > 0, VaultError::ZeroAmount);
        require!(
            ctx.accounts.user_card.shares >= shares_amount,
            VaultError::InsufficientShares
        );
        require!(
            ctx.accounts.vault_state.total_shares >= shares_amount,
            VaultError::InsufficientShares
        );

        let total_tokens = ctx.accounts.vault_state.total_tokens;
        let total_shares = ctx.accounts.vault_state.total_shares;

        let tokens_out = (shares_amount as u128)
            .checked_mul(total_tokens as u128)
            .ok_or(VaultError::MathOverflow)?
            .checked_div(total_shares as u128)
            .ok_or(VaultError::MathOverflow)? as u64;
        require!(tokens_out > 0, VaultError::ZeroAmount);

        token::burn(
            CpiContext::new(
                ctx.accounts.token_program.to_account_info(),
                Burn {
                    mint: ctx.accounts.share_mint.to_account_info(),
                    from: ctx.accounts.user_share_account.to_account_info(),
                    authority: ctx.accounts.user.to_account_info(),
                },
            ),
            shares_amount,
        )?;

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.vault_state.to_account_info(),
                },
                signer_seeds,
            ),
            tokens_out,
        )?;

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.total_tokens = vault_state
            .total_tokens
            .checked_sub(tokens_out)
            .ok_or(VaultError::MathOverflow)?;
        vault_state.total_shares = vault_state
            .total_shares
            .checked_sub(shares_amount)
            .ok_or(VaultError::MathOverflow)?;

        let user_card = &mut ctx.accounts.user_card;
        user_card.shares = user_card
            .shares
            .checked_sub(shares_amount)
            .ok_or(VaultError::MathOverflow)?;
        user_card.deposited = user_card.deposited.saturating_sub(tokens_out);

        Ok(())
    }

    pub fn swap<'info>(
        ctx: Context<'_, '_, '_, 'info, Swap<'info>>,
        amount_in: u64,
        min_amount_out: u64,
        a_to_b: bool,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require_keys_neq!(
            ctx.accounts.vault_state.token_mint_b,
            Pubkey::default(),
            VaultError::InvalidPair
        );
        require_keys_eq!(
            *ctx.accounts.pool_state.owner,
            RAYDIUM_CLMM_DEVNET_ID,
            VaultError::WrongPool
        );

        let (token_mint_0, token_mint_1) =
            read_pool_mints(&ctx.accounts.pool_state.to_account_info())?;
        let mint_a = ctx.accounts.vault_state.token_mint;
        let mint_b = ctx.accounts.vault_state.token_mint_b;
        let valid_pair = (token_mint_0 == mint_a && token_mint_1 == mint_b)
            || (token_mint_0 == mint_b && token_mint_1 == mint_a);
        require!(valid_pair, VaultError::WrongPool);

        let (input_vault_acc, output_vault_acc) = if a_to_b {
            (
                &ctx.accounts.vault_token_account,
                &ctx.accounts.vault_token_account_b,
            )
        } else {
            (
                &ctx.accounts.vault_token_account_b,
                &ctx.accounts.vault_token_account,
            )
        };

        let actual_amount_in = if amount_in == u64::MAX {
            input_vault_acc.amount
        } else {
            amount_in
        };
        require!(actual_amount_in > 0, VaultError::ZeroAmount);

        if a_to_b {
            let max_allowed_a = ctx.accounts.vault_token_account.amount / 2;
            require!(
                actual_amount_in <= max_allowed_a,
                VaultError::SwapExceedsHalfVaultA
            );
        }

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        let cpi_accounts = SwapSingleV2 {
            payer: ctx.accounts.vault_state.to_account_info(),
            amm_config: ctx.accounts.amm_config.to_account_info(),
            pool_state: ctx.accounts.pool_state.to_account_info(),
            input_token_account: input_vault_acc.to_account_info(),
            output_token_account: output_vault_acc.to_account_info(),
            input_vault: ctx.accounts.input_vault.to_account_info(),
            output_vault: ctx.accounts.output_vault.to_account_info(),
            observation_state: ctx.accounts.observation_state.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
            memo_program: ctx.accounts.memo_program.to_account_info(),
            input_vault_mint: ctx.accounts.input_vault_mint.to_account_info(),
            output_vault_mint: ctx.accounts.output_vault_mint.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.raydium_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        )
        .with_remaining_accounts(ctx.remaining_accounts.to_vec());

        cpi::swap_v2(cpi_ctx, actual_amount_in, min_amount_out, 0, true)?;

        ctx.accounts.vault_state.in_market = true;
        Ok(())
    }

    pub fn open_position<'info>(
        ctx: Context<'_, '_, '_, 'info, OpenPosition<'info>>,
        tick_lower: i32,
        tick_upper: i32,
        tick_array_lower_start: i32,
        tick_array_upper_start: i32,
        amount_0_max: u64,
        amount_1_max: u64,
        base_0: bool,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require!(
            !ctx.accounts.vault_state.has_open_position,
            VaultError::PositionAlreadyOpen
        );
        require_keys_neq!(
            ctx.accounts.vault_state.token_mint_b,
            Pubkey::default(),
            VaultError::InvalidPair
        );
        require_keys_eq!(
            *ctx.accounts.pool_state.owner,
            RAYDIUM_CLMM_DEVNET_ID,
            VaultError::WrongPool
        );

        let (token_mint_0, token_mint_1) =
            read_pool_mints(&ctx.accounts.pool_state.to_account_info())?;
        let mint_a = ctx.accounts.vault_state.token_mint;
        let mint_b = ctx.accounts.vault_state.token_mint_b;
        require!(
            (token_mint_0 == mint_a && token_mint_1 == mint_b)
                || (token_mint_0 == mint_b && token_mint_1 == mint_a),
            VaultError::WrongPool
        );

        let (vault_0_acc, vault_1_acc) = if token_mint_0 == mint_a {
            (
                ctx.accounts.vault_token_account.to_account_info(),
                ctx.accounts.vault_token_account_b.to_account_info(),
            )
        } else {
            (
                ctx.accounts.vault_token_account_b.to_account_info(),
                ctx.accounts.vault_token_account.to_account_info(),
            )
        };

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        set_delegate_amount(
            ctx.accounts.token_program.to_account_info(),
            vault_0_acc.clone(),
            ctx.accounts.admin.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_0_max,
        )?;
        set_delegate_amount(
            ctx.accounts.token_program.to_account_info(),
            vault_1_acc.clone(),
            ctx.accounts.admin.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_1_max,
        )?;

        run_open_position_cpi(
            &ctx,
            vault_0_acc.clone(),
            vault_1_acc.clone(),
            signer_seeds,
            tick_lower,
            tick_upper,
            tick_array_lower_start,
            tick_array_upper_start,
            amount_0_max,
            amount_1_max,
            base_0,
        )?;

        revoke_delegate(
            ctx.accounts.token_program.to_account_info(),
            vault_0_acc,
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_0_max > 0,
        )?;
        revoke_delegate(
            ctx.accounts.token_program.to_account_info(),
            vault_1_acc,
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_1_max > 0,
        )?;

        let nft_mint_key = ctx.accounts.position_nft_mint.key();
        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.has_open_position = true;
        vault_state.in_market = true;
        vault_state.position_nft_mint = nft_mint_key;
        Ok(())
    }

    pub fn increase_liquidity<'info>(
        ctx: Context<'_, '_, '_, 'info, ModifyPosition<'info>>,
        liquidity: u128,
        amount_0_max: u64,
        amount_1_max: u64,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require!(
            ctx.accounts.vault_state.has_open_position,
            VaultError::NoOpenPosition
        );

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        set_delegate_amount(
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.token_account_0.to_account_info(),
            ctx.accounts.admin.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_0_max,
        )?;
        set_delegate_amount(
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.token_account_1.to_account_info(),
            ctx.accounts.admin.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_1_max,
        )?;

        let cpi_accounts = IncreaseLiquidityV2 {
            nft_owner: ctx.accounts.vault_state.to_account_info(),
            nft_account: ctx.accounts.position_nft_account.to_account_info(),
            pool_state: ctx.accounts.pool_state.to_account_info(),
            protocol_position: ctx.accounts.protocol_position.to_account_info(),
            personal_position: ctx.accounts.personal_position.to_account_info(),
            tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
            tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
            token_account_0: ctx.accounts.token_account_0.to_account_info(),
            token_account_1: ctx.accounts.token_account_1.to_account_info(),
            token_vault_0: ctx.accounts.token_vault_0.to_account_info(),
            token_vault_1: ctx.accounts.token_vault_1.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
            vault_0_mint: ctx.accounts.vault_0_mint.to_account_info(),
            vault_1_mint: ctx.accounts.vault_1_mint.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.raydium_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        )
        .with_remaining_accounts(ctx.remaining_accounts.to_vec());

        cpi::increase_liquidity_v2(cpi_ctx, liquidity, amount_0_max, amount_1_max, Some(true))?;

        revoke_delegate(
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.token_account_0.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_0_max > 0,
        )?;
        revoke_delegate(
            ctx.accounts.token_program.to_account_info(),
            ctx.accounts.token_account_1.to_account_info(),
            ctx.accounts.vault_state.to_account_info(),
            signer_seeds,
            amount_1_max > 0,
        )?;

        ctx.accounts.vault_state.in_market = true;
        Ok(())
    }

    pub fn collect_fees<'info>(
        ctx: Context<'_, '_, '_, 'info, ModifyPosition<'info>>,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require!(
            ctx.accounts.vault_state.has_open_position,
            VaultError::NoOpenPosition
        );

        let before_0 = ctx.accounts.token_account_0.amount;
        let before_1 = ctx.accounts.token_account_1.amount;

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        let cpi_accounts = DecreaseLiquidityV2 {
            nft_owner: ctx.accounts.vault_state.to_account_info(),
            nft_account: ctx.accounts.position_nft_account.to_account_info(),
            pool_state: ctx.accounts.pool_state.to_account_info(),
            protocol_position: ctx.accounts.protocol_position.to_account_info(),
            personal_position: ctx.accounts.personal_position.to_account_info(),
            tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
            tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
            recipient_token_account_0: ctx.accounts.token_account_0.to_account_info(),
            recipient_token_account_1: ctx.accounts.token_account_1.to_account_info(),
            token_vault_0: ctx.accounts.token_vault_0.to_account_info(),
            token_vault_1: ctx.accounts.token_vault_1.to_account_info(),
            token_program: ctx.accounts.token_program.to_account_info(),
            token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
            vault_0_mint: ctx.accounts.vault_0_mint.to_account_info(),
            vault_1_mint: ctx.accounts.vault_1_mint.to_account_info(),
            memo_program: ctx.accounts.memo_program.to_account_info(),
        };

        let cpi_ctx = CpiContext::new_with_signer(
            ctx.accounts.raydium_program.to_account_info(),
            cpi_accounts,
            signer_seeds,
        )
        .with_remaining_accounts(ctx.remaining_accounts.to_vec());

        cpi::decrease_liquidity_v2(cpi_ctx, 0, 0, 0)?;

        ctx.accounts.token_account_0.reload()?;
        ctx.accounts.token_account_1.reload()?;
        let delta_0 = ctx.accounts.token_account_0.amount.saturating_sub(before_0);
        let delta_1 = ctx.accounts.token_account_1.amount.saturating_sub(before_1);

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.total_fees_a = vault_state
            .total_fees_a
            .checked_add(delta_0)
            .ok_or(VaultError::MathOverflow)?;
        vault_state.total_fees_b = vault_state
            .total_fees_b
            .checked_add(delta_1)
            .ok_or(VaultError::MathOverflow)?;

        Ok(())
    }

    pub fn close_position<'info>(
        ctx: Context<'_, '_, '_, 'info, ClosePositionCtx<'info>>,
        amount_0_min: u64,
        amount_1_min: u64,
    ) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require!(
            ctx.accounts.vault_state.has_open_position,
            VaultError::NoOpenPosition
        );

        let before_0 = ctx.accounts.token_account_0.amount;
        let before_1 = ctx.accounts.token_account_1.amount;
        let position_liquidity = read_position_liquidity(&ctx.accounts.personal_position.to_account_info())?;

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        if position_liquidity > 0 {
            let dec_accounts = DecreaseLiquidityV2 {
                nft_owner: ctx.accounts.vault_state.to_account_info(),
                nft_account: ctx.accounts.position_nft_account.to_account_info(),
                pool_state: ctx.accounts.pool_state.to_account_info(),
                protocol_position: ctx.accounts.protocol_position.to_account_info(),
                personal_position: ctx.accounts.personal_position.to_account_info(),
                tick_array_lower: ctx.accounts.tick_array_lower.to_account_info(),
                tick_array_upper: ctx.accounts.tick_array_upper.to_account_info(),
                recipient_token_account_0: ctx.accounts.token_account_0.to_account_info(),
                recipient_token_account_1: ctx.accounts.token_account_1.to_account_info(),
                token_vault_0: ctx.accounts.token_vault_0.to_account_info(),
                token_vault_1: ctx.accounts.token_vault_1.to_account_info(),
                token_program: ctx.accounts.token_program.to_account_info(),
                token_program_2022: ctx.accounts.token_program_2022.to_account_info(),
                vault_0_mint: ctx.accounts.vault_0_mint.to_account_info(),
                vault_1_mint: ctx.accounts.vault_1_mint.to_account_info(),
                memo_program: ctx.accounts.memo_program.to_account_info(),
            };
            let dec_ctx = CpiContext::new_with_signer(
                ctx.accounts.raydium_program.to_account_info(),
                dec_accounts,
                signer_seeds,
            )
            .with_remaining_accounts(ctx.remaining_accounts.to_vec());

            cpi::decrease_liquidity_v2(dec_ctx, position_liquidity, amount_0_min, amount_1_min)?;
        }

        let close_accounts = ClosePosition {
            nft_owner: ctx.accounts.vault_state.to_account_info(),
            position_nft_mint: ctx.accounts.position_nft_mint.to_account_info(),
            position_nft_account: ctx.accounts.position_nft_account.to_account_info(),
            personal_position: ctx.accounts.personal_position.to_account_info(),
            system_program: ctx.accounts.system_program.to_account_info(),
            token_program: ctx.accounts.token_program_2022.to_account_info(),
        };
        let close_ctx = CpiContext::new_with_signer(
            ctx.accounts.raydium_program.to_account_info(),
            close_accounts,
            signer_seeds,
        );
        cpi::close_position(close_ctx)?;

        ctx.accounts.token_account_0.reload()?;
        ctx.accounts.token_account_1.reload()?;
        let delta_0 = ctx.accounts.token_account_0.amount.saturating_sub(before_0);
        let delta_1 = ctx.accounts.token_account_1.amount.saturating_sub(before_1);

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.total_fees_a = vault_state
            .total_fees_a
            .checked_add(delta_0)
            .ok_or(VaultError::MathOverflow)?;
        vault_state.total_fees_b = vault_state
            .total_fees_b
            .checked_add(delta_1)
            .ok_or(VaultError::MathOverflow)?;
        vault_state.has_open_position = false;
        vault_state.position_nft_mint = Pubkey::default();

        refund_excess_rent_to_admin(
            &ctx.accounts.vault_state.to_account_info(),
            &ctx.accounts.admin.to_account_info(),
        )?;

        Ok(())
    }

    pub fn sync_vault(ctx: Context<SyncVault>) -> Result<()> {
        require_keys_eq!(
            ctx.accounts.admin.key(),
            ctx.accounts.vault_state.admin,
            VaultError::Unauthorized
        );
        require!(
            !ctx.accounts.vault_state.has_open_position,
            VaultError::PositionOpen
        );
        require!(
            ctx.accounts.vault_token_account_b.amount == 0,
            VaultError::TokenBNotZero
        );

        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.total_tokens = ctx.accounts.vault_token_account.amount;
        vault_state.in_market = false;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    pub token_mint: Account<'info, Mint>,

    #[account(
        init,
        payer = admin,
        space = 8 + VaultState::INIT_SPACE,
        seeds = [b"vault", token_mint.key().as_ref()],
        bump
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(
        init,
        payer = admin,
        token::mint = token_mint,
        token::authority = vault_state,
        seeds = [b"vault_tokens", token_mint.key().as_ref()],
        bump
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        init,
        payer = admin,
        mint::decimals = 6,
        mint::authority = vault_state,
        seeds = [b"share_mint", token_mint.key().as_ref()],
        bump
    )]
    pub share_mint: Account<'info, Mint>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct AddPairToken<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub vault_state: Account<'info, VaultState>,

    pub token_mint_b: Account<'info, Mint>,

    #[account(
        init,
        payer = admin,
        token::mint = token_mint_b,
        token::authority = vault_state,
        seeds = [b"vault_tokens_b", vault_state.token_mint.as_ref()],
        bump
    )]
    pub vault_token_account_b: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct CreateUserCard<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    pub vault_state: Account<'info, VaultState>,

    #[account(
        init,
        payer = user,
        space = 8 + UserCard::INIT_SPACE,
        seeds = [b"user_card", vault_state.key().as_ref(), user.key().as_ref()],
        bump
    )]
    pub user_card: Account<'info, UserCard>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = vault_token_account,
        has_one = share_mint,
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [b"user_card", vault_state.key().as_ref(), user.key().as_ref()],
        bump = user_card.bump,
        constraint = user_card.owner == user.key(),
        constraint = user_card.vault == vault_state.key(),
    )]
    pub user_card: Account<'info, UserCard>,

    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub share_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == vault_state.token_mint,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_share_account.owner == user.key(),
        constraint = user_share_account.mint == share_mint.key(),
    )]
    pub user_share_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = vault_token_account,
        has_one = share_mint,
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(
        mut,
        seeds = [b"user_card", vault_state.key().as_ref(), user.key().as_ref()],
        bump = user_card.bump,
        constraint = user_card.owner == user.key(),
        constraint = user_card.vault == vault_state.key(),
    )]
    pub user_card: Account<'info, UserCard>,

    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub share_mint: Account<'info, Mint>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == vault_state.token_mint,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        constraint = user_share_account.owner == user.key(),
        constraint = user_share_account.mint == share_mint.key(),
    )]
    pub user_share_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct Swap<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = vault_token_account,
        has_one = vault_token_account_b,
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub vault_token_account_b: Account<'info, TokenAccount>,

    pub amm_config: Account<'info, AmmConfig>,

    /// CHECK: Десериализуется и проверяется в инструкции
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub input_vault: UncheckedAccount<'info>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub output_vault: UncheckedAccount<'info>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub observation_state: UncheckedAccount<'info>,

    /// CHECK: Mint входного токена
    pub input_vault_mint: UncheckedAccount<'info>,

    /// CHECK: Mint выходного токена
    pub output_vault_mint: UncheckedAccount<'info>,

    pub raydium_program: Program<'info, RaydiumClmm>,
    pub token_program: Program<'info, Token>,

    /// CHECK: Token-2022 program ID
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: SPL Memo program ID
    pub memo_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct OpenPosition<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = vault_token_account,
        has_one = vault_token_account_b,
    )]
    pub vault_state: Account<'info, VaultState>,

    #[account(mut)]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(mut)]
    pub vault_token_account_b: Account<'info, TokenAccount>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: NFT mint позиции
    #[account(mut)]
    pub position_nft_mint: Signer<'info>,

    /// CHECK: Токен-аккаунт NFT позиции
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: Metadata account for position NFT
    #[account(mut)]
    pub metadata_account: UncheckedAccount<'info>,

    /// CHECK: Protocol position PDA
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: Lower tick array
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Upper tick array
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    /// CHECK: Personal position PDA
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    /// CHECK: Pool vault 0
    #[account(mut)]
    pub token_vault_0: UncheckedAccount<'info>,

    /// CHECK: Pool vault 1
    #[account(mut)]
    pub token_vault_1: UncheckedAccount<'info>,

    /// CHECK: Mint 0
    pub vault_0_mint: UncheckedAccount<'info>,

    /// CHECK: Mint 1
    pub vault_1_mint: UncheckedAccount<'info>,

    pub raydium_program: Program<'info, RaydiumClmm>,
    pub token_program: Program<'info, Token>,

    /// CHECK: Token-2022 program ID
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: Associated Token program ID
    pub associated_token_program: UncheckedAccount<'info>,

    /// CHECK: Metaplex Token Metadata program ID
    pub metadata_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct ModifyPosition<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub vault_state: Account<'info, VaultState>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: Position NFT account
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: Protocol position PDA
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: Personal position PDA
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    /// CHECK: Lower tick array
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Upper tick array
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    #[account(mut)]
    pub token_account_0: Account<'info, TokenAccount>,

    #[account(mut)]
    pub token_account_1: Account<'info, TokenAccount>,

    /// CHECK: Pool vault 0
    #[account(mut)]
    pub token_vault_0: UncheckedAccount<'info>,

    /// CHECK: Pool vault 1
    #[account(mut)]
    pub token_vault_1: UncheckedAccount<'info>,

    /// CHECK: Mint 0
    pub vault_0_mint: UncheckedAccount<'info>,

    /// CHECK: Mint 1
    pub vault_1_mint: UncheckedAccount<'info>,

    pub raydium_program: Program<'info, RaydiumClmm>,
    pub token_program: Program<'info, Token>,

    /// CHECK: Token-2022 program ID
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: Memo program ID
    pub memo_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct ClosePositionCtx<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
    )]
    pub vault_state: Account<'info, VaultState>,

    /// CHECK: Проверяется Raydium CLMM в CPI
    #[account(mut)]
    pub pool_state: UncheckedAccount<'info>,

    /// CHECK: Position NFT mint
    #[account(mut)]
    pub position_nft_mint: UncheckedAccount<'info>,

    /// CHECK: Position NFT account
    #[account(mut)]
    pub position_nft_account: UncheckedAccount<'info>,

    /// CHECK: Protocol position PDA
    #[account(mut)]
    pub protocol_position: UncheckedAccount<'info>,

    /// CHECK: Personal position PDA
    #[account(mut)]
    pub personal_position: UncheckedAccount<'info>,

    /// CHECK: Lower tick array
    #[account(mut)]
    pub tick_array_lower: UncheckedAccount<'info>,

    /// CHECK: Upper tick array
    #[account(mut)]
    pub tick_array_upper: UncheckedAccount<'info>,

    #[account(mut)]
    pub token_account_0: Account<'info, TokenAccount>,

    #[account(mut)]
    pub token_account_1: Account<'info, TokenAccount>,

    /// CHECK: Pool vault 0
    #[account(mut)]
    pub token_vault_0: UncheckedAccount<'info>,

    /// CHECK: Pool vault 1
    #[account(mut)]
    pub token_vault_1: UncheckedAccount<'info>,

    /// CHECK: Mint 0
    pub vault_0_mint: UncheckedAccount<'info>,

    /// CHECK: Mint 1
    pub vault_1_mint: UncheckedAccount<'info>,

    pub raydium_program: Program<'info, RaydiumClmm>,
    pub token_program: Program<'info, Token>,

    /// CHECK: Token-2022 program ID
    pub token_program_2022: UncheckedAccount<'info>,

    /// CHECK: Memo program ID
    pub memo_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SyncVault<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"vault", vault_state.token_mint.as_ref()],
        bump = vault_state.bump,
        has_one = admin @ VaultError::Unauthorized,
        has_one = vault_token_account,
        has_one = vault_token_account_b,
    )]
    pub vault_state: Account<'info, VaultState>,

    pub vault_token_account: Account<'info, TokenAccount>,
    pub vault_token_account_b: Account<'info, TokenAccount>,
}

#[account]
#[derive(InitSpace)]
pub struct VaultState {
    pub admin: Pubkey,
    pub token_mint: Pubkey,
    pub vault_token_account: Pubkey,
    pub share_mint: Pubkey,
    pub token_mint_b: Pubkey,
    pub vault_token_account_b: Pubkey,
    pub in_market: bool,
    pub has_open_position: bool,
    pub position_nft_mint: Pubkey,
    pub total_fees_a: u64,
    pub total_fees_b: u64,
    pub total_tokens: u64,
    pub total_shares: u64,
    pub bump: u8,
}

#[account]
#[derive(InitSpace)]
pub struct UserCard {
    pub vault: Pubkey,
    pub owner: Pubkey,
    pub deposited: u64,
    pub shares: u64,
    pub bump: u8,
}

#[error_code]
pub enum VaultError {
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Insufficient shares")]
    InsufficientShares,
    #[msg("Math overflow")]
    MathOverflow,
    #[msg("Unauthorized")]
    Unauthorized,
    #[msg("Vault is currently in market")]
    InMarket,
    #[msg("Wrong Raydium pool for vault token pair")]
    WrongPool,
    #[msg("Pair token B is already set")]
    PairAlreadySet,
    #[msg("Invalid pair token mint")]
    InvalidPair,
    #[msg("Token B balance must be zero before syncing vault")]
    TokenBNotZero,
    #[msg("Position is still open")]
    PositionOpen,
    #[msg("Swap amount exceeds 50% of vault TOKEN_A balance")]
    SwapExceedsHalfVaultA,
    #[msg("Position is already open")]
    PositionAlreadyOpen,
    #[msg("No open position")]
    NoOpenPosition,
}