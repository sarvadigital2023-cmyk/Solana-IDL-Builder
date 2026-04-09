// heymint -- Token-2022 bonding-curve launchpad
// creator_wallet royalties: buy=1%, sell=0.5% of gross_sol
use anchor_lang::prelude::*;
use anchor_lang::system_program;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token_2022::{self, Token2022};
use anchor_spl::token_interface::{Mint, TokenAccount};
use spl_token_2022::extension::{BaseStateWithExtensions, StateWithExtensions};
use spl_token_2022::extension::transfer_hook::TransferHook;
use spl_token_2022::extension::interest_bearing_mint::InterestBearingConfig;
use spl_token_2022::state::Mint as SplMint;

declare_id!("Hs8AFRdQUHLdqXkwcGtU2ftANUxjTiJPA4h5shsvFTQs");

pub const BUY_FEE_BPS: u64 = 200;
pub const SELL_FEE_BPS: u64 = 100;
pub const CREATE_FEE_LAMPORTS: u64 = 10_000_000;
pub const MIN_SUPPLY_WHOLE: u64 = 1_000_000;
pub const FIXED_DECIMALS: u8 = 6;
pub const MAX_NAME_LEN: usize = 32;
pub const MAX_SYMBOL_LEN: usize = 10;
pub const MAX_DESC_LEN: usize = 200;
pub const K_DENOM: u128 = 10_000_000;
pub const PRICE_PREC: u128 = 1_000_000_000;
pub const MAX_TOKENS_PER_TX: u64 = 200_000;
pub const PUMP_THRESHOLD_X: u64 = 3;
pub const PUMP_CAP_X: u64 = 10;
pub const BURN_NUMER: u64 = 5;
pub const BURN_DENOM: u64 = 1_000;
pub const CREATOR_BUY_NUM: u64 = 100;
pub const CREATOR_SELL_NUM: u64 = 50;
pub const DEFAULT_K_BUY: u128 = 200;
pub const K_BUY_MIN: u128 = 40;
pub const K_BUY_MAX: u128 = 1_000;

#[event]
pub struct BuyEvent {
    pub mint: Pubkey, pub user: Pubkey,
    pub whole_tokens: u64, pub gross_cost: u64,
    pub platform_fee: u64, pub creator_fee: u64, pub pool_sol: u64,
}
#[event]
pub struct SellEvent {
    pub mint: Pubkey, pub user: Pubkey,
    pub whole_tokens_in: u64, pub gross_sol: u64,
    pub platform_fee: u64, pub creator_fee: u64, pub user_payout: u64,
}

fn assert_no_forbidden_extensions(mint_info: &AccountInfo) -> Result<()> {
    let data = mint_info.try_borrow_data()?;
    if let Ok(state) = StateWithExtensions::<SplMint>::unpack(&data) {
        if let Ok(hook) = state.get_extension::<TransferHook>() {
            let pid: Pubkey = hook.program_id.into();
            require!(pid == Pubkey::default(), HeymintError::TransferHookForbidden);
        }
        if state.get_extension::<InterestBearingConfig>().is_ok() {
            return err!(HeymintError::InterestBearingForbidden);
        }
    }
    Ok(())
}

fn move_lamports(from: &AccountInfo, to: &AccountInfo, amount: u64) -> Result<()> {
    if amount == 0 { return Ok(()); }
    **from.try_borrow_mut_lamports()? = from.lamports()
        .checked_sub(amount).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    **to.try_borrow_mut_lamports()? = to.lamports()
        .checked_add(amount).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    Ok(())
}

fn pow_scaled(init_scaled: u128, n: u64, r_num: u128, r_den: u128) -> Option<u128> {
    if n == 0 { return Some(init_scaled); }
    let mut r_k = r_num.checked_mul(PRICE_PREC)?.checked_div(r_den)?;
    let mut res = init_scaled;
    let mut exp = n;
    while exp > 0 {
        if exp & 1 == 1 { res = res.checked_mul(r_k)?.checked_div(PRICE_PREC)?; }
        r_k = r_k.checked_mul(r_k)?.checked_div(PRICE_PREC)?;
        exp >>= 1;
    }
    Some(res)
}

