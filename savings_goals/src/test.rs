#![cfg(test)]

use super::*;
use soroban_sdk::{
    testutils::{Address as AddressTrait, Events, Ledger, LedgerInfo},
    Address, Env, String,
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
fn test_create_goal_unique_ids() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();

    let name1 = String::from_str(&env, "Goal 1");
    let name2 = String::from_str(&env, "Goal 2");

    // Tell the environment to auto-approve the 'user' signature
    env.mock_all_auths();

    let id1 = client.create_goal(&user, &name1, &1000, &1735689600);
    let id2 = client.create_goal(&user, &name2, &2000, &1735689600);

    assert_ne!(id1, id2);
}

#[test]
fn test_add_to_goal_increments() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();

    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "Save"), &1000, &2000000000);

    let new_balance = client.add_to_goal(&user, &id, &500);
    assert_eq!(new_balance, 500);
}

#[test]
fn test_add_to_non_existent_goal() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let res = client.try_add_to_goal(&user, &99, &500);
    assert_eq!(res, Err(Ok(SavingsGoalError::GoalNotFound)));
}

#[test]
fn test_get_goal_retrieval() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let name = String::from_str(&env, "Car");
    let id = client.create_goal(&user, &name, &5000, &2000000000);

    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.name, name);
}

#[test]
fn test_get_all_goals() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    client.create_goal(&user, &String::from_str(&env, "A"), &100, &2000000000);
    client.create_goal(&user, &String::from_str(&env, "B"), &200, &2000000000);

    let all_goals = client.get_all_goals(&user);
    assert_eq!(all_goals.len(), 2);
}

#[test]
fn test_is_goal_completed() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // 1. Create a goal with a target of 1000
    let target = 1000;
    let name = String::from_str(&env, "Trip");
    let id = client.create_goal(&user, &name, &target, &2000000000);

    // 2. It should NOT be completed initially (balance is 0)
    assert!(
        !client.is_goal_completed(&id),
        "Goal should not be complete at start"
    );

    // 3. Add exactly the target amount
    client.add_to_goal(&user, &id, &target);

    // 4. Verify the balance actually updated in storage
    let goal = client.get_goal(&id).unwrap();
    assert_eq!(
        goal.current_amount, target,
        "The amount was not saved correctly"
    );

    // 5. This will now pass once you fix the .instance() vs .persistent() mismatch in lib.rs
    assert!(
        client.is_goal_completed(&id),
        "Goal should be completed when current == target"
    );

    // 6. Bonus: Check that it stays completed if we go over the target
    client.add_to_goal(&user, &id, &1);
    assert!(
        client.is_goal_completed(&id),
        "Goal should stay completed if overfunded"
    );
}

#[test]
fn test_edge_cases_large_amounts() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Max"),
        &i128::MAX,
        &2000000000,
    );

    client.add_to_goal(&user, &id, &(i128::MAX - 100));
    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.current_amount, i128::MAX - 100);
}

#[test]
fn test_zero_amount_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let res = client.try_create_goal(&user, &String::from_str(&env, "Fail"), &0, &2000000000);
    assert_eq!(res, Err(Ok(SavingsGoalError::TargetAmountMustBePositive)));
}

#[test]
fn test_multiple_goals_management() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id1 = client.create_goal(&user, &String::from_str(&env, "G1"), &1000, &2000000000);
    let id2 = client.create_goal(&user, &String::from_str(&env, "G2"), &2000, &2000000000);

    client.add_to_goal(&user, &id1, &500);
    client.add_to_goal(&user, &id2, &1500);

    let g1 = client.get_goal(&id1).unwrap();
    let g2 = client.get_goal(&id2).unwrap();

    assert_eq!(g1.current_amount, 500);
    assert_eq!(g2.current_amount, 1500);
}

