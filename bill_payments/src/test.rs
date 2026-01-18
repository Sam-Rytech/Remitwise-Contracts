#![cfg(test)]

use super::*;
use soroban_sdk::{symbol_short, Env, String};

#[test]
fn test_create_bill() {
    let env = Env::default();
    let contract_id = env.register_contract(None, BillPayments);
    let client = BillPaymentsClient::new(&env, &contract_id);
    
    let name = String::from_str(&env, "Electricity");
    let bill_id = client.create_bill(
        &name,
        &5000i128, // $50
        &1735689600u64, // Due date
        &true, // Recurring
        &30u32, // Monthly
    );
    
    assert_eq!(bill_id, 1);
}

#[test]
fn test_pay_bill() {
    let env = Env::default();
    let contract_id = env.register_contract(None, BillPayments);
    let client = BillPaymentsClient::new(&env, &contract_id);
    
    let name = String::from_str(&env, "Electricity");
    let bill_id = client.create_bill(&name, &5000i128, &1735689600u64, &true, &30u32);
    
    let result = client.pay_bill(&bill_id);
    assert!(result);
    
    // Should create next recurring bill
    let next_bill = client.get_bill(&2u32);
    assert!(next_bill.is_some());
}

#[test]
fn test_get_unpaid_bills() {
    let env = Env::default();
    let contract_id = env.register_contract(None, BillPayments);
    let client = BillPaymentsClient::new(&env, &contract_id);
    
    let name1 = String::from_str(&env, "Electricity");
    let name2 = String::from_str(&env, "School Fees");
    
    client.create_bill(&name1, &5000i128, &1735689600u64, &false, &0u32);
    client.create_bill(&name2, &10000i128, &1735689600u64, &false, &0u32);
    
    let unpaid = client.get_unpaid_bills();
    assert_eq!(unpaid.len(), 2);
    
    client.pay_bill(&1u32);
    let unpaid = client.get_unpaid_bills();
    assert_eq!(unpaid.len(), 1);
}