fn price_sum(from: u64, count: u64, base: u64, k: u128) -> Result<u64> {
    if count == 0 { return Ok(0); }
    require!(count <= MAX_TOKENS_PER_TX, HeymintError::TooManyTokensPerTx);
    let r_num = K_DENOM.checked_add(k).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    let p_from = pow_scaled(
        (base as u128).checked_mul(PRICE_PREC).ok_or_else(|| error!(HeymintError::MathOverflow))?,
        from, r_num, K_DENOM,
    ).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    let r_pow = pow_scaled(PRICE_PREC, count, r_num, K_DENOM)
        .ok_or_else(|| error!(HeymintError::MathOverflow))?;
    let mid = p_from
        .checked_mul(r_pow.checked_sub(PRICE_PREC).ok_or_else(|| error!(HeymintError::MathOverflow))?)
        .ok_or_else(|| error!(HeymintError::MathOverflow))?
        .checked_div(PRICE_PREC).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    let total = mid.checked_mul(K_DENOM)
        .ok_or_else(|| error!(HeymintError::MathOverflow))?
        .checked_div(PRICE_PREC.checked_mul(k).ok_or_else(|| error!(HeymintError::MathOverflow))?)
        .ok_or_else(|| error!(HeymintError::MathOverflow))?;
    u64::try_from(total).map_err(|_| error!(HeymintError::MathOverflow))
}

fn fee_bps(amount: u64, bps: u64) -> Result<(u64, u64)> {
    let fee = u64::try_from((amount as u128 * bps as u128) / 10_000)
        .map_err(|_| error!(HeymintError::MathOverflow))?;
    let net = amount.checked_sub(fee).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    Ok((fee, net))
}
fn creator_fee(gross: u64, num: u64) -> Result<u64> {
    u64::try_from((gross as u128 * num as u128) / 10_000)
        .map_err(|_| error!(HeymintError::MathOverflow))
}
fn to_raw(whole: u64, scale: u64) -> Result<u64> {
    whole.checked_mul(scale).ok_or_else(|| error!(HeymintError::MathOverflow))
}
fn derive_base_price(fund: u64) -> Result<u64> {
    match fund {
        20_000_000     => Ok(100_000),
        1_000_000_000  => Ok(1_000_000),
        10_000_000_000 => Ok(10_000_000),
        _              => err!(HeymintError::InvalidFundingLevel),
    }
}
fn derive_k_buy(_supply: u64) -> Result<u128> { Ok(DEFAULT_K_BUY) }
fn derive_k_sell(k_buy: u128) -> Result<u128> { Ok((k_buy / 5).max(1)) }
fn pump_commission(net: u64, real_price: u64, base_price: u64) -> Result<(u64, u64)> {
    if base_price == 0 || real_price == 0 { return Ok((0, 0)); }
    let px = (real_price as u128 / base_price as u128) as u64;
    if px < PUMP_THRESHOLD_X { return Ok((0, px)); }
    let pct = px.min(PUMP_CAP_X);
    let c = u64::try_from((net as u128 * pct as u128) / 100)
        .map_err(|_| error!(HeymintError::MathOverflow))?;
    Ok((c, px))
}
fn burn_amount(whole_in: u64) -> u64 {
    let b = (whole_in as u128 * BURN_NUMER as u128) / BURN_DENOM as u128;
    if b >= whole_in as u128 { 0 } else { b as u64 }
}
fn pool_invariant(actual: u64, sol_bal: u64, rent_min: u64) -> Result<()> {
    let floor = sol_bal.checked_add(rent_min).ok_or_else(|| error!(HeymintError::MathOverflow))?;
    require!(actual >= floor, HeymintError::BalanceMismatch);
    Ok(())
}

#[program]
pub mod heymint {
    use super::*;

