#![no_std]

mod events;
use events::{RemitwiseEvents, EventCategory, EventPriority};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Vec, Symbol,
};

// If upstream added a schedule module, we keep the declaration but don't use it if it's causing errors.
// Uncomment the next line if you have a schedule.rs file
// mod schedule; 

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

const ARCHIVE_LIFETIME_THRESHOLD: u32 = 17280; 
const ARCHIVE_BUMP_AMOUNT: u32 = 2592000; 

/// Bill data structure 
#[derive(Clone)]
#[contracttype]
pub struct Bill {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub amount: i128,
    pub due_date: u64,
    pub recurring: bool,
    pub frequency_days: u32,
    pub paid: bool,
    pub created_at: u64,
    pub paid_at: Option<u64>,
    // Merged from upstream: Keep this to match their data shape
    pub schedule_id: Option<u32>, 
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    BillNotFound = 1,
    BillAlreadyPaid = 2,
    InvalidAmount = 3,
    InvalidFrequency = 4,
    Unauthorized = 5,
}

/// Archived bill
#[contracttype]
#[derive(Clone)]
pub struct ArchivedBill {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub amount: i128,
    pub paid_at: u64,
    pub archived_at: u64,
}

/// Storage statistics
#[contracttype]
#[derive(Clone)]
pub struct StorageStats {
    pub active_bills: u32,
    pub archived_bills: u32,
    pub total_unpaid_amount: i128,
    pub total_archived_amount: i128,
    pub last_updated: u64,
}

#[contract]
pub struct BillPayments;

#[contractimpl]
impl BillPayments {
    /// Create a new bill
    pub fn create_bill(
        env: Env,
        owner: Address,
        name: String,
        amount: i128,
        due_date: u64,
        recurring: bool,
        frequency_days: u32,
    ) -> Result<u32, Error> {
        owner.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if recurring && frequency_days == 0 {
            return Err(Error::InvalidFrequency);
        }

        Self::extend_instance_ttl(&env);
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let next_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32)
            + 1;

        let current_time = env.ledger().timestamp();
        let bill = Bill {
            id: next_id,
            owner: owner.clone(),
            name: name.clone(),
            amount,
            due_date,
            recurring,
            frequency_days,
            paid: false,
            created_at: current_time,
            paid_at: None,
            schedule_id: None, // Initialize to None
        };

        let bill_owner = bill.owner.clone();
        bills.set(next_id, bill);
        env.storage().instance().set(&symbol_short!("BILLS"), &bills);
        env.storage().instance().set(&symbol_short!("NEXT_ID"), &next_id);

        // Standardized Notification
        RemitwiseEvents::emit(
            &env,
            EventCategory::State,
            EventPriority::Medium,
            symbol_short!("created"),
            (next_id, bill_owner, amount, due_date)
        );

