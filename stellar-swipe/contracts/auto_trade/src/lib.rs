#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

mod auth;
mod errors;
mod history;
mod multi_asset;
mod portfolio;
mod risk;
mod sdex;
mod storage;
mod strategies;

use crate::storage::DataKey;
use errors::AutoTradeError;

/// ==========================
/// Types
/// ==========================

#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OrderType {
    Market,
    Limit,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TradeStatus {
    Pending,
    PartiallyFilled,
    Filled,
    Failed,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trade {
    pub signal_id: u64,
    pub user: Address,
    pub requested_amount: i128,
    pub executed_amount: i128,
    pub executed_price: i128,
    pub timestamp: u64,
    pub status: TradeStatus,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TradeResult {
    pub trade: Trade,
}

/// ==========================
/// Contract
/// ==========================

#[contract]
pub struct AutoTradeContract;

/// ==========================
/// Implementation
/// ==========================

#[contractimpl]
impl AutoTradeContract {
    /// Execute a trade on behalf of a user based on a signal
    pub fn execute_trade(
        env: Env,
        user: Address,
        signal_id: u64,
        order_type: OrderType,
        amount: i128,
    ) -> Result<TradeResult, AutoTradeError> {
        if amount <= 0 {
            return Err(AutoTradeError::InvalidAmount);
        }

        user.require_auth();

        let signal = storage::get_signal(&env, signal_id).ok_or(AutoTradeError::SignalNotFound)?;

        if env.ledger().timestamp() > signal.expiry {
            return Err(AutoTradeError::SignalExpired);
        }

        if !auth::is_authorized(&env, &user, amount) {
            return Err(AutoTradeError::Unauthorized);
        }

        if !sdex::has_sufficient_balance(&env, &user, &signal.base_asset, amount) {
            return Err(AutoTradeError::InsufficientBalance);
        }

        // Determine if this is a sell operation (simplified)
        let is_sell = false; // This should be determined from the signal or order details

        // Set current asset price for risk calculations
        risk::set_asset_price(&env, signal.base_asset, signal.price);

        // Perform risk checks
        let stop_loss_triggered = risk::validate_trade(
            &env,
            &user,
            signal.base_asset,
            amount,
            signal.price,
            is_sell,
        )?;

        // If stop-loss is triggered, emit event and proceed with sell
        if stop_loss_triggered {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "stop_loss_triggered"),
                    user.clone(),
                    signal.base_asset,
                ),
                signal.price,
            );
        }

        let execution = match order_type {
            OrderType::Market => sdex::execute_market_order(&env, &user, &signal, amount)?,
            OrderType::Limit => sdex::execute_limit_order(&env, &user, &signal, amount)?,
        };

        let status = if execution.executed_amount == 0 {
            TradeStatus::Failed
        } else if execution.executed_amount < amount {
            TradeStatus::PartiallyFilled
        } else {
            TradeStatus::Filled
        };

        let trade = Trade {
            signal_id,
            user: user.clone(),
            requested_amount: amount,
            executed_amount: execution.executed_amount,
            executed_price: execution.executed_price,
            timestamp: env.ledger().timestamp(),
            status: status.clone(),
        };

        // Update position tracking
        if execution.executed_amount > 0 {
            let positions = risk::get_user_positions(&env, &user);
            let current_amount = positions
                .get(signal.base_asset)
                .map(|p| p.amount)
                .unwrap_or(0);

            let new_amount = if is_sell {
                current_amount - execution.executed_amount
            } else {
                current_amount + execution.executed_amount
            };

            risk::update_position(
                &env,
                &user,
                signal.base_asset,
                new_amount,
                execution.executed_price,
            );

            // Record trade in history
            risk::add_trade_record(&env, &user, signal_id, execution.executed_amount);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Trades(user.clone(), signal_id), &trade);

        if execution.executed_amount > 0 {
            let hist_status = match status {
                TradeStatus::Filled | TradeStatus::PartiallyFilled => {
                    history::HistoryTradeStatus::Executed
                }
                TradeStatus::Failed => history::HistoryTradeStatus::Failed,
                TradeStatus::Pending => history::HistoryTradeStatus::Pending,
            };
            history::record_trade(
                &env,
                &user,
                signal_id,
                signal.base_asset,
                execution.executed_amount,
                execution.executed_price,
                0,
                hist_status,
            );
        }

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "trade_executed"), user.clone(), signal_id),
            trade.clone(),
        );

        // Emit event if trade was blocked by risk limits (status = Failed due to risk)
        if status == TradeStatus::Failed {
            #[allow(deprecated)]
            env.events().publish(
                (
                    Symbol::new(&env, "risk_limit_block"),
                    user.clone(),
                    signal_id,
                ),
                amount,
            );
        }

        Ok(TradeResult { trade })
    }

    /// Fetch executed trade by user + signal
    pub fn get_trade(env: Env, user: Address, signal_id: u64) -> Option<Trade> {
        env.storage()
            .persistent()
            .get(&DataKey::Trades(user, signal_id))
    }

    /// Get user's risk configuration
    pub fn get_risk_config(env: Env, user: Address) -> risk::RiskConfig {
        risk::get_risk_config(&env, &user)
    }

    /// Update user's risk configuration
    pub fn set_risk_config(env: Env, user: Address, config: risk::RiskConfig) {
        user.require_auth();
        risk::set_risk_config(&env, &user, &config);

        #[allow(deprecated)]
        env.events().publish(
            (Symbol::new(&env, "risk_config_updated"), user.clone()),
            config,
        );
    }

    /// Get user's current positions
    pub fn get_user_positions(env: Env, user: Address) -> soroban_sdk::Map<u32, risk::Position> {
        risk::get_user_positions(&env, &user)
    }

    /// Get user's trade history (risk module, legacy)
    pub fn get_trade_history_legacy(
        env: Env,
        user: Address,
    ) -> soroban_sdk::Vec<risk::TradeRecord> {
        risk::get_trade_history(&env, &user)
    }

    /// Get paginated trade history (newest first)
    pub fn get_trade_history(
        env: Env,
        user: Address,
        offset: u32,
        limit: u32,
    ) -> soroban_sdk::Vec<history::HistoryTrade> {
        history::get_trade_history(&env, &user, offset, limit)
    }

    /// Get user portfolio with holdings and P&L
    pub fn get_portfolio(env: Env, user: Address) -> portfolio::Portfolio {
        portfolio::get_portfolio(&env, &user)
    }

    /// Grant authorization to execute trades
    pub fn grant_authorization(
        env: Env,
        user: Address,
        max_amount: i128,
        duration_days: u32,
    ) -> Result<(), AutoTradeError> {
        auth::grant_authorization(&env, &user, max_amount, duration_days)
    }

    /// Revoke authorization
    pub fn revoke_authorization(env: Env, user: Address) -> Result<(), AutoTradeError> {
        auth::revoke_authorization(&env, &user)
    }

    /// Get authorization config
    pub fn get_auth_config(env: Env, user: Address) -> Option<auth::AuthConfig> {
        auth::get_auth_config(&env, &user)
    }

    // ── DCA ──────────────────────────────────────────────────────────────────

    pub fn create_dca(
        env: Env,
        user: Address,
        asset_pair: u32,
        purchase_amount: i128,
        frequency: strategies::dca::DCAFrequency,
        duration_days: Option<u64>,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::dca::create_dca_strategy(&env, user, asset_pair, purchase_amount, frequency, duration_days)
    }

    pub fn execute_due_dca(env: Env) -> soroban_sdk::Vec<u64> {
        strategies::dca::execute_due_dca_purchases(&env)
    }

    pub fn execute_dca_purchase(env: Env, strategy_id: u64) -> Result<(), AutoTradeError> {
        strategies::dca::execute_dca_purchase(&env, strategy_id)
    }

    pub fn pause_dca(env: Env, user: Address, strategy_id: u64) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::pause_dca_strategy(&env, strategy_id)
    }

    pub fn resume_dca(env: Env, user: Address, strategy_id: u64) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::resume_dca_strategy(&env, strategy_id)
    }

    pub fn update_dca(
        env: Env,
        user: Address,
        strategy_id: u64,
        new_amount: Option<i128>,
        new_frequency: Option<strategies::dca::DCAFrequency>,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::dca::update_dca_schedule(&env, strategy_id, new_amount, new_frequency)
    }

    pub fn handle_missed_dca(env: Env, strategy_id: u64) -> Result<u32, AutoTradeError> {
        strategies::dca::handle_missed_dca_purchases(&env, strategy_id)
    }

    pub fn get_dca_strategy(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::dca::DCAStrategy, AutoTradeError> {
        strategies::dca::get_dca_strategy(&env, strategy_id)
    }

    pub fn analyze_dca(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::dca::DCAPerformance, AutoTradeError> {
        strategies::dca::analyze_dca_performance(&env, strategy_id)
    }

    // ── Mean Reversion ────────────────────────────────────────────────────────

    pub fn create_mean_reversion(
        env: Env,
        user: Address,
        asset_pair: u32,
        lookback_period_days: u32,
        entry_z_score: i128,
        exit_z_score: i128,
        position_size_pct: u32,
        max_positions: u32,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::create_mean_reversion_strategy(
            &env, user, asset_pair, lookback_period_days,
            entry_z_score, exit_z_score, position_size_pct, max_positions,
        )
    }

    pub fn get_mean_reversion(
        env: Env,
        strategy_id: u64,
    ) -> Result<strategies::mean_reversion::MeanReversionStrategy, AutoTradeError> {
        strategies::mean_reversion::get_mean_reversion_strategy(&env, strategy_id)
    }

    pub fn check_mr_signals(
        env: Env,
        strategy_id: u64,
    ) -> Result<Option<strategies::mean_reversion::ReversionSignal>, AutoTradeError> {
        strategies::mean_reversion::check_mean_reversion_signals(&env, strategy_id)
    }

    pub fn execute_mr_trade(
        env: Env,
        user: Address,
        strategy_id: u64,
        signal: strategies::mean_reversion::ReversionSignal,
    ) -> Result<u64, AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::execute_mean_reversion_trade(&env, strategy_id, signal)
    }

    pub fn check_mr_exits(
        env: Env,
        strategy_id: u64,
    ) -> Result<soroban_sdk::Vec<u64>, AutoTradeError> {
        strategies::mean_reversion::check_reversion_exits(&env, strategy_id)
    }

    pub fn adjust_mr_params(
        env: Env,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        strategies::mean_reversion::adjust_strategy_parameters(&env, strategy_id)
    }

    pub fn disable_mean_reversion(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::disable_mean_reversion_strategy(&env, strategy_id)
    }

    pub fn enable_mean_reversion(
        env: Env,
        user: Address,
        strategy_id: u64,
    ) -> Result<(), AutoTradeError> {
        user.require_auth();
        strategies::mean_reversion::enable_mean_reversion_strategy(&env, strategy_id)
    }
}

mod test;