    pub fn initialize_treasury(ctx: Context<InitializeTreasury>, admin: Pubkey) -> Result<()> {
        require!(admin != Pubkey::default(), HeymintError::InvalidAdmin);
        require_keys_eq!(ctx.accounts.signer.key(), admin, HeymintError::Unauthorized);
        let t = &mut ctx.accounts.treasury;
        require!(t.admin == Pubkey::default(), HeymintError::AlreadyInitialized);
        t.admin = admin;
        t.bump = ctx.bumps.treasury;
        Ok(())
    }

    pub fn create_token(
        ctx: Context<CreateToken>,
        name: String, symbol: String,
        total_supply_whole: u64, description: String,
        initial_fund_sol: u64, creator_wallet: Pubkey,
    ) -> Result<()> {
        require!(name.len() <= MAX_NAME_LEN, HeymintError::NameTooLong);
        require!(symbol.len() <= MAX_SYMBOL_LEN, HeymintError::SymbolTooLong);
        require!(description.len() <= MAX_DESC_LEN, HeymintError::DescriptionTooLong);
        require!(total_supply_whole >= MIN_SUPPLY_WHOLE, HeymintError::SupplyTooLow);
        require!(total_supply_whole == 1_000_000, HeymintError::SupplyNotExact);
        require!(initial_fund_sol >= 20_000_000, HeymintError::FundingTooLow);
        require!(ctx.accounts.treasury.admin != Pubkey::default(), HeymintError::TreasuryNotInitialized);
        require!(creator_wallet != Pubkey::default(), HeymintError::InvalidCreatorWallet);
        let pool_pda = Pubkey::find_program_address(
            &[b"pool", ctx.accounts.mint.key().as_ref()], ctx.program_id).0;
        require!(creator_wallet != pool_pda, HeymintError::InvalidCreatorWallet);
        require_keys_eq!(ctx.accounts.creator.key(), creator_wallet, HeymintError::InvalidCreatorWallet);
        assert_no_forbidden_extensions(&ctx.accounts.mint.to_account_info())?;

        let decimals = FIXED_DECIMALS;
        let scale    = 10u64.pow(decimals as u32);
        let base     = derive_base_price(initial_fund_sol)?;
        let k_buy    = derive_k_buy(total_supply_whole)?;
        let k_sell   = derive_k_sell(k_buy)?;
        let bump     = ctx.bumps.pool;
        {
            let p = &mut ctx.accounts.pool;
            p.mint = ctx.accounts.mint.key();
            p.name = name.clone(); p.symbol = symbol.clone(); p.description = description.clone();
            p.decimals = decimals; p.scale = scale;
            p.total_supply_whole = total_supply_whole;
            p.sold_whole = 0; p.sol_balance = 0;
            p.base_price_lamports = base;
            p.admin_k_buy = k_buy; p.k_buy = k_buy; p.k_sell = k_sell;
            p.avg_buy_price_lamports = 0; p.pump_commission_collected = 0;
            p.burned_total = 0; p.bump = bump;
            p.initial_fund_sol = initial_fund_sol;
            p.creator_wallet = creator_wallet;
            p.starter_pack_issued = 0;
            p.transfer_hook_checked = true;
        }

        let mint_key = ctx.accounts.mint.key();
        let seeds: &[&[u8]] = &[b"pool", mint_key.as_ref(), &[bump]];

        token_2022::mint_to_checked(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                token_2022::MintToChecked {
                    mint: ctx.accounts.mint.to_account_info(),
                    to: ctx.accounts.pool_token_account.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                }, &[seeds],
            ), to_raw(total_supply_whole, scale)?, decimals,
        )?;

