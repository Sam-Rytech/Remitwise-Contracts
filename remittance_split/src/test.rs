#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as AddressTrait, Ledger, LedgerInfo},
    Address, Env,
};

fn set_time(env: &Env, timestamp: u64) {
    let proto = env.ledger().protocol_version();

    env.ledger().set(LedgerInfo {
        protocol_version: proto,
        sequence_number: 1,
        timestamp,
        network_id: [0; 32],
        base_reserve: 10,
        min_temp_entry_ttl: 1,
        min_persistent_entry_ttl: 1,
        max_entry_ttl: 100000,
    });
}

#[test]
fn test_initialize_split() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    let success = client.initialize_split(
        &owner, &0,  // nonce
        &50, // spending
        &30, // savings
        &15, // bills
        &5,  // insurance
    );

    assert_eq!(success, true);

    let config = client.get_config().unwrap();
    assert_eq!(config.owner, owner);
    assert_eq!(config.spending_percent, 50);
    assert_eq!(config.savings_percent, 30);
    assert_eq!(config.bills_percent, 15);
    assert_eq!(config.insurance_percent, 5);
}

#[test]
fn test_initialize_split_invalid_sum() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    let result = client.try_initialize_split(
        &owner, &0, // nonce
        &50, &50, &10, // Sums to 110
        &0,
    );
    assert_eq!(result, Err(Ok(RemittanceSplitError::InvalidPercentages)));
}

#[test]
fn test_initialize_split_already_initialized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    client.initialize_split(&owner, &0, &50, &30, &15, &5);
    // Second init should fail
    let result = client.try_initialize_split(&owner, &1, &50, &30, &15, &5);
    assert_eq!(result, Err(Ok(RemittanceSplitError::AlreadyInitialized)));
}

#[test]
fn test_update_split() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let success = client.update_split(&owner, &1, &40, &40, &10, &10);
    assert_eq!(success, true);

    let config = client.get_config().unwrap();
    assert_eq!(config.spending_percent, 40);
    assert_eq!(config.savings_percent, 40);
    assert_eq!(config.bills_percent, 10);
    assert_eq!(config.insurance_percent, 10);
}

#[test]
fn test_update_split_unauthorized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);
    let other = Address::generate(&env);

    env.mock_all_auths();

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let result = client.try_update_split(&other, &0, &40, &40, &10, &10);
    assert_eq!(result, Err(Ok(RemittanceSplitError::Unauthorized)));
}

#[test]
fn test_calculate_split() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    // Test with 1000 units
    let amounts = client.calculate_split(&1000);

    // spending: 50% of 1000 = 500
    // savings: 30% of 1000 = 300
    // bills: 15% of 1000 = 150
    // insurance: remainder = 1000 - 500 - 300 - 150 = 50

    assert_eq!(amounts.get(0).unwrap(), 500);
    assert_eq!(amounts.get(1).unwrap(), 300);
    assert_eq!(amounts.get(2).unwrap(), 150);
    assert_eq!(amounts.get(3).unwrap(), 50);
}

#[test]
fn test_calculate_split_rounding() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();

    // 33, 33, 33, 1 setup
    client.initialize_split(&owner, &0, &33, &33, &33, &1);

    // Total 100
    // 33% = 33
    // Remainder should go to last one (insurance) logic in contract:
    // insurance = total - spending - savings - bills
    // 100 - 33 - 33 - 33 = 1. Correct.

    let amounts = client.calculate_split(&100);
    assert_eq!(amounts.get(0).unwrap(), 33);
    assert_eq!(amounts.get(1).unwrap(), 33);
    assert_eq!(amounts.get(2).unwrap(), 33);
    assert_eq!(amounts.get(3).unwrap(), 1);
}

#[test]
fn test_calculate_split_zero_amount() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();
    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let result = client.try_calculate_split(&0);
    assert_eq!(result, Err(Ok(RemittanceSplitError::InvalidAmount)));
}

#[test]
fn test_calculate_complex_rounding() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = Address::generate(&env);

    env.mock_all_auths();
    // 17, 19, 23, 41 (Primes summing to 100)
    client.initialize_split(&owner, &0, &17, &19, &23, &41);

    // Amount 1000
    // 17% = 170
    // 19% = 190
    // 23% = 230
    // 41% = 410
    // Sum = 1000. Perfect.
    let amounts = client.calculate_split(&1000);
    assert_eq!(amounts.get(0).unwrap(), 170);
    assert_eq!(amounts.get(1).unwrap(), 190);
    assert_eq!(amounts.get(2).unwrap(), 230);
    assert_eq!(amounts.get(3).unwrap(), 410);

    // Amount 3
    // 17% of 3 = 0
    // 19% of 3 = 0
    // 23% of 3 = 0
    // Remainder = 3 - 0 - 0 - 0 = 3. All goes to insurance.
    let tiny_amounts = client.calculate_split(&3);
    assert_eq!(tiny_amounts.get(0).unwrap(), 0);
    assert_eq!(tiny_amounts.get(3).unwrap(), 3);
}

#[test]
fn test_create_remittance_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let schedule_id = client.create_remittance_schedule(&owner, &10000, &3000, &86400);
    assert_eq!(schedule_id, 1);

    let schedule = client.get_remittance_schedule(&schedule_id);
    assert!(schedule.is_some());
    let schedule = schedule.unwrap();
    assert_eq!(schedule.amount, 10000);
    assert_eq!(schedule.next_due, 3000);
    assert_eq!(schedule.interval, 86400);
    assert!(schedule.active);
}

#[test]
fn test_modify_remittance_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let schedule_id = client.create_remittance_schedule(&owner, &10000, &3000, &86400);
    client.modify_remittance_schedule(&owner, &schedule_id, &15000, &4000, &172800);

    let schedule = client.get_remittance_schedule(&schedule_id).unwrap();
    assert_eq!(schedule.amount, 15000);
    assert_eq!(schedule.next_due, 4000);
    assert_eq!(schedule.interval, 172800);
}

#[test]
fn test_cancel_remittance_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let schedule_id = client.create_remittance_schedule(&owner, &10000, &3000, &86400);
    client.cancel_remittance_schedule(&owner, &schedule_id);

    let schedule = client.get_remittance_schedule(&schedule_id).unwrap();
    assert!(!schedule.active);
}

#[test]
fn test_get_remittance_schedules() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    client.create_remittance_schedule(&owner, &10000, &3000, &86400);
    client.create_remittance_schedule(&owner, &5000, &4000, &172800);

    let schedules = client.get_remittance_schedules(&owner);
    assert_eq!(schedules.len(), 2);
}

#[test]
fn test_remittance_schedule_validation() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 5000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let result = client.try_create_remittance_schedule(&owner, &10000, &3000, &86400);
    assert!(result.is_err());
}

#[test]
fn test_remittance_schedule_zero_amount() {
    let env = Env::default();
    let contract_id = env.register_contract(None, RemittanceSplit);
    let client = RemittanceSplitClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    client.initialize_split(&owner, &0, &50, &30, &15, &5);

    let result = client.try_create_remittance_schedule(&owner, &0, &3000, &86400);
    assert!(result.is_err());
}