#[test]
fn test_withdraw_from_goal() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "W"), &1000, &2000000000);

    // Unlock first (created locked)
    client.unlock_goal(&user, &id);

    client.add_to_goal(&user, &id, &500);

    let new_balance = client.withdraw_from_goal(&user, &id, &200);
    assert_eq!(new_balance, 300);

    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.current_amount, 300);
}

#[test]
fn test_withdraw_too_much() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "W"), &1000, &2000000000);

    client.unlock_goal(&user, &id);
    client.add_to_goal(&user, &id, &100);

    let res = client.try_withdraw_from_goal(&user, &id, &200);
    assert_eq!(res, Err(Ok(SavingsGoalError::InsufficientBalance)));
}

#[test]
fn test_withdraw_locked() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "L"), &1000, &2000000000);

    // Goal is locked by default
    client.add_to_goal(&user, &id, &500);
    let res = client.try_withdraw_from_goal(&user, &id, &100);
    assert_eq!(res, Err(Ok(SavingsGoalError::GoalLocked)));
}

#[test]
fn test_withdraw_unauthorized() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let other = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "Auth"), &1000, &2000000000);

    client.unlock_goal(&user, &id);
    client.add_to_goal(&user, &id, &500);

    let res = client.try_withdraw_from_goal(&other, &id, &100);
    assert_eq!(res, Err(Ok(SavingsGoalError::Unauthorized)));
}

#[test]
fn test_lock_unlock_goal() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "Lock"), &1000, &2000000000);

    let goal = client.get_goal(&id).unwrap();
    assert!(goal.locked);

    client.unlock_goal(&user, &id);
    let goal = client.get_goal(&id).unwrap();
    assert!(!goal.locked);

    client.lock_goal(&user, &id);
    let goal = client.get_goal(&id).unwrap();
    assert!(goal.locked);
}

#[test]
fn test_full_withdrawal() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "W"), &1000, &2000000000);

    client.unlock_goal(&user, &id);
    client.add_to_goal(&user, &id, &500);

    // Withdraw everything
    let new_balance = client.withdraw_from_goal(&user, &id, &500);
    assert_eq!(new_balance, 0);

    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.current_amount, 0);
    assert!(!client.is_goal_completed(&id));
}

#[test]
fn test_exact_goal_completion() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(&user, &String::from_str(&env, "Exact"), &1000, &2000000000);

    // Add 500 twice
    client.add_to_goal(&user, &id, &500);
    assert!(!client.is_goal_completed(&id));

    client.add_to_goal(&user, &id, &500);
    assert!(client.is_goal_completed(&id));

    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.current_amount, 1000);
}

#[test]
fn test_set_time_lock() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    client.set_time_lock(&owner, &goal_id, &10000);

    let goal = client.get_goal(&goal_id).unwrap();
    assert_eq!(goal.unlock_date, Some(10000));
}

#[test]
fn test_withdraw_time_locked_goal_before_unlock() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    client.add_to_goal(&owner, &goal_id, &5000);
    client.unlock_goal(&owner, &goal_id);
    client.set_time_lock(&owner, &goal_id, &10000);

    let result = client.try_withdraw_from_goal(&owner, &goal_id, &1000);
    assert!(result.is_err());
}

#[test]
fn test_withdraw_time_locked_goal_after_unlock() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    client.add_to_goal(&owner, &goal_id, &5000);
    client.unlock_goal(&owner, &goal_id);
    client.set_time_lock(&owner, &goal_id, &3000);

    set_time(&env, 3500);
    let new_amount = client.withdraw_from_goal(&owner, &goal_id, &1000);
    assert_eq!(new_amount, 4000);
}

#[test]
fn test_create_savings_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &86400);
    assert_eq!(schedule_id, 1);

    let schedule = client.get_savings_schedule(&schedule_id);
    assert!(schedule.is_some());
    let schedule = schedule.unwrap();
    assert_eq!(schedule.amount, 500);
    assert_eq!(schedule.next_due, 3000);
    assert!(schedule.active);
}

