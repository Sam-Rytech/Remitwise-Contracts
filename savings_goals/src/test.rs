use super::*;
use soroban_sdk::{
    testutils::{Address as AddressTrait, Ledger, LedgerInfo},
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
#[should_panic] // It will panic because the goal doesn't exist
fn test_add_to_non_existent_goal() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    client.add_to_goal(&user, &99, &500);
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
#[should_panic]
fn test_zero_amount_fails() {
    let env = Env::default();
    let contract_id = env.register_contract(None, SavingsGoalContract);
    let client = SavingsGoalContractClient::new(&env, &contract_id);
    let user = Address::generate(&env);

    client.init();
    env.mock_all_auths();
    client.create_goal(&user, &String::from_str(&env, "Fail"), &0, &2000000000);
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
#[should_panic(expected = "Insufficient balance")]
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

    client.withdraw_from_goal(&user, &id, &200);
}

#[test]
#[should_panic(expected = "Cannot withdraw from a locked goal")]
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
    client.withdraw_from_goal(&user, &id, &100);
}

#[test]
#[should_panic(expected = "Only the goal owner can withdraw funds")]
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

    client.withdraw_from_goal(&other, &id, &100);
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