        let starter_pack_whole: u64 = match initial_fund_sol {
            1_000_000_000  => 2_000,
            10_000_000_000 => 5_000,
            _              => 0,
        };
        if starter_pack_whole > 0 {
            token_2022::transfer_checked(
                CpiContext::new_with_signer(
                    ctx.accounts.token_program.to_account_info(),
                    token_2022::TransferChecked {
                        from: ctx.accounts.pool_token_account.to_account_info(),
                        to: ctx.accounts.creator_token_account.to_account_info(),
                        authority: ctx.accounts.pool.to_account_info(),
                        mint: ctx.accounts.mint.to_account_info(),
                    }, &[seeds],
                ), to_raw(starter_pack_whole, scale)?, decimals,
            )?;
            let pool = &mut ctx.accounts.pool;
            pool.sold_whole = pool.sold_whole.checked_add(starter_pack_whole)
                .ok_or_else(|| error!(HeymintError::MathOverflow))?;
            pool.starter_pack_issued = starter_pack_whole;
        }

        system_program::transfer(CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.signer.to_account_info(),
                to: ctx.accounts.treasury.to_account_info(),
            },
        ), CREATE_FEE_LAMPORTS)?;

        system_program::transfer(CpiContext::new(
            ctx.accounts.system_program.to_account_info(),
            system_program::Transfer {
                from: ctx.accounts.signer.to_account_info(),
                to: ctx.accounts.pool.to_account_info(),
            },
        ), initial_fund_sol)?;
        ctx.accounts.pool.sol_balance = ctx.accounts.pool.sol_balance
            .checked_add(initial_fund_sol).ok_or_else(|| error!(HeymintError::MathOverflow))?;

        Ok(())
    }

    pub fn buy_exact_tokens(ctx: Context<Trade>, whole_tokens_out: u64, max_sol_in: u64) -> Result<()> {
        require!(whole_tokens_out > 0, HeymintError::ZeroAmount);
        require!(max_sol_in > 0, HeymintError::ZeroAmount);
        require!(whole_tokens_out <= MAX_TOKENS_PER_TX, HeymintError::TooManyTokensPerTx);
        require!(ctx.accounts.pool.transfer_hook_checked, HeymintError::TransferHookForbidden);

        let pool = &ctx.accounts.pool;
        let sold = pool.sold_whole;
        let remaining = pool.total_supply_whole.checked_sub(sold)
            .ok_or_else(|| error!(HeymintError::MathOverflow))?;
        require!(whole_tokens_out <= remaining, HeymintError::PoolExhausted);

        let gross_cost = price_sum(sold, whole_tokens_out, pool.base_price_lamports, pool.k_buy)?;
        require!(gross_cost <= max_sol_in, HeymintError::SlippageExceeded);

        let (platform_fee, after_plat) = fee_bps(gross_cost, BUY_FEE_BPS)?;
        let cfee     = creator_fee(gross_cost, CREATOR_BUY_NUM)?;
        let pool_sol = after_plat.checked_sub(cfee).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        let raw_out  = to_raw(whole_tokens_out, pool.scale)?;
        let decimals = pool.decimals;
        let bump     = pool.bump;
        let cw       = pool.creator_wallet;
        let old_avg  = pool.avg_buy_price_lamports;
        let new_sold = sold.checked_add(whole_tokens_out).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        let new_avg  = if new_sold > 0 {
            u64::try_from(((old_avg as u128 * sold as u128).saturating_add(gross_cost as u128)) / new_sold as u128).unwrap_or(old_avg)
        } else { old_avg };

        system_program::transfer(CpiContext::new(ctx.accounts.system_program.to_account_info(),
            system_program::Transfer { from: ctx.accounts.signer.to_account_info(), to: ctx.accounts.treasury.to_account_info() }), platform_fee)?;

        if cfee >= 1 && cw != Pubkey::default() && cw != ctx.accounts.pool.key() {
            system_program::transfer(CpiContext::new(ctx.accounts.system_program.to_account_info(),
                system_program::Transfer { from: ctx.accounts.signer.to_account_info(), to: ctx.accounts.creator.to_account_info() }), cfee)?;
        }

        system_program::transfer(CpiContext::new(ctx.accounts.system_program.to_account_info(),
            system_program::Transfer { from: ctx.accounts.signer.to_account_info(), to: ctx.accounts.pool.to_account_info() }), pool_sol)?;

        let mint_key = ctx.accounts.mint.key();
        let seeds: &[&[u8]] = &[b"pool", mint_key.as_ref(), &[bump]];
        token_2022::transfer_checked(
            CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(),
                token_2022::TransferChecked {
                    from: ctx.accounts.pool_token_account.to_account_info(),
                    to: ctx.accounts.user_token_account.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                    mint: ctx.accounts.mint.to_account_info(),
                }, &[seeds]), raw_out, decimals)?;

        let pool = &mut ctx.accounts.pool;
        pool.sold_whole = new_sold;
        pool.sol_balance = pool.sol_balance.checked_add(pool_sol).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        pool.avg_buy_price_lamports = new_avg;
        let rent = Rent::get()?;
        pool_invariant(pool.to_account_info().lamports(), pool.sol_balance, rent.minimum_balance(PoolState::LEN))?;
        emit!(BuyEvent { mint: ctx.accounts.mint.key(), user: ctx.accounts.signer.key(),
            whole_tokens: whole_tokens_out, gross_cost, platform_fee, creator_fee: cfee, pool_sol });
        Ok(())
    }

    pub fn sell_exact_tokens(ctx: Context<Trade>, whole_tokens_in: u64, min_sol_out: u64) -> Result<()> {
        require!(whole_tokens_in > 0, HeymintError::ZeroAmount);
        require!(whole_tokens_in <= MAX_TOKENS_PER_TX, HeymintError::TooManyTokensPerTx);
        require!(ctx.accounts.pool.transfer_hook_checked, HeymintError::TransferHookForbidden);

        let pool = &ctx.accounts.pool;
        let sold = pool.sold_whole;
        require!(whole_tokens_in <= sold, HeymintError::NotEnoughTokensSold);

        let burn_w   = burn_amount(whole_tokens_in);
        let eff_sell = whole_tokens_in.checked_sub(burn_w).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        require!(eff_sell > 0, HeymintError::BurnExceedsAmount);
        require!(burn_w <= pool.total_supply_whole, HeymintError::BurnExceedsSupply);

        let sell_from = sold.checked_sub(whole_tokens_in).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        let gross_sol = price_sum(sell_from, eff_sell, pool.base_price_lamports, pool.k_sell)?;
        require!(gross_sol <= pool.sol_balance, HeymintError::InsufficientPoolSol);

        let real_price          = gross_sol.checked_div(eff_sell).unwrap_or(0);
        let (platform_fee, net) = fee_bps(gross_sol, SELL_FEE_BPS)?;
        let (pump_comm, _px)    = pump_commission(net, real_price, pool.base_price_lamports)?;
        let cfee_raw = creator_fee(gross_sol, CREATOR_SELL_NUM)?;
        let cw       = pool.creator_wallet;
        let cfee     = if cfee_raw >= 1 && cw != Pubkey::default() && cw != ctx.accounts.pool.key() { cfee_raw } else { 0 };
        let user_payout = net.checked_sub(pump_comm).ok_or_else(|| error!(HeymintError::MathOverflow))?
            .checked_sub(cfee).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        require!(user_payout >= min_sol_out, HeymintError::SlippageExceeded);

        let decimals = pool.decimals;
        let scale    = pool.scale;
        let bump     = pool.bump;
        let mint_key = ctx.accounts.mint.key();

        token_2022::transfer_checked(CpiContext::new(ctx.accounts.token_program.to_account_info(),
            token_2022::TransferChecked {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.pool_token_account.to_account_info(),
                authority: ctx.accounts.signer.to_account_info(),
                mint: ctx.accounts.mint.to_account_info(),
            }), to_raw(whole_tokens_in, scale)?, decimals)?;

        let seeds: &[&[u8]] = &[b"pool", mint_key.as_ref(), &[bump]];
        if burn_w > 0 {
            token_2022::burn(CpiContext::new_with_signer(ctx.accounts.token_program.to_account_info(),
                token_2022::Burn {
                    mint: ctx.accounts.mint.to_account_info(),
                    from: ctx.accounts.pool_token_account.to_account_info(),
                    authority: ctx.accounts.pool.to_account_info(),
                }, &[seeds]), to_raw(burn_w, scale)?)?;
        }

        {
            let rent     = Rent::get()?;
            let rent_min = rent.minimum_balance(PoolState::LEN);
            let lam_after   = ctx.accounts.pool.to_account_info().lamports()
                .checked_sub(gross_sol).ok_or_else(|| error!(HeymintError::MathOverflow))?;
            let bal_after   = pool.sol_balance.checked_sub(gross_sol).ok_or_else(|| error!(HeymintError::MathOverflow))?;
            pool_invariant(lam_after, bal_after, rent_min)?;
        }

        move_lamports(&ctx.accounts.pool.to_account_info(), &ctx.accounts.treasury.to_account_info(), platform_fee)?;
        move_lamports(&ctx.accounts.pool.to_account_info(), &ctx.accounts.treasury.to_account_info(), pump_comm)?;
        move_lamports(&ctx.accounts.pool.to_account_info(), &ctx.accounts.signer.to_account_info(), user_payout)?;
        move_lamports(&ctx.accounts.pool.to_account_info(), &ctx.accounts.creator.to_account_info(), cfee)?;

        let pool = &mut ctx.accounts.pool;
        pool.sold_whole = sold.checked_sub(whole_tokens_in).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        pool.sol_balance = pool.sol_balance.checked_sub(gross_sol).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        pool.total_supply_whole = pool.total_supply_whole.checked_sub(burn_w).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        pool.burned_total = pool.burned_total.checked_add(burn_w).ok_or_else(|| error!(HeymintError::MathOverflow))?;
        pool.pump_commission_collected = pool.pump_commission_collected.checked_add(pump_comm).ok_or_else(|| error!(HeymintError::MathOverflow))?;

        emit!(SellEvent { mint: ctx.accounts.mint.key(), user: ctx.accounts.signer.key(),
            whole_tokens_in, gross_sol, platform_fee, creator_fee: cfee, user_payout });
        Ok(())
    }

    pub fn set_k_buy(ctx: Context<SetKBuy>, new_k: u128) -> Result<()> {
        require_keys_eq!(ctx.accounts.signer.key(), ctx.accounts.treasury.admin, HeymintError::Unauthorized);
        require!(new_k >= K_BUY_MIN, HeymintError::KBuyOutOfRange);
        require!(new_k <= K_BUY_MAX, HeymintError::KBuyOutOfRange);
        let pool = &mut ctx.accounts.pool;
        pool.admin_k_buy = new_k; pool.k_buy = new_k; pool.k_sell = (new_k / 5).max(1);
        msg!("heymint: set_k_buy pool={} k_buy={} k_sell={}", pool.mint, new_k, pool.k_sell);
        Ok(())
    }

    pub fn set_admin(ctx: Context<SetAdmin>, new_admin: Pubkey) -> Result<()> {
        require_keys_eq!(ctx.accounts.signer.key(), ctx.accounts.treasury.admin, HeymintError::Unauthorized);
        require!(new_admin != Pubkey::default(), HeymintError::InvalidAdmin);
        require!(new_admin != ctx.accounts.treasury.admin, HeymintError::SameAdmin);
        ctx.accounts.treasury.admin = new_admin;
        Ok(())
    }

    pub fn withdraw_treasury(ctx: Context<WithdrawTreasury>, amount: u64) -> Result<()> {
        require_keys_eq!(ctx.accounts.signer.key(), ctx.accounts.treasury.admin, HeymintError::Unauthorized);
        require!(amount > 0, HeymintError::ZeroAmount);
        let rent     = Rent::get()?;
        let rent_min = rent.minimum_balance(TreasuryState::LEN);
        let balance  = ctx.accounts.treasury.to_account_info().lamports();
        require!(balance >= amount.checked_add(rent_min).ok_or_else(|| error!(HeymintError::MathOverflow))?,
            HeymintError::TreasuryRentViolation);
        move_lamports(&ctx.accounts.treasury.to_account_info(), &ctx.accounts.destination.to_account_info(), amount)?;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializeTreasury<'info> {
    #[account(mut)] pub signer: Signer<'info>,
    #[account(init_if_needed, payer = signer, space = TreasuryState::LEN, seeds = [b"treasury"], bump)]
    pub treasury: Account<'info, TreasuryState>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
#[instruction(name: String, symbol: String, total_supply_whole: u64, description: String)]
pub struct CreateToken<'info> {
    #[account(mut)] pub signer: Signer<'info>,
    #[account(init, payer = signer, mint::decimals = 6, mint::authority = pool,
              mint::freeze_authority = pool, mint::token_program = token_program)]
    pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"treasury"], bump = treasury.bump)]
    pub treasury: Account<'info, TreasuryState>,
    #[account(init, payer = signer, space = PoolState::LEN, seeds = [b"pool", mint.key().as_ref()], bump)]
    pub pool: Account<'info, PoolState>,
    #[account(init, payer = signer, associated_token::mint = mint, associated_token::authority = pool,
              associated_token::token_program = token_program)]
    pub pool_token_account: InterfaceAccount<'info, TokenAccount>,
    /// CHECK: creator hot wallet
    #[account(mut)] pub creator: UncheckedAccount<'info>,
    #[account(init_if_needed, payer = signer, associated_token::mint = mint,
              associated_token::authority = creator, associated_token::token_program = token_program)]
    pub creator_token_account: InterfaceAccount<'info, TokenAccount>,
    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}