#[test]
fn test_modify_savings_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &86400);
    client.modify_savings_schedule(&owner, &schedule_id, &1000, &4000, &172800);

    let schedule = client.get_savings_schedule(&schedule_id).unwrap();
    assert_eq!(schedule.amount, 1000);
    assert_eq!(schedule.next_due, 4000);
    assert_eq!(schedule.interval, 172800);
}

#[test]
fn test_cancel_savings_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &86400);
    client.cancel_savings_schedule(&owner, &schedule_id);

    let schedule = client.get_savings_schedule(&schedule_id).unwrap();
    assert!(!schedule.active);
}

#[test]
fn test_execute_due_savings_schedules() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &0);

    set_time(&env, 3500);
    let executed = client.execute_due_savings_schedules();

    assert_eq!(executed.len(), 1);
    assert_eq!(executed.get(0).unwrap(), schedule_id);

    let goal = client.get_goal(&goal_id).unwrap();
    assert_eq!(goal.current_amount, 500);
}

#[test]
fn test_execute_recurring_savings_schedule() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &86400);

    set_time(&env, 3500);
    client.execute_due_savings_schedules();

    let schedule = client.get_savings_schedule(&schedule_id).unwrap();
    assert!(schedule.active);
    assert_eq!(schedule.next_due, 3000 + 86400);

    let goal = client.get_goal(&goal_id).unwrap();
    assert_eq!(goal.current_amount, 500);
}

#[test]
fn test_execute_missed_savings_schedules() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &10000, &5000);

    let schedule_id = client.create_savings_schedule(&owner, &goal_id, &500, &3000, &86400);

    set_time(&env, 3000 + 86400 * 3 + 100);
    client.execute_due_savings_schedules();

    let schedule = client.get_savings_schedule(&schedule_id).unwrap();
    assert_eq!(schedule.missed_count, 3);
    assert!(schedule.next_due > 3000 + 86400 * 3);
}

#[test]
fn test_savings_schedule_goal_completion() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let owner = <soroban_sdk::Address as AddressTrait>::generate(&env);

    env.mock_all_auths();
    set_time(&env, 1000);

    let goal_id = client.create_goal(&owner, &String::from_str(&env, "Education"), &1000, &5000);

    client.create_savings_schedule(&owner, &goal_id, &1000, &3000, &0);

    set_time(&env, 3500);
    client.execute_due_savings_schedules();

    let goal = client.get_goal(&goal_id).unwrap();
    assert_eq!(goal.current_amount, 1000);
    assert!(client.is_goal_completed(&goal_id));
}

#[test]
fn test_lock_goal_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Lock Test"),
        &1000,
        &2000000000,
    );

    client.unlock_goal(&user, &id);
    assert!(!client.get_goal(&id).unwrap().locked);

    client.lock_goal(&user, &id);
    assert!(client.get_goal(&id).unwrap().locked);
}

#[test]
fn test_unlock_goal_success() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Unlock Test"),
        &1000,
        &2000000000,
    );

    assert!(client.get_goal(&id).unwrap().locked);

    client.unlock_goal(&user, &id);
    assert!(!client.get_goal(&id).unwrap().locked);
}

#[test]
fn test_lock_goal_unauthorized_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let other = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Auth Test"),
        &1000,
        &2000000000,
    );

    client.unlock_goal(&user, &id);

    let res = client.try_lock_goal(&other, &id);
    assert_eq!(res, Err(Ok(SavingsGoalError::Unauthorized)));
}

#[test]
fn test_unlock_goal_unauthorized_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let other = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Auth Test"),
        &1000,
        &2000000000,
    );

    let res = client.try_unlock_goal(&other, &id);
    assert_eq!(res, Err(Ok(SavingsGoalError::Unauthorized)));
}

