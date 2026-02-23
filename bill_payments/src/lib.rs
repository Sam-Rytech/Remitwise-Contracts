#![no_std]

mod events;
use events::{EventCategory, EventPriority, RemitwiseEvents};

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Symbol, Vec,
};
// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 518400;
const ARCHIVE_LIFETIME_THRESHOLD: u32 = 17280;
const ARCHIVE_BUMP_AMOUNT: u32 = 2592000;

/// Pagination limits
pub const DEFAULT_PAGE_LIMIT: u32 = 20;
pub const MAX_PAGE_LIMIT: u32 = 50;

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
    pub schedule_id: Option<u32>,
}

/// Paginated result for bill queries
#[contracttype]
#[derive(Clone)]
pub struct BillPage {
    /// The bills for this page
    pub items: Vec<Bill>,
    /// The ID to pass as `cursor` for the next page. 0 means no more pages.
    pub next_cursor: u32,
    /// Total items returned in this page
    pub count: u32,
}

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
    InvalidLimit = 11,
}

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

/// Paginated result for archived bill queries
#[contracttype]
#[derive(Clone)]
pub struct ArchivedBillPage {
    pub items: Vec<ArchivedBill>,
    /// 0 means no more pages
    pub next_cursor: u32,
    pub count: u32,
}

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
    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

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

    /// Clamp a caller-supplied limit to [1, MAX_PAGE_LIMIT].
    /// A value of 0 is treated as DEFAULT_PAGE_LIMIT.
    fn clamp_limit(limit: u32) -> u32 {
        if limit == 0 {
            DEFAULT_PAGE_LIMIT
        } else if limit > MAX_PAGE_LIMIT {
            MAX_PAGE_LIMIT
        } else {
            limit
        }
    }

    // -----------------------------------------------------------------------
    // Pause / upgrade (unchanged from original)
    // -----------------------------------------------------------------------

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

    pub fn unpause(env: Env, caller: Address) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        let unpause_at: Option<u64> = env.storage().instance().get(&symbol_short!("UNP_AT"));
        if let Some(at) = unpause_at {
            if env.ledger().timestamp() < at {
                return Err(Error::ContractPaused);
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

    pub fn schedule_unpause(env: Env, caller: Address, at_timestamp: u64) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::UnauthorizedPause)?;
        if admin != caller {
            return Err(Error::UnauthorizedPause);
        }
        if at_timestamp <= env.ledger().timestamp() {
            return Err(Error::InvalidAmount);
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UNP_AT"), &at_timestamp);
        Ok(())
    }

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
    pub fn get_version(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("VERSION"))
            .unwrap_or(CONTRACT_VERSION)
    }
    fn get_upgrade_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&symbol_short!("UPG_ADM"))
    }
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

    // -----------------------------------------------------------------------
    // Core bill operations (unchanged)
    // -----------------------------------------------------------------------

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
            schedule_id: None,
        };

        let bill_owner = bill.owner.clone();
        bills.set(next_id, bill);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);

        RemitwiseEvents::emit(
            &env,
            EventCategory::State,
            EventPriority::Medium,
            symbol_short!("created"),
            (next_id, bill_owner, amount, due_date),
        );

        Ok(next_id)
    }

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
                schedule_id: bill.schedule_id,
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

    // -----------------------------------------------------------------------
    // PAGINATED LIST QUERIES  (new in this version)
    // -----------------------------------------------------------------------

    /// Get a page of unpaid bills for `owner`.
    ///
    /// # Arguments
    /// * `owner`  – whose bills to return
    /// * `cursor` – start after this bill ID (pass 0 for the first page)
    /// * `limit`  – max items per page (0 → DEFAULT_PAGE_LIMIT, capped at MAX_PAGE_LIMIT)
    ///
    /// # Returns
    /// `BillPage { items, next_cursor, count }`.
    /// When `next_cursor == 0` there are no more pages.
    pub fn get_unpaid_bills(env: Env, owner: Address, cursor: u32, limit: u32) -> BillPage {
        let limit = Self::clamp_limit(limit);
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        // Collect all matching items into a staging buffer (limit+1 max).
        // We must gather limit+1 *matching* items to know if a next page exists,
        // because non-matching items (wrong owner / paid) must be skipped entirely.
        let mut staging: Vec<(u32, Bill)> = Vec::new(&env);
        for (id, bill) in bills.iter() {
            if id <= cursor {
                continue;
            }
            if bill.owner != owner || bill.paid {
                continue;
            }
            staging.push_back((id, bill));
            if staging.len() > limit {
                break; // have limit+1 matching items — enough to determine next_cursor
            }
        }

        Self::build_page(&env, staging, limit)
    }

    /// Get a page of ALL bills (paid + unpaid) for `owner`.
    ///
    /// Same cursor/limit semantics as `get_unpaid_bills`.
    pub fn get_all_bills_for_owner(env: Env, owner: Address, cursor: u32, limit: u32) -> BillPage {
        owner.require_auth();
        let limit = Self::clamp_limit(limit);
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut staging: Vec<(u32, Bill)> = Vec::new(&env);
        for (id, bill) in bills.iter() {
            if id <= cursor {
                continue;
            }
            if bill.owner != owner {
                continue;
            }
            staging.push_back((id, bill));
            if staging.len() > limit {
                break;
            }
        }

        Self::build_page(&env, staging, limit)
    }

    /// Get a page of overdue (unpaid + past due_date) bills across all owners.
    ///
    /// Same cursor/limit semantics.
    pub fn get_overdue_bills(env: Env, cursor: u32, limit: u32) -> BillPage {
        let limit = Self::clamp_limit(limit);
        let current_time = env.ledger().timestamp();
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut staging: Vec<(u32, Bill)> = Vec::new(&env);
        for (id, bill) in bills.iter() {
            if id <= cursor {
                continue;
            }
            if bill.paid || bill.due_date >= current_time {
                continue;
            }
            staging.push_back((id, bill));
            if staging.len() > limit {
                break;
            }
        }

        Self::build_page(&env, staging, limit)
    }

    /// Admin-only: get ALL bills (any owner), paginated.
    pub fn get_all_bills(
        env: Env,
        caller: Address,
        cursor: u32,
        limit: u32,
    ) -> Result<BillPage, Error> {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).ok_or(Error::Unauthorized)?;
        if admin != caller {
            return Err(Error::Unauthorized);
        }

        let limit = Self::clamp_limit(limit);
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut staging: Vec<(u32, Bill)> = Vec::new(&env);
        for (id, bill) in bills.iter() {
            if id <= cursor {
                continue;
            }
            staging.push_back((id, bill));
            if staging.len() > limit {
                break;
            }
        }

        Ok(Self::build_page(&env, staging, limit))
    }

    /// Build a `BillPage` from a staging buffer of up to `limit+1` matching items.
    /// If the buffer has more than `limit` entries the last one becomes `next_cursor`.
    /// Soroban `Vec::len()` returns `u32`, so all arithmetic stays in `u32`.
    fn build_page(env: &Env, staging: Vec<(u32, Bill)>, limit: u32) -> BillPage {
        let n = staging.len(); // u32 in Soroban
        let has_next = n > limit;
        let mut items = Vec::new(env);
        let mut next_cursor: u32 = 0;

        // Number of items to emit: all of them, or all-but-last if there's a next page.
        let take = if has_next { n - 1 } else { n };

        for i in 0..take {
            if let Some((_, bill)) = staging.get(i) {
                items.push_back(bill);
            }
        }

        if has_next {
            if let Some((id, _)) = staging.get(take - 1) {
                // take - 1 = last emitted item
                next_cursor = id;
            }
        }

        let count = items.len(); // u32
        BillPage {
            items,
            next_cursor,
            count,
        }
    }

    // -----------------------------------------------------------------------
    // Backward-compat "get all" wrappers (collect every page internally)
    // -----------------------------------------------------------------------

    /// Legacy helper: returns ALL unpaid bills for owner in one Vec.
    /// Only safe for owners with a small number of bills. Prefer the
    /// paginated `get_unpaid_bills` for production use.
    pub fn get_all_unpaid_bills_legacy(env: Env, owner: Address) -> Vec<Bill> {
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

    // -----------------------------------------------------------------------
    // Archived bill queries (paginated)
    // -----------------------------------------------------------------------

    /// Get a page of archived bills for `owner`.
    pub fn get_archived_bills(
        env: Env,
        owner: Address,
        cursor: u32,
        limit: u32,
    ) -> ArchivedBillPage {
        let limit = Self::clamp_limit(limit);
        let archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));

        let mut staging: Vec<(u32, ArchivedBill)> = Vec::new(&env);
        for (id, bill) in archived.iter() {
            if id <= cursor {
                continue;
            }
            if bill.owner != owner {
                continue;
            }
            staging.push_back((id, bill));
            if staging.len() > limit {
                break;
            }
        }

        let has_next = staging.len() > limit;
        let mut items = Vec::new(&env);
        let mut next_cursor: u32 = 0;
        let take = if has_next {
            staging.len() - 1
        } else {
            staging.len()
        };

        for i in 0..take {
            if let Some((_, bill)) = staging.get(i) {
                items.push_back(bill);
            }
        }
        if has_next {
            if let Some((id, _)) = staging.get(take - 1) {
                next_cursor = id;
            }
        }

        let count = items.len();
        ArchivedBillPage {
            items,
            next_cursor,
            count,
        }
    }

    pub fn get_archived_bill(env: Env, bill_id: u32) -> Option<ArchivedBill> {
        let archived: Map<u32, ArchivedBill> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_BILL"))
            .unwrap_or_else(|| Map::new(&env));
        archived.get(bill_id)
    }

    // -----------------------------------------------------------------------
    // Remaining operations (unchanged)
    // -----------------------------------------------------------------------

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
            schedule_id: None,
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

    pub fn batch_pay_bills(env: Env, caller: Address, bill_ids: Vec<u32>) -> Result<u32, Error> {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_BILL)?;
        if bill_ids.len() > (MAX_BATCH_SIZE as usize).try_into().unwrap() {
            return Err(Error::BatchTooLarge);
        }
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

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

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
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env, String,
    };

    fn make_env() -> Env {
        Env::default()
    }

    /// Create `count` bills with a static name. Returns their IDs.
    /// Due dates are set in the future (timestamp + offset) so they are NOT overdue.
    fn setup_bills(
        env: &Env,
        client: &BillPaymentsClient,
        owner: &Address,
        count: u32,
    ) -> Vec<u32> {
        let mut ids = Vec::new(env);
        for i in 0..count {
            // Soroban client methods return the value directly (not Result)
            let id = client.create_bill(
                owner,
                &String::from_str(env, "Test Bill"),
                &(100i128 * (i as i128 + 1)),
                &(env.ledger().timestamp() + 86400 * (i as u64 + 1)), // future due date
                &false,
                &0,
            );
            ids.push_back(id);
        }
        ids
    }

    // --- get_unpaid_bills ---

    #[test]
    fn test_get_unpaid_bills_empty() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        let page = client.get_unpaid_bills(&owner, &0, &0);
        assert_eq!(page.count, 0);
        assert_eq!(page.next_cursor, 0);
        assert_eq!(page.items.len(), 0);
    }

    #[test]
    fn test_get_unpaid_bills_single_page() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        setup_bills(&env, &client, &owner, 5);

        let page = client.get_unpaid_bills(&owner, &0, &10);
        assert_eq!(page.count, 5);
        assert_eq!(page.next_cursor, 0);
    }

    #[test]
    fn test_get_unpaid_bills_multiple_pages() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        setup_bills(&env, &client, &owner, 7);

        let page1 = client.get_unpaid_bills(&owner, &0, &3);
        assert_eq!(page1.count, 3);
        assert!(page1.next_cursor > 0, "expected a next cursor");

        let page2 = client.get_unpaid_bills(&owner, &page1.next_cursor, &3);
        assert_eq!(page2.count, 3);
        assert!(page2.next_cursor > 0);

        let page3 = client.get_unpaid_bills(&owner, &page2.next_cursor, &3);
        assert_eq!(page3.count, 1);
        assert_eq!(page3.next_cursor, 0);
    }

    #[test]
    fn test_get_unpaid_bills_excludes_paid() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        let ids = setup_bills(&env, &client, &owner, 4);
        // pay_bill returns () — do NOT call .unwrap() on it
        let second_id = ids.get(1).unwrap();
        client.pay_bill(&owner, &second_id);

        let page = client.get_unpaid_bills(&owner, &0, &10);
        assert_eq!(page.count, 3);
    }

    #[test]
    fn test_get_unpaid_bills_excludes_other_owner() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner_a = Address::generate(&env);
        let owner_b = Address::generate(&env);

        setup_bills(&env, &client, &owner_a, 3);
        setup_bills(&env, &client, &owner_b, 2);

        let page = client.get_unpaid_bills(&owner_a, &0, &10);
        assert_eq!(page.count, 3);
        for bill in page.items.iter() {
            assert_eq!(bill.owner, owner_a);
        }
    }

    // --- get_overdue_bills ---

    #[test]
    fn test_get_overdue_bills_not_overdue() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        // Future due dates — none overdue
        setup_bills(&env, &client, &owner, 3);
        let page = client.get_overdue_bills(&0, &10);
        assert_eq!(page.count, 0);
    }

    #[test]
    fn test_get_overdue_bills_pagination() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        // Ledger starts at timestamp 0. Create bills with due_date = 0.
        // The check is `bill.due_date < current_time`. At timestamp 0, 0 < 0 is false.
        // So we advance the ledger to timestamp 1 after creating bills.
        for _ in 0..6u32 {
            client.create_bill(
                &owner,
                &String::from_str(&env, "Overdue Bill"),
                &100,
                &0, // due at epoch 0
                &false,
                &0,
            );
        }

        // Advance ledger so bills become overdue
        env.ledger().set_timestamp(1);

        let page1 = client.get_overdue_bills(&0, &4);
        assert_eq!(page1.count, 4);
        assert!(page1.next_cursor > 0);

        let page2 = client.get_overdue_bills(&page1.next_cursor, &4);
        assert_eq!(page2.count, 2);
        assert_eq!(page2.next_cursor, 0);
    }

    // --- get_all_bills_for_owner ---

    #[test]
    fn test_get_all_bills_for_owner_includes_paid() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        let ids = setup_bills(&env, &client, &owner, 5);
        let first_id = ids.get(0).unwrap();
        // pay_bill returns () — no .unwrap()
        client.pay_bill(&owner, &first_id);

        let page = client.get_all_bills_for_owner(&owner, &0, &10);
        assert_eq!(page.count, 5); // both paid and unpaid
    }

    // --- limit clamping ---

    #[test]
    fn test_limit_zero_uses_default() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        setup_bills(&env, &client, &owner, 3);
        let page = client.get_unpaid_bills(&owner, &0, &0);
        assert_eq!(page.count, 3);
    }

    #[test]
    fn test_limit_clamped_to_max() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        setup_bills(&env, &client, &owner, 55);
        let page = client.get_unpaid_bills(&owner, &0, &9999);
        assert_eq!(page.count, MAX_PAGE_LIMIT);
        assert!(page.next_cursor > 0);
    }

    // --- archived bill pagination ---

    #[test]
    fn test_get_archived_bills_pagination() {
        let env = make_env();
        env.mock_all_auths();
        let cid = env.register_contract(None, BillPayments);
        let client = BillPaymentsClient::new(&env, &cid);
        let owner = Address::generate(&env);

        // set_pause_admin returns () — no .unwrap()
        client.set_pause_admin(&owner, &owner);

        let ids = setup_bills(&env, &client, &owner, 6);
        // pay all — pay_bill returns ()
        for bill_id in ids.iter() {
            client.pay_bill(&owner, &bill_id);
        }
        // archive_paid_bills returns u32 (count) — no .unwrap()
        client.archive_paid_bills(&owner, &u64::MAX);

        let page1 = client.get_archived_bills(&owner, &0, &4);
        assert_eq!(page1.count, 4);
        assert!(page1.next_cursor > 0);

        let page2 = client.get_archived_bills(&owner, &page1.next_cursor, &4);
        assert_eq!(page2.count, 2);
        assert_eq!(page2.next_cursor, 0);
    }
}