#[derive(Accounts)]
pub struct Trade<'info> {
    #[account(mut)] pub signer: Signer<'info>,
    #[account(mut)] pub mint: InterfaceAccount<'info, Mint>,
    #[account(mut, seeds = [b"pool", mint.key().as_ref()], bump = pool.bump, has_one = mint)]
    pub pool: Account<'info, PoolState>,
    #[account(mut, associated_token::mint = mint, associated_token::authority = pool,
              associated_token::token_program = token_program)]
    pub pool_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(init_if_needed, payer = signer, associated_token::mint = mint,
              associated_token::authority = signer, associated_token::token_program = token_program)]
    pub user_token_account: InterfaceAccount<'info, TokenAccount>,
    #[account(mut, seeds = [b"treasury"], bump = treasury.bump)]
    pub treasury: Account<'info, TreasuryState>,
    /// CHECK: enforced via address = pool.creator_wallet
    #[account(mut, address = pool.creator_wallet)] pub creator: UncheckedAccount<'info>,
    pub token_program: Program<'info, Token2022>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetKBuy<'info> {
    pub signer: Signer<'info>,
    #[account(mut, seeds = [b"pool", pool.mint.as_ref()], bump = pool.bump)]
    pub pool: Account<'info, PoolState>,
    #[account(seeds = [b"treasury"], bump = treasury.bump)]
    pub treasury: Account<'info, TreasuryState>,
}