#[test]
fn test_withdraw_after_lock_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Withdraw Fail"),
        &1000,
        &2000000000,
    );

    client.unlock_goal(&user, &id);
    client.add_to_goal(&user, &id, &500);
    client.lock_goal(&user, &id);

    let res = client.try_withdraw_from_goal(&user, &id, &100);
    assert_eq!(res, Err(Ok(SavingsGoalError::GoalLocked)));
}

#[test]
fn test_withdraw_after_unlock_succeeds() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    let id = client.create_goal(
        &user,
        &String::from_str(&env, "Withdraw Success"),
        &1000,
        &2000000000,
    );

    client.unlock_goal(&user, &id);
    client.add_to_goal(&user, &id, &500);

    let new_balance = client.withdraw_from_goal(&user, &id, &200);
    assert_eq!(new_balance, 300);

    let goal = client.get_goal(&id).unwrap();
    assert_eq!(goal.current_amount, 300);
}

#[test]
fn test_lock_nonexistent_goal_panics() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    let res = client.try_lock_goal(&user, &99);
    assert_eq!(res, Err(Ok(SavingsGoalError::GoalNotFound)));
}

#[test]
fn test_create_goal_emits_event() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create a goal
    let goal_id = client.create_goal(
        &user,
        &String::from_str(&env, "Education"),
        &10000,
        &1735689600, // Future date
    );
    assert_eq!(goal_id, 1);

    // Verify 2 events were emitted:
    // 1. GoalCreated struct event
    // 2. SavingsEvent::GoalCreated enum event (audit)
    let events = env.events().all();
    assert_eq!(events.len(), 2);
}

#[test]
fn test_add_to_goal_emits_event() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create a goal
    let goal_id = client.create_goal(
        &user,
        &String::from_str(&env, "Medical"),
        &5000,
        &1735689600,
    );

    // Get events before adding funds (should be 2 from creation)
    let events_before = env.events().all().len();

    // Add funds
    let new_amount = client.add_to_goal(&user, &goal_id, &1000);
    assert_eq!(new_amount, 1000);

    // Verify 2 new events:
    // 1. FundsAdded struct event
    // 2. SavingsEvent::FundsAdded enum event
    let events_after = env.events().all().len();
    assert_eq!(events_after - events_before, 2);
}

#[test]
fn test_goal_completed_emits_event() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create a goal with small target
    let goal_id = client.create_goal(
        &user,
        &String::from_str(&env, "Emergency Fund"),
        &1000,
        &1735689600,
    );

    // Get events before adding funds
    let events_before = env.events().all().len();

    // Add funds to complete the goal
    client.add_to_goal(&user, &goal_id, &1000);

    // Verify 4 new events (2 types for added, 2 types for completion):
    // 1. FundsAdded struct
    // 2. GoalCompleted struct
    // 3. SavingsEvent::FundsAdded enum
    // 4. SavingsEvent::GoalCompleted enum
    let events_after = env.events().all().len();
    assert_eq!(events_after - events_before, 4);
}

#[test]
fn test_multiple_goals_emit_separate_events() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create multiple goals
    client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);
    client.create_goal(&user, &String::from_str(&env, "Goal 3"), &3000, &1735689600);

    // Should have 3 * 2 events = 6 events
    let events = env.events().all();
    assert_eq!(events.len(), 6);
}

#[test]
fn test_get_goals_paginated_empty_owner() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);
    let empty_user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create goals for user but not for empty_user
    client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);

    // Test pagination for empty owner
    let response = client.get_goals_paginated(&empty_user, &None, &Some(10));
    assert_eq!(response.goals.len(), 0);
    assert!(!response.has_more);
    assert_eq!(response.next_cursor, None);
}

