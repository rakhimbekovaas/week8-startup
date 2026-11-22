use anchor_lang::prelude::*;
use anchor_spl::token::{self, Burn, Mint, MintTo, Token, TokenAccount, Transfer};

declare_id!("EnnLxUs8kh7Ctuvc5DPs3sjn9wcM6k8XEVpjPs1Pi9ta");

#[program]
pub mod simple_vault {
    use super::*;

    pub fn initialize_vault(ctx: Context<InitializeVault>) -> Result<()> {
        let vault_state = &mut ctx.accounts.vault_state;
        vault_state.admin = ctx.accounts.admin.key();
        vault_state.token_mint = ctx.accounts.token_mint.key();
        vault_state.vault_token_account = ctx.accounts.vault_token_account.key();
        vault_state.share_mint = ctx.accounts.share_mint.key();
        vault_state.total_tokens = 0;
        vault_state.total_shares = 0;
        vault_state.bump = ctx.bumps.vault_state;
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

        let transfer_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::transfer(transfer_ctx, amount)?;

        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        let mint_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            MintTo {
                mint: ctx.accounts.share_mint.to_account_info(),
                to: ctx.accounts.user_share_account.to_account_info(),
                authority: ctx.accounts.vault_state.to_account_info(),
            },
            signer_seeds,
        );
        token::mint_to(mint_ctx, shares)?;

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

        // 1. Сжигаем shares у пользователя
        let burn_ctx = CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            Burn {
                mint: ctx.accounts.share_mint.to_account_info(),
                from: ctx.accounts.user_share_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        );
        token::burn(burn_ctx, shares_amount)?;

        // 2. Переводим токены из хранилища пользователю от имени PDA vault_state
        let token_mint_key = ctx.accounts.vault_state.token_mint;
        let bump = ctx.accounts.vault_state.bump;
        let signer_seeds: &[&[&[u8]]] = &[&[b"vault", token_mint_key.as_ref(), &[bump]]];

        let transfer_ctx = CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from: ctx.accounts.vault_token_account.to_account_info(),
                to: ctx.accounts.user_token_account.to_account_info(),
                authority: ctx.accounts.vault_state.to_account_info(),
            },
            signer_seeds,
        );
        token::transfer(transfer_ctx, tokens_out)?;

        // 3. Обновляем состояние хранилища и карточки пользователя
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

#[account]
#[derive(InitSpace)]
pub struct VaultState {
    pub admin: Pubkey,
    pub token_mint: Pubkey,
    pub vault_token_account: Pubkey,
    pub share_mint: Pubkey,
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
}