#[derive(Accounts)]
pub struct SetAdmin<'info> {
    pub signer: Signer<'info>,
    #[account(mut, seeds = [b"treasury"], bump = treasury.bump)]
    pub treasury: Account<'info, TreasuryState>,
}

#[derive(Accounts)]
pub struct WithdrawTreasury<'info> {
    pub signer: Signer<'info>,
    #[account(mut, seeds = [b"treasury"], bump = treasury.bump)]
    pub treasury: Account<'info, TreasuryState>,
    /// CHECK: admin-chosen recipient
    #[account(mut)] pub destination: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
pub struct PoolState {
    pub mint: Pubkey, pub name: String, pub symbol: String, pub description: String,
    pub decimals: u8, pub scale: u64, pub total_supply_whole: u64, pub sold_whole: u64,
    pub sol_balance: u64, pub base_price_lamports: u64,
    pub k_buy: u128, pub k_sell: u128,
    pub avg_buy_price_lamports: u64, pub pump_commission_collected: u64,
    pub burned_total: u64, pub bump: u8, pub initial_fund_sol: u64,
    pub creator_wallet: Pubkey, pub starter_pack_issued: u64,
    pub admin_k_buy: u128, pub transfer_hook_checked: bool,
}
impl PoolState {
    // Borsh layout (packed, no alignment padding):
    //   discriminator             :   8
    //   mint: Pubkey              :  32
    //   name: String              :   4 (len prefix) + 32 (MAX_NAME_LEN)   =  36
    //   symbol: String            :   4 (len prefix) + 10 (MAX_SYMBOL_LEN) =  14
    //   description: String       :   4 (len prefix) + 200 (MAX_DESC_LEN)  = 204
    //   decimals: u8              :   1
    //   scale: u64                :   8
    //   total_supply_whole: u64   :   8
    //   sold_whole: u64           :   8
    //   sol_balance: u64          :   8
    //   base_price_lamports: u64  :   8
    //   k_buy: u128               :  16
    //   k_sell: u128              :  16
    //   avg_buy_price_lamports:u64:   8
    //   pump_commission_coll: u64 :   8
    //   burned_total: u64         :   8
    //   bump: u8                  :   1
    //   initial_fund_sol: u64     :   8
    //   creator_wallet: Pubkey    :  32
    //   starter_pack_issued: u64  :   8
    //   admin_k_buy: u128         :  16
    //   transfer_hook_checked:bool:   1
    //                               ---
    //   TOTAL                     : 457
    pub const LEN: usize = 457;
}