#[test]
fn test_get_goals_paginated_single_page() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 3 goals
    let goal1 = client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    let goal2 = client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);
    let goal3 = client.create_goal(&user, &String::from_str(&env, "Goal 3"), &3000, &1735689600);

    // Test single page with limit 10 (should return all goals)
    let response = client.get_goals_paginated(&user, &None, &Some(10));
    assert_eq!(response.goals.len(), 3);
    assert!(!response.has_more);
    assert_eq!(response.next_cursor, None);

    // Verify goal IDs in response
    let mut goal_ids = Vec::new(&env);
    for i in 0..response.goals.len() {
        if let Some(goal) = response.goals.get(i) {
            goal_ids.push_back(goal.id);
        }
    }
    assert!(goal_ids.contains(&goal1));
    assert!(goal_ids.contains(&goal2));
    assert!(goal_ids.contains(&goal3));
}

#[test]
fn test_get_goals_paginated_multiple_pages() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 5 goals
    let goal1 = client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    let goal2 = client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);
    let goal3 = client.create_goal(&user, &String::from_str(&env, "Goal 3"), &3000, &1735689600);
    let goal4 = client.create_goal(&user, &String::from_str(&env, "Goal 4"), &4000, &1735689600);
    let goal5 = client.create_goal(&user, &String::from_str(&env, "Goal 5"), &5000, &1735689600);

    // Test first page with limit 2
    let page1 = client.get_goals_paginated(&user, &None, &Some(2));
    assert_eq!(page1.goals.len(), 2);
    assert!(page1.has_more);
    assert!(page1.next_cursor.is_some());

    // Test second page using cursor
    let page2 = client.get_goals_paginated(&user, &page1.next_cursor, &Some(2));
    assert_eq!(page2.goals.len(), 2);
    assert!(page2.has_more);
    assert!(page2.next_cursor.is_some());

    // Test third page using cursor
    let page3 = client.get_goals_paginated(&user, &page2.next_cursor, &Some(2));
    assert_eq!(page3.goals.len(), 1);
    assert!(!page3.has_more);
    assert_eq!(page3.next_cursor, None);

    // Verify all goals are returned across pages
    let mut all_goals = Vec::new(&env);

    // Add goals from page1
    for i in 0..page1.goals.len() {
        if let Some(goal) = page1.goals.get(i) {
            all_goals.push_back(goal.id);
        }
    }

    // Add goals from page2
    for i in 0..page2.goals.len() {
        if let Some(goal) = page2.goals.get(i) {
            all_goals.push_back(goal.id);
        }
    }

    // Add goals from page3
    for i in 0..page3.goals.len() {
        if let Some(goal) = page3.goals.get(i) {
            all_goals.push_back(goal.id);
        }
    }

    assert_eq!(all_goals.len(), 5);
    assert!(all_goals.contains(&goal1));
    assert!(all_goals.contains(&goal2));
    assert!(all_goals.contains(&goal3));
    assert!(all_goals.contains(&goal4));
    assert!(all_goals.contains(&goal5));
}

#[test]
fn test_get_goals_paginated_default_limit() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 25 goals (more than default limit of 20)
    let goal_names = [
        "Goal 0", "Goal 1", "Goal 2", "Goal 3", "Goal 4", "Goal 5", "Goal 6", "Goal 7", "Goal 8",
        "Goal 9", "Goal 10", "Goal 11", "Goal 12", "Goal 13", "Goal 14", "Goal 15", "Goal 16",
        "Goal 17", "Goal 18", "Goal 19", "Goal 20", "Goal 21", "Goal 22", "Goal 23", "Goal 24",
    ];

    for i in 0..25 {
        client.create_goal(
            &user,
            &String::from_str(&env, goal_names[i]),
            &(1000 + i as i128),
            &1735689600,
        );
    }

    // Test with default limit (None)
    let response = client.get_goals_paginated(&user, &None, &None);
    assert_eq!(response.goals.len(), 20); // Default limit
    assert!(response.has_more);
    assert!(response.next_cursor.is_some());
}