        Ok(next_id)
    }

    /// Mark a bill as paid
    pub fn pay_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        caller.require_auth();

        Self::extend_instance_ttl(&env);
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut bill = bills.get(bill_id).ok_or(Error::BillNotFound)?;

        if bill.owner != caller {
            return Err(Error::Unauthorized);
        }

        if bill.paid {
            return Err(Error::BillAlreadyPaid);
        }

        let current_time = env.ledger().timestamp();
        bill.paid = true;
        bill.paid_at = Some(current_time);

        // Handle recurring logic
        if bill.recurring {
            let next_due_date = bill.due_date + (bill.frequency_days as u64 * 86400);
            let next_id = env.storage().instance().get(&symbol_short!("NEXT_ID")).unwrap_or(0u32) + 1;

            let next_bill = Bill {
                id: next_id,
                owner: bill.owner.clone(),
                name: bill.name.clone(),
                amount: bill.amount,
                due_date: next_due_date,
                recurring: true,
                frequency_days: bill.frequency_days,
                paid: false,
                created_at: current_time,
                paid_at: None,
                schedule_id: bill.schedule_id, // Preserve schedule ID
            };
            bills.set(next_id, next_bill);
            env.storage().instance().set(&symbol_short!("NEXT_ID"), &next_id);
        }

        let paid_amount = bill.amount;
        bills.set(bill_id, bill);
        env.storage().instance().set(&symbol_short!("BILLS"), &bills);

        // Standardized Notification
        RemitwiseEvents::emit(
            &env,
            EventCategory::Transaction,
            EventPriority::High,
            symbol_short!("paid"),
            (bill_id, caller, paid_amount)
        );

        Ok(())
    }

    pub fn get_bill(env: Env, bill_id: u32) -> Option<Bill> {
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        bills.get(bill_id)
    }

    pub fn get_unpaid_bills(env: Env, owner: Address) -> Vec<Bill> {
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.owner == owner { result.push_back(bill); }
        }
        result
    }

    pub fn get_overdue_bills(env: Env) -> Vec<Bill> {
        let current_time = env.ledger().timestamp();
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.due_date < current_time { result.push_back(bill); }
        }
        result
    }

    pub fn get_total_unpaid(env: Env, owner: Address) -> i128 {
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        let mut total = 0i128;
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.owner == owner { total += bill.amount; }
        }
        total
    }

    pub fn cancel_bill(env: Env, bill_id: u32) -> Result<(), Error> {
        let mut bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        if bills.get(bill_id).is_none() { return Err(Error::BillNotFound); }
        bills.remove(bill_id);
        env.storage().instance().set(&symbol_short!("BILLS"), &bills);

        RemitwiseEvents::emit(&env, EventCategory::State, EventPriority::Medium, symbol_short!("canceled"), bill_id);
        Ok(())
    }

    pub fn get_all_bills(env: Env) -> Vec<Bill> {
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() { result.push_back(bill); }
        result
    }

    pub fn archive_paid_bills(env: Env, caller: Address, before_timestamp: u64) -> u32 {
        caller.require_auth();
        Self::extend_instance_ttl(&env);

        let mut bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));
        let mut archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(&env));

        let current_time = env.ledger().timestamp();
        let mut archived_count = 0u32;
        let mut to_remove: Vec<u32> = Vec::new(&env);

        for (id, bill) in bills.iter() {
            if let Some(paid_at) = bill.paid_at {
                if bill.paid && paid_at < before_timestamp {
                    let archived_bill = ArchivedBill {
                        id: bill.id,
                        owner: bill.owner.clone(),
                        name: bill.name.clone(),
                        amount: bill.amount,
                        paid_at,
                        archived_at: current_time,
                    };
                    archived.set(id, archived_bill);
                    to_remove.push_back(id);
                    archived_count += 1;
                }
            }
        }

        for id in to_remove.iter() {
            bills.remove(id);
        }

        env.storage().instance().set(&symbol_short!("BILLS"), &bills);
        env.storage().instance().set(&symbol_short!("ARCH_BILL"), &archived);

        Self::extend_archive_ttl(&env);
        Self::update_storage_stats(&env);

        RemitwiseEvents::emit_batch(&env, EventCategory::System, symbol_short!("archived"), archived_count);

        archived_count
    }

    pub fn get_archived_bills(env: Env, owner: Address) -> Vec<ArchivedBill> {
        let archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in archived.iter() {
            if bill.owner == owner { result.push_back(bill); }
        }
        result
    }

    pub fn get_archived_bill(env: Env, bill_id: u32) -> Option<ArchivedBill> {
        let archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(&env));
        archived.get(bill_id)
    }

    pub fn restore_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        caller.require_auth();
        Self::extend_instance_ttl(&env);

        let mut archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(&env));
        let archived_bill = archived.get(bill_id).ok_or(Error::BillNotFound)?;

        if archived_bill.owner != caller {
            return Err(Error::Unauthorized);
        }

        let mut bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(&env));

        let restored_bill = Bill {
            id: archived_bill.id,
            owner: archived_bill.owner.clone(),
            name: archived_bill.name.clone(),
            amount: archived_bill.amount,
            due_date: env.ledger().timestamp() + 2592000, 
            recurring: false,
            frequency_days: 0,
            paid: true,
            created_at: archived_bill.paid_at,
            paid_at: Some(archived_bill.paid_at),
            schedule_id: None, // Reset schedule on restore
        };

        bills.set(bill_id, restored_bill);
        archived.remove(bill_id);

        env.storage().instance().set(&symbol_short!("BILLS"), &bills);
        env.storage().instance().set(&symbol_short!("ARCH_BILL"), &archived);

        Self::update_storage_stats(&env);

        RemitwiseEvents::emit(&env, EventCategory::State, EventPriority::Medium, symbol_short!("restored"), bill_id);
        Ok(())
    }

    pub fn bulk_cleanup_bills(env: Env, caller: Address, before_timestamp: u64) -> u32 {
        caller.require_auth();
        Self::extend_instance_ttl(&env);

        let mut archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(&env));
        let mut deleted_count = 0u32;
        let mut to_remove: Vec<u32> = Vec::new(&env);

        for (id, bill) in archived.iter() {
            if bill.archived_at < before_timestamp {
                to_remove.push_back(id);
                deleted_count += 1;
            }
        }

        for id in to_remove.iter() {
            archived.remove(id);
        }

        env.storage().instance().set(&symbol_short!("ARCH_BILL"), &archived);
        Self::update_storage_stats(&env);

        RemitwiseEvents::emit_batch(&env, EventCategory::System, symbol_short!("cleaned"), deleted_count);
        deleted_count
    }

    pub fn get_storage_stats(env: Env) -> StorageStats {
        env.storage().instance().get(&symbol_short!("STOR_STAT")).unwrap_or(StorageStats {
            active_bills: 0,
            archived_bills: 0,
            total_unpaid_amount: 0,
            total_archived_amount: 0,
            last_updated: 0,
        })
    }

    // Helper functions
    fn extend_instance_ttl(env: &Env) {
        env.storage().instance().extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    fn extend_archive_ttl(env: &Env) {
        env.storage().instance().extend_ttl(ARCHIVE_LIFETIME_THRESHOLD, ARCHIVE_BUMP_AMOUNT);
    }

    fn update_storage_stats(env: &Env) {
        let bills: Map<u32, Bill> = env.storage().instance().get(&symbol_short!("BILLS")).unwrap_or_else(|| Map::new(env));
        let archived: Map<u32, ArchivedBill> = env.storage().instance().get(&symbol_short!("ARCH_BILL")).unwrap_or_else(|| Map::new(env));

        let mut active_count = 0u32;
        let mut unpaid_amount = 0i128;
        for (_, bill) in bills.iter() {
            active_count += 1;
            if !bill.paid { unpaid_amount = unpaid_amount.saturating_add(bill.amount); }
        }

        let mut archived_count = 0u32;
        let mut archived_amount = 0i128;
        for (_, bill) in archived.iter() {
            archived_count += 1;
            archived_amount = archived_amount.saturating_add(bill.amount);
        }

        let stats = StorageStats {
            active_bills: active_count,
            archived_bills: archived_count,
            total_unpaid_amount: unpaid_amount,
            total_archived_amount: archived_amount,
            last_updated: env.ledger().timestamp(),
        };

        env.storage().instance().set(&symbol_short!("STOR_STAT"), &stats);
    }
}

// Ensure tests module is linked
mod test;