#[account]
pub struct TreasuryState { pub admin: Pubkey, pub bump: u8 }
impl TreasuryState { pub const LEN: usize = 8 + 32 + 1; }

#[error_code]
pub enum HeymintError {
    #[msg("Name too long")] NameTooLong,
    #[msg("Symbol too long")] SymbolTooLong,
    #[msg("Description too long")] DescriptionTooLong,
    #[msg("Supply too low")] SupplyTooLow,
    #[msg("Supply must be exactly 1 million")] SupplyNotExact,
    #[msg("Amount must be > 0")] ZeroAmount,
    #[msg("Exceeds MAX_TOKENS_PER_TX")] TooManyTokensPerTx,
    #[msg("Pool exhausted")] PoolExhausted,
    #[msg("Sell exceeds sold_whole")] NotEnoughTokensSold,
    #[msg("Insufficient pool SOL")] InsufficientPoolSol,
    #[msg("Burn exceeds amount")] BurnExceedsAmount,
    #[msg("Burn exceeds supply")] BurnExceedsSupply,
    #[msg("Math overflow")] MathOverflow,
    #[msg("Unauthorized")] Unauthorized,
    #[msg("Invalid admin")] InvalidAdmin,
    #[msg("Same admin")] SameAdmin,
    #[msg("Slippage exceeded")] SlippageExceeded,
    #[msg("Pool balance mismatch")] BalanceMismatch,
    #[msg("Already initialized")] AlreadyInitialized,
    #[msg("Treasury not initialized")] TreasuryNotInitialized,
    #[msg("Treasury rent violation")] TreasuryRentViolation,
    #[msg("Invalid funding level (0.02/1/10 SOL only)")] InvalidFundingLevel,
    #[msg("Minimum funding 0.02 SOL")] FundingTooLow,
    #[msg("creator_wallet: zero or pool PDA forbidden")] InvalidCreatorWallet,
    #[msg("k_buy must be between 40 and 1000 inclusive")] KBuyOutOfRange,
    #[msg("Mint has TransferHook: forbidden")] TransferHookForbidden,
    #[msg("Mint has InterestBearingConfig: forbidden")] InterestBearingForbidden,
}
