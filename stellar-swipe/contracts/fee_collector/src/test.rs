#![cfg(test)]

use super::*;
use soroban_sdk::testutils::{Address as _, Env as _};
use soroban_sdk::{Address, Env};

fn create_test_env() -> Env {
    let env = Env::default();
    env.mock_all_auths();
    env
}

fn setup_contract(env: &Env) -> Address {
    let contract_id = env.register_contract(None, FeeCollectorContract);
    let admin = Address::generate(env);

    let client = FeeCollectorContractClient::new(env, &contract_id);
    client.initialize(&admin);

    contract_id
}

#[test]
fn test_normal_trade() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 10_000_000_000; // 1000 XLM
    let calculated_fee = 10_000_000;   // 1 XLM

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 10_000_000); // Should return the calculated fee as is
}

#[test]
fn test_large_trade_cap() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 1_000_000_000_000; // 100,000 XLM
    let calculated_fee = 2_000_000_000;   // 200 XLM (above max)

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 1_000_000_000); // Should be capped at max_fee_per_trade (100 XLM)
}

#[test]
fn test_small_trade_floor() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 1_000_000_000; // 100 XLM
    let calculated_fee = 10_000;       // 0.001 XLM (below min)

    let result = client.collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, 100_000); // Should be floored at min_fee_per_trade (0.01 XLM)
}

#[test]
fn test_tiny_trade_reject() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let trade_amount = 50_000; // 0.005 XLM (below min_fee_per_trade)
    let calculated_fee = 5_000;

    let result = client.try_collect_fee(&trade_amount, &calculated_fee);
    assert_eq!(result, Err(Ok(FeeCollectorError::TradeTooSmall)));
}

#[test]
fn test_set_fee_config() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_config = FeeConfig {
        max_fee_per_trade: 2_000_000_000, // 200 XLM
        min_fee_per_trade: 200_000,       // 0.02 XLM
    };

    client.set_fee_config(&admin, &new_config);

    let retrieved_config = client.get_fee_config();
    assert_eq!(retrieved_config, new_config);
}

#[test]
fn test_claim_fees_normal() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let provider = Address::generate(&env);
    let token = Address::generate(&env);
    let amount = 1_000_000; // 0.1 XLM

    // Simulate adding pending fees by setting storage directly
    let key = StorageKey::ProviderPendingFees(provider.clone(), token.clone());
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(&key, &amount);
    });

    // Claim fees
    let claimed = client.claim_fees(&provider, &token);
    assert_eq!(claimed, amount);

    // Check that storage is reset
    let remaining = env.as_contract(&contract_id, || {
        env.storage().persistent().get(&key).unwrap_or(0)
    });
    assert_eq!(remaining, 0);
}

#[test]
fn test_claim_fees_zero_balance() {
    let env = create_test_env();
    let contract_id = setup_contract(&env);
    let client = FeeCollectorContractClient::new(&env, &contract_id);

    let provider = Address::generate(&env);
    let token = Address::generate(&env);

    // No pending fees set, should return 0
    let claimed = client.claim_fees(&provider, &token);
    assert_eq!(claimed, 0);
}

#[test]
fn test_claim_fees_unauthorized() {
    let env = Env::default(); // No mock_all_auths
    let contract_id = env.register_contract(None, FeeCollectorContract);
    let admin = Address::generate(&env);

    let client = FeeCollectorContractClient::new(&env, &contract_id);
    client.initialize(&admin);

    let provider = Address::generate(&env);
    let token = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);

    // Try to claim with different caller - should fail auth
    let result = env.try_invoke_contract(
        &contract_id,
        &soroban_sdk::symbol_short!("claim_fees"),
        (&unauthorized_caller, &token).into_val(&env),
    );
    assert!(result.is_err()); // Should fail due to auth
}