#[test]
fn test_get_goals_paginated_max_limit_enforcement() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 25 goals (more than max limit of 100 for testing)
    let goal_names = [
        "Goal 0", "Goal 1", "Goal 2", "Goal 3", "Goal 4", "Goal 5", "Goal 6", "Goal 7", "Goal 8",
        "Goal 9", "Goal 10", "Goal 11", "Goal 12", "Goal 13", "Goal 14", "Goal 15", "Goal 16",
        "Goal 17", "Goal 18", "Goal 19", "Goal 20", "Goal 21", "Goal 22", "Goal 23", "Goal 24",
    ];

    for i in 0..25 {
        client.create_goal(
            &user,
            &String::from_str(&env, goal_names[i]),
            &(1000 + i as i128),
            &1735689600,
        );
    }

    // Test with limit exceeding max (200 should be capped to 100, but we only have 25)
    let response = client.get_goals_paginated(&user, &None, &Some(200));
    assert_eq!(response.goals.len(), 25); // All goals returned since we only have 25
    assert!(!response.has_more);
    assert_eq!(response.next_cursor, None);
}

#[test]
fn test_get_goals_paginated_minimum_limit() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 5 goals
    let goal_names = ["Goal 0", "Goal 1", "Goal 2", "Goal 3", "Goal 4"];

    for i in 0..5 {
        client.create_goal(
            &user,
            &String::from_str(&env, goal_names[i]),
            &(1000 + i as i128),
            &1735689600,
        );
    }

    // Test with limit 0 (should be treated as 1)
    let response = client.get_goals_paginated(&user, &None, &Some(0));
    assert_eq!(response.goals.len(), 1); // Minimum limit enforced
    assert!(response.has_more);
    assert!(response.next_cursor.is_some());
}

#[test]
fn test_get_goals_paginated_cursor_behavior() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 3 goals
    let goal1 = client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    let goal2 = client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);
    let goal3 = client.create_goal(&user, &String::from_str(&env, "Goal 3"), &3000, &1735689600);

    // Test first page with limit 1
    let page1 = client.get_goals_paginated(&user, &None, &Some(1));
    assert_eq!(page1.goals.len(), 1);
    assert!(page1.has_more);
    assert!(page1.next_cursor.is_some());

    // Check which goal is on first page
    let first_goal_id = page1.goals.get(0).unwrap().id;
    assert_eq!(first_goal_id, goal1);
    assert_eq!(page1.next_cursor.unwrap(), goal1);

    // Test second page using cursor
    let page2 = client.get_goals_paginated(&user, &page1.next_cursor, &Some(1));
    assert_eq!(page2.goals.len(), 1);
    assert!(page2.has_more);
    assert!(page2.next_cursor.is_some());

    // Check which goal is on second page
    let second_goal_id = page2.goals.get(0).unwrap().id;
    assert_eq!(second_goal_id, goal2);
    assert_eq!(page2.next_cursor.unwrap(), goal2);

    // Test third page using cursor
    let page3 = client.get_goals_paginated(&user, &page2.next_cursor, &Some(1));
    assert_eq!(page3.goals.len(), 1);
    assert!(!page3.has_more);
    assert_eq!(page3.next_cursor, None);

    // Check which goal is on third page
    let third_goal_id = page3.goals.get(0).unwrap().id;
    assert_eq!(third_goal_id, goal3);
}

#[test]
fn test_get_goals_paginated_cursor_not_found() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();

    // Create 3 goals
    client.create_goal(&user, &String::from_str(&env, "Goal 1"), &1000, &1735689600);
    client.create_goal(&user, &String::from_str(&env, "Goal 2"), &2000, &1735689600);
    client.create_goal(&user, &String::from_str(&env, "Goal 3"), &3000, &1735689600);

    // Test with cursor that doesn't exist (999)
    let response = client.get_goals_paginated(&user, &Some(999), &Some(10));
    assert_eq!(response.goals.len(), 0); // Should return empty since cursor not found
    assert!(!response.has_more);
    assert_eq!(response.next_cursor, None);
}
