#![no_std]

mod events;
use events::{EventCategory, EventPriority, RemitwiseEvents};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Symbol, Vec,
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
#[derive(Clone, Debug)]
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

/// Function names for selective pause (symbol_short max 9 chars)
pub mod pause_functions {
    use soroban_sdk::symbol_short;
    pub const CREATE_BILL: soroban_sdk::Symbol = symbol_short!("crt_bill");
    pub const PAY_BILL: soroban_sdk::Symbol = symbol_short!("pay_bill");
    pub const CANCEL_BILL: soroban_sdk::Symbol = symbol_short!("can_bill");
    pub const ARCHIVE: soroban_sdk::Symbol = symbol_short!("archive");
    pub const RESTORE: soroban_sdk::Symbol = symbol_short!("restore");
}

const CONTRACT_VERSION: u32 = 1;
const MAX_BATCH_SIZE: u32 = 50;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    BillNotFound = 1,
    BillAlreadyPaid = 2,
    InvalidAmount = 3,
    InvalidFrequency = 4,
    Unauthorized = 5,
    ContractPaused = 6,
    UnauthorizedPause = 7,
    FunctionPaused = 8,
    BatchTooLarge = 9,
    BatchValidationFailed = 10,
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
    // --- Pause & upgrade (admin-only) ---
    fn get_pause_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&symbol_short!("PAUSE_ADM"))
    }
    fn get_global_paused(env: &Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("PAUSED"))
            .unwrap_or(false)
    }
    fn is_function_paused(env: &Env, func: Symbol) -> bool {
        env.storage()
            .instance()
            .get::<_, Map<Symbol, bool>>(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(env))
            .get(func)
            .unwrap_or(false)
    }
    fn require_not_paused(env: &Env, func: Symbol) -> Result<(), Error> {
        if Self::get_global_paused(env) {
            return Err(Error::ContractPaused);
        }
        if Self::is_function_paused(env, func) {
            return Err(Error::FunctionPaused);
        }
        Ok(())
    }

    /// Set or transfer pause admin. First caller can set themselves as admin if none set.
    pub fn set_pause_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), Error> {
        caller.require_auth();
        let current = Self::get_pause_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(Error::UnauthorizedPause);
                }
            }
            Some(admin) if admin != caller => return Err(Error::UnauthorizedPause),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSE_ADM"), &new_admin);
        Ok(())
    }

    /// Pause all state-changing functions (admin only).
    pub fn pause(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &true);
        RemitwiseEvents::emit(
            &env,
            EventCategory::System,
            EventPriority::High,
            symbol_short!("paused"),
            (),
        );
        Ok(())
    }

    /// Unpause (admin only). Fails if time-locked unpause is set and not yet reached.
    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        let unpause_at: Option<u64> = env.storage().instance().get(&symbol_short!("UNP_AT"));
        if let Some(at) = unpause_at {
            if env.ledger().timestamp() < at {
                return Err(Error::ContractPaused); // still time-locked
            }
            env.storage().instance().remove(&symbol_short!("UNP_AT"));
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &false);
        RemitwiseEvents::emit(
            &env,
            EventCategory::System,
            EventPriority::High,
            symbol_short!("unpaused"),
            (),
        );
        Ok(())
    }

    /// Schedule unpause at a future timestamp (time-locked unpause).
    pub fn schedule_unpause(env: Env, caller: Address, at_timestamp: u64) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        if at_timestamp <= env.ledger().timestamp() {
            return Err(Error::InvalidAmount); // reuse for invalid time
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UNP_AT"), &at_timestamp);
        Ok(())
    }

    /// Pause a specific function (admin only).
    pub fn pause_function(env: Env, caller: Address, func: Symbol) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        let mut m: Map<Symbol, bool> = env
            .storage()
            .instance()
            .get(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(&env));
        m.set(func, true);
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED_FN"), &m);
        Ok(())
    }

    /// Unpause a specific function (admin only).
    pub fn unpause_function(env: Env, caller: Address, func: Symbol) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        let mut m: Map<Symbol, bool> = env
            .storage()
            .instance()
            .get(&symbol_short!("PAUSED_FN"))
            .unwrap_or_else(|| Map::new(&env));
        m.set(func, false);
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED_FN"), &m);
        Ok(())
    }

    /// Emergency pause: pause global and all tracked functions (admin only).
    pub fn emergency_pause_all(env: Env, caller: Address) -> Result<(), Error> {
        Self::pause(env.clone(), caller.clone())?;
        for func in [
            pause_functions::CREATE_BILL,
            pause_functions::PAY_BILL,
            pause_functions::CANCEL_BILL,
            pause_functions::ARCHIVE,
            pause_functions::RESTORE,
        ] {
            let _ = Self::pause_function(env.clone(), caller.clone(), func);
        }
        Ok(())
    }

    pub fn is_paused(env: Env) -> bool {
        Self::get_global_paused(&env)
    }
    pub fn is_function_paused_public(env: Env, func: Symbol) -> bool {
        Self::is_function_paused(&env, func)
    }
    pub fn get_pause_admin_public(env: Env) -> Option<Address> {
        Self::get_pause_admin(&env)
    }

    /// Contract version for upgrade tracking.
    pub fn get_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("VERSION"))
            .unwrap_or(CONTRACT_VERSION)
    }
    fn get_upgrade_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&symbol_short!("UPG_ADM"))
    }
    /// Set upgrade admin (only current upgrade_admin or first setter if none).
    pub fn set_upgrade_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), Error> {
        caller.require_auth();
        let current = Self::get_upgrade_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    return Err(Error::Unauthorized);
                }
            }
            Some(adm) if adm != caller => return Err(Error::Unauthorized),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
        Ok(())
    }
    /// Set new version (upgrade_admin only). Emits upgrade event.
    pub fn set_version(env: Env, caller: Address, new_version: u32) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).ok_or(Error::Unauthorized)?;
        if admin != caller {
            return Err(Error::Unauthorized);
        }
        let prev = Self::get_version(env.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("VERSION"), &new_version);
        RemitwiseEvents::emit(
            &env,
            EventCategory::System,
            EventPriority::High,
            symbol_short!("upgraded"),
            (prev, new_version),
        );
        Ok(())
    }

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
        Self::require_not_paused(&env, pause_functions::CREATE_BILL)?;

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
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);

        // Standardized Notification
        RemitwiseEvents::emit(
            &env,
            EventCategory::State,
            EventPriority::Medium,
            symbol_short!("created"),
            (next_id, bill_owner, amount, due_date),
        );

        Ok(next_id)
    }

    /// Mark a bill as paid
    pub fn pay_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_BILL)?;

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
            let next_id = env
                .storage()
                .instance()
                .get(&symbol_short!("NEXT_ID"))
                .unwrap_or(0u32)
                + 1;

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
            env.storage()
                .instance()
                .set(&symbol_short!("NEXT_ID"), &next_id);
        }

        let paid_amount = bill.amount;
        bills.set(bill_id, bill);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);

        // Standardized Notification
        RemitwiseEvents::emit(
            &env,
            EventCategory::Transaction,
            EventPriority::High,
            symbol_short!("paid"),
            (bill_id, caller, paid_amount),
        );

        Ok(())
    }

    pub fn get_bill(env: Env, bill_id: u32) -> Option<Bill> {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        bills.get(bill_id)
    }

    pub fn get_unpaid_bills(env: Env, owner: Address) -> Vec<Bill> {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.owner == owner {
                result.push_back(bill);
            }
        }
        result
    }

    pub fn get_overdue_bills(env: Env) -> Vec<Bill> {
        let current_time = env.ledger().timestamp();
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.due_date < current_time {
                result.push_back(bill);
            }
        }
        result
    }

    pub fn get_total_unpaid(env: Env, owner: Address) -> i128 {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut total = 0i128;
        for (_, bill) in bills.iter() {
            if !bill.paid && bill.owner == owner {
                total += bill.amount;
            }
        }
        total
    }

    pub fn cancel_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::CANCEL_BILL)?;
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let bill = bills.get(bill_id).ok_or(Error::BillNotFound)?;
        if bill.owner != caller {
            return Err(Error::Unauthorized);
        }
        bills.remove(bill_id);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);

        RemitwiseEvents::emit(
            &env,
            EventCategory::State,
            EventPriority::Medium,
            symbol_short!("canceled"),
            bill_id,
        );
        Ok(())
    }

    pub fn archive_paid_bills(
        env: Env,
        caller: Address,
        before_timestamp: u64,
    ) -> Result<u32, Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::ARCHIVE)?;
        Self::extend_instance_ttl(&env);

        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));

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

        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);
        env.storage()
            .instance()
            .set(&symbol_short!("ARCH_BILL"), &archived);

        Self::extend_archive_ttl(&env);
        Self::update_storage_stats(&env);

        RemitwiseEvents::emit_batch(
            &env,
            EventCategory::System,
            symbol_short!("archived"),
            archived_count,
        );

        Ok(archived_count)
    }

    pub fn get_archived_bills(env: Env, owner: Address) -> Vec<ArchivedBill> {
        let archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in archived.iter() {
            if bill.owner == owner {
                result.push_back(bill);
            }
        }
        result
    }

    pub fn get_archived_bill(env: Env, bill_id: u32) -> Option<ArchivedBill> {
        let archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));
        archived.get(bill_id)
    }

    pub fn restore_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::RESTORE)?;
        Self::extend_instance_ttl(&env);

        let mut archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));
        let archived_bill = archived.get(bill_id).ok_or(Error::BillNotFound)?;

        if archived_bill.owner != caller {
            return Err(Error::Unauthorized);
        }

        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

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

        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);
        env.storage()
            .instance()
            .set(&symbol_short!("ARCH_BILL"), &archived);

        Self::update_storage_stats(&env);

        RemitwiseEvents::emit(
            &env,
            EventCategory::State,
            EventPriority::Medium,
            symbol_short!("restored"),
            bill_id,
        );
        Ok(())
    }

    pub fn bulk_cleanup_bills(
        env: Env,
        caller: Address,
        before_timestamp: u64,
    ) -> Result<u32, Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::ARCHIVE)?;
        Self::extend_instance_ttl(&env);

        let mut archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));
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

        env.storage()
            .instance()
            .set(&symbol_short!("ARCH_BILL"), &archived);
        Self::update_storage_stats(&env);

        RemitwiseEvents::emit_batch(
            &env,
            EventCategory::System,
            symbol_short!("cleaned"),
            deleted_count,
        );
        Ok(deleted_count)
    }

    /// Batch pay multiple bills (atomic: all or nothing). Caller must be owner of all bills.
    pub fn batch_pay_bills(env: Env, caller: Address, bill_ids: Vec<u32>) -> Result<u32, Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_BILL)?;
        if bill_ids.len() > (MAX_BATCH_SIZE as usize).try_into().unwrap() {
            return Err(Error::BatchTooLarge);
        }
        // Validate all up front
        let bills_map: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        for id in bill_ids.iter() {
            let bill = bills_map.get(id).ok_or(Error::BillNotFound)?;
            if bill.owner != caller {
                return Err(Error::Unauthorized);
            }
            if bill.paid {
                return Err(Error::BillAlreadyPaid);
            }
        }
        Self::extend_instance_ttl(&env);
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let current_time = env.ledger().timestamp();
        let mut next_id: u32 = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32);
        let mut paid_count = 0u32;
        for id in bill_ids.iter() {
            let mut bill = bills.get(id).ok_or(Error::BillNotFound)?;
            if bill.owner != caller || bill.paid {
                return Err(Error::BatchValidationFailed);
            }
            let amount = bill.amount;
            bill.paid = true;
            bill.paid_at = Some(current_time);
            if bill.recurring {
                next_id = next_id.saturating_add(1);
                let next_due_date = bill.due_date + (bill.frequency_days as u64 * 86400);
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
                    schedule_id: bill.schedule_id,
                };
                bills.set(next_id, next_bill);
            }
            bills.set(id, bill);
            paid_count += 1;
            RemitwiseEvents::emit(
                &env,
                EventCategory::Transaction,
                EventPriority::High,
                symbol_short!("paid"),
                (id, caller.clone(), amount),
            );
        }
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);
        Self::update_storage_stats(&env);
        RemitwiseEvents::emit(
            &env,
            EventCategory::System,
            EventPriority::Medium,
            symbol_short!("batch_pay"),
            (paid_count, caller),
        );
        Ok(paid_count)
    }

    pub fn get_storage_stats(env: Env) -> StorageStats {
        env.storage()
            .instance()
            .get(&symbol_short!("STOR_STAT"))
            .unwrap_or(StorageStats {
                active_bills: 0,
                archived_bills: 0,
                total_unpaid_amount: 0,
                total_archived_amount: 0,
                last_updated: 0,
            })
    }

    // Helper functions
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    fn extend_archive_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(ARCHIVE_LIFETIME_THRESHOLD, ARCHIVE_BUMP_AMOUNT);
    }

    fn update_storage_stats(env: &Env) {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(env));
        let archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(env));

        let mut active_count = 0u32;
        let mut unpaid_amount = 0i128;
        for (_, bill) in bills.iter() {
            active_count += 1;
            if !bill.paid {
                unpaid_amount = unpaid_amount.saturating_add(bill.amount);
            }
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

        env.storage()
            .instance()
            .set(&symbol_short!("STOR_STAT"), &stats);
    }

    /// Returns only bills belonging to `owner`.
    /// This is the ONLY production-facing bills query — callers see only their own data.
    pub fn get_all_bills_for_owner(env: Env, owner: Address) -> Vec<Bill> {
        owner.require_auth();
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            if bill.owner == owner {
                result.push_back(bill);
            }
        }
        result
    }

    /// Returns ALL bills regardless of owner.
    ///
    /// ⚠️  ADMIN ONLY — restricted to the pause/upgrade admin.
    ///     Do NOT expose this in any user-facing SDK or frontend.
    pub fn get_all_bills(env: Env, caller: Address) -> Result<Vec<Bill>, Error> {
        caller.require_auth();
        // Reuse the existing pause admin as the "admin" gate —
        // it's already established in the contract, no new storage key needed.
        let admin = Self::get_pause_admin(&env).ok_or(Error::Unauthorized)?;
        if admin != caller {
            return Err(Error::Unauthorized);
        }
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            result.push_back(bill);
        }
        Ok(result)
    }
}

// Ensure tests module is linked
#[cfg(test)]
mod test;
