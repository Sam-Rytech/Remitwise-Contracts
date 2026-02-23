#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Map, String, Symbol, Vec,
};

// Event topics
const POLICY_CREATED: Symbol = symbol_short!("created");
const PREMIUM_PAID: Symbol = symbol_short!("paid");
const POLICY_DEACTIVATED: Symbol = symbol_short!("deactive");

#[derive(Clone)]
#[contracttype]
pub struct PolicyCreatedEvent {
    pub policy_id: u32,
    pub name: String,
    pub coverage_type: String,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct PremiumPaidEvent {
    pub policy_id: u32,
    pub name: String,
    pub amount: i128,
    pub next_payment_date: u64,
    pub timestamp: u64,
}

#[derive(Clone)]
#[contracttype]
pub struct PolicyDeactivatedEvent {
    pub policy_id: u32,
    pub name: String,
    pub timestamp: u64,
}

const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 518400;

const CONTRACT_VERSION: u32 = 1;
const MAX_BATCH_SIZE: u32 = 50;

/// Pagination constants
pub const DEFAULT_PAGE_LIMIT: u32 = 20;
pub const MAX_PAGE_LIMIT: u32 = 50;

pub mod pause_functions {
    use soroban_sdk::{symbol_short, Symbol};
    pub const CREATE_POLICY: Symbol = symbol_short!("crt_pol");
    pub const PAY_PREMIUM: Symbol = symbol_short!("pay_prem");
    pub const DEACTIVATE: Symbol = symbol_short!("deact");
    pub const CREATE_SCHED: Symbol = symbol_short!("crt_sch");
    pub const MODIFY_SCHED: Symbol = symbol_short!("mod_sch");
    pub const CANCEL_SCHED: Symbol = symbol_short!("can_sch");
}

#[derive(Clone)]
#[contracttype]
pub struct InsurancePolicy {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub coverage_type: String,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub active: bool,
    pub next_payment_date: u64,
    pub schedule_id: Option<u32>,
}

/// Paginated result for insurance policy queries
#[contracttype]
#[derive(Clone)]
pub struct PolicyPage {
    /// Policies for this page
    pub items: Vec<InsurancePolicy>,
    /// Pass as `cursor` for the next page. 0 = no more pages.
    pub next_cursor: u32,
    /// Number of items returned
    pub count: u32,
}

#[contracttype]
#[derive(Clone)]
pub struct PremiumSchedule {
    pub id: u32,
    pub owner: Address,
    pub policy_id: u32,
    pub next_due: u64,
    pub interval: u64,
    pub recurring: bool,
    pub active: bool,
    pub created_at: u64,
    pub last_executed: Option<u64>,
    pub missed_count: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum InsuranceEvent {
    PolicyCreated,
    PremiumPaid,
    PolicyDeactivated,
    ScheduleCreated,
    ScheduleExecuted,
    ScheduleMissed,
    ScheduleModified,
    ScheduleCancelled,
}

#[contract]
pub struct Insurance;

#[contractimpl]
impl Insurance {
    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn clamp_limit(limit: u32) -> u32 {
        if limit == 0 {
            DEFAULT_PAGE_LIMIT
        } else if limit > MAX_PAGE_LIMIT {
            MAX_PAGE_LIMIT
        } else {
            limit
        }
    }

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
    fn require_not_paused(env: &Env, func: Symbol) {
        if Self::get_global_paused(env) {
            panic!("Contract is paused");
        }
        if Self::is_function_paused(env, func) {
            panic!("Function is paused");
        }
    }

    // -----------------------------------------------------------------------
    // Pause / upgrade (unchanged)
    // -----------------------------------------------------------------------

    pub fn set_pause_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current = Self::get_pause_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    panic!("Unauthorized");
                }
            }
            Some(admin) if admin != caller => panic!("Unauthorized"),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSE_ADM"), &new_admin);
    }
    pub fn pause(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            panic!("Unauthorized");
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &true);
        env.events()
            .publish((symbol_short!("insure"), symbol_short!("paused")), ());
    }
    pub fn unpause(env: Env, caller: Address) {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            panic!("Unauthorized");
        }
        let unpause_at: Option<u64> = env.storage().instance().get(&symbol_short!("UNP_AT"));
        if let Some(at) = unpause_at {
            if env.ledger().timestamp() < at {
                panic!("Time-locked unpause not yet reached");
            }
            env.storage().instance().remove(&symbol_short!("UNP_AT"));
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &false);
        env.events()
            .publish((symbol_short!("insure"), symbol_short!("unpaused")), ());
    }
    pub fn pause_function(env: Env, caller: Address, func: Symbol) {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            panic!("Unauthorized");
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
    }
    pub fn unpause_function(env: Env, caller: Address, func: Symbol) {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).expect("No pause admin set");
        if admin != caller {
            panic!("Unauthorized");
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
    }
    pub fn emergency_pause_all(env: Env, caller: Address) {
        Self::pause(env.clone(), caller.clone());
        for func in [
            pause_functions::CREATE_POLICY,
            pause_functions::PAY_PREMIUM,
            pause_functions::DEACTIVATE,
            pause_functions::CREATE_SCHED,
            pause_functions::MODIFY_SCHED,
            pause_functions::CANCEL_SCHED,
        ] {
            Self::pause_function(env.clone(), caller.clone(), func);
        }
    }
    pub fn is_paused(env: Env) -> bool {
        Self::get_global_paused(&env)
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
    pub fn set_upgrade_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current = Self::get_upgrade_admin(&env);
        match current {
            None => {
                if caller != new_admin {
                    panic!("Unauthorized");
                }
            }
            Some(adm) if adm != caller => panic!("Unauthorized"),
            _ => {}
        }
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
    }
    pub fn set_version(env: Env, caller: Address, new_version: u32) {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).expect("No upgrade admin set");
        if admin != caller {
            panic!("Unauthorized");
        }
        let prev = Self::get_version(env.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("VERSION"), &new_version);
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("upgraded")),
            (prev, new_version),
        );
    }

    // -----------------------------------------------------------------------
    // Core policy operations (unchanged)
    // -----------------------------------------------------------------------

    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: String,
        monthly_premium: i128,
        coverage_amount: i128,
    ) -> u32 {
        owner.require_auth();
        Self::require_not_paused(&env, pause_functions::CREATE_POLICY);

        if monthly_premium <= 0 {
            panic!("Monthly premium must be positive");
        }
        if coverage_amount <= 0 {
            panic!("Coverage amount must be positive");
        }

        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let next_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_ID"))
            .unwrap_or(0u32)
            + 1;

        let next_payment_date = env.ledger().timestamp() + (30 * 86400);

        let policy = InsurancePolicy {
            id: next_id,
            owner: owner.clone(),
            name: name.clone(),
            coverage_type: coverage_type.clone(),
            monthly_premium,
            coverage_amount,
            active: true,
            next_payment_date,
            schedule_id: None,
        };

        let policy_owner = policy.owner.clone();
        policies.set(next_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_ID"), &next_id);

        let event = PolicyCreatedEvent {
            policy_id: next_id,
            name: name.clone(),
            coverage_type: coverage_type.clone(),
            monthly_premium,
            coverage_amount,
            timestamp: env.ledger().timestamp(),
        };
        env.events().publish((POLICY_CREATED,), event);
        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PolicyCreated),
            (next_id, policy_owner),
        );

        next_id
    }

    pub fn pay_premium(env: Env, caller: Address, policy_id: u32) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_PREMIUM);
        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies.get(policy_id).expect("Policy not found");

        if policy.owner != caller {
            panic!("Only the policy owner can pay premiums");
        }
        if !policy.active {
            panic!("Policy is not active");
        }

        policy.next_payment_date = env.ledger().timestamp() + (30 * 86400);

        let event = PremiumPaidEvent {
            policy_id,
            name: policy.name.clone(),
            amount: policy.monthly_premium,
            next_payment_date: policy.next_payment_date,
            timestamp: env.ledger().timestamp(),
        };
        env.events().publish((PREMIUM_PAID,), event);

        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
            (policy_id, caller),
        );

        true
    }

    pub fn batch_pay_premiums(env: Env, caller: Address, policy_ids: Vec<u32>) -> u32 {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::PAY_PREMIUM);
        if policy_ids.len() > MAX_BATCH_SIZE {
            panic!("Batch too large");
        }
        let policies_map: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));
        for id in policy_ids.iter() {
            let policy = policies_map.get(id).expect("Policy not found");
            if policy.owner != caller {
                panic!("Not owner of all policies");
            }
            if !policy.active {
                panic!("Policy not active");
            }
        }
        Self::extend_instance_ttl(&env);
        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));
        let current_time = env.ledger().timestamp();
        let mut paid_count = 0u32;
        for id in policy_ids.iter() {
            let mut policy = policies.get(id).expect("Policy not found");
            if policy.owner != caller || !policy.active {
                panic!("Batch validation failed");
            }
            policy.next_payment_date = current_time + (30 * 86400);
            let event = PremiumPaidEvent {
                policy_id: id,
                name: policy.name.clone(),
                amount: policy.monthly_premium,
                next_payment_date: policy.next_payment_date,
                timestamp: current_time,
            };
            env.events().publish((PREMIUM_PAID,), event);
            env.events().publish(
                (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
                (id, caller.clone()),
            );
            policies.set(id, policy);
            paid_count += 1;
        }
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("batch_pay")),
            (paid_count, caller),
        );
        paid_count
    }

    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));
        policies.get(policy_id)
    }

    // -----------------------------------------------------------------------
    // PAGINATED LIST QUERIES  (new in this version)
    // -----------------------------------------------------------------------

    /// Get a page of ACTIVE policies for `owner`.
    ///
    /// # Arguments
    /// * `owner`  – whose policies to return
    /// * `cursor` – start after this policy ID (pass 0 for the first page)
    /// * `limit`  – max items per page (0 → DEFAULT_PAGE_LIMIT, capped at MAX_PAGE_LIMIT)
    ///
    /// # Returns
    /// `PolicyPage { items, next_cursor, count }`.
    /// `next_cursor == 0` means no more pages.
    pub fn get_active_policies(env: Env, owner: Address, cursor: u32, limit: u32) -> PolicyPage {
        let limit = Self::clamp_limit(limit);
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        let mut next_cursor: u32 = 0;
        let mut collected: u32 = 0;

        for (id, policy) in policies.iter() {
            if id <= cursor {
                continue;
            }
            if !policy.active || policy.owner != owner {
                continue;
            }
            if collected < limit {
                result.push_back(policy);
                collected += 1;
                next_cursor = id; // ← track last returned ID as we go
            } else {
                break; // ← stop without touching next_cursor
            }
        }

        // Then reset next_cursor to 0 if we didn't fill the page (no more items)
        if collected < limit {
            next_cursor = 0;
        }

        PolicyPage {
            items: result,
            next_cursor,
            count: collected,
        }
    }

    /// Get a page of ALL policies (active + inactive) for `owner`.
    ///
    /// Same cursor/limit semantics as `get_active_policies`.
    pub fn get_all_policies_for_owner(
        env: Env,
        owner: Address,
        cursor: u32,
        limit: u32,
    ) -> PolicyPage {
        owner.require_auth();
        let limit = Self::clamp_limit(limit);
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        let mut next_cursor: u32 = 0;
        let mut collected: u32 = 0;

        for (id, policy) in policies.iter() {
            if id <= cursor {
                continue;
            }
            if policy.owner != owner {
                continue;
            }
            if collected < limit {
                result.push_back(policy);
                collected += 1;
            } else {
                next_cursor = id;
                break;
            }
        }

        PolicyPage {
            items: result,
            next_cursor,
            count: collected,
        }
    }

    pub fn get_total_monthly_premium(env: Env, owner: Address) -> i128 {
        let mut total = 0i128;
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));
        for (_, policy) in policies.iter() {
            if policy.active && policy.owner == owner {
                total += policy.monthly_premium;
            }
        }
        total
    }

    pub fn deactivate_policy(env: Env, caller: Address, policy_id: u32) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::DEACTIVATE);
        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies.get(policy_id).expect("Policy not found");

        if policy.owner != caller {
            panic!("Only the policy owner can deactivate this policy");
        }

        policy.active = false;

        let event = PolicyDeactivatedEvent {
            policy_id,
            name: policy.name.clone(),
            timestamp: env.ledger().timestamp(),
        };
        env.events().publish((POLICY_DEACTIVATED,), event);
        env.events().publish(
            (symbol_short!("insuranc"), InsuranceEvent::PolicyDeactivated),
            (policy_id, caller),
        );

        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);
        true
    }

    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    // -----------------------------------------------------------------------
    // Schedule operations (unchanged)
    // -----------------------------------------------------------------------

    pub fn create_premium_schedule(
        env: Env,
        owner: Address,
        policy_id: u32,
        next_due: u64,
        interval: u64,
    ) -> u32 {
        owner.require_auth();
        Self::require_not_paused(&env, pause_functions::CREATE_SCHED);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies.get(policy_id).expect("Policy not found");

        if policy.owner != owner {
            panic!("Only the policy owner can create schedules");
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            panic!("Next due date must be in the future");
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let next_schedule_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_PSCH"))
            .unwrap_or(0u32)
            + 1;

        let schedule = PremiumSchedule {
            id: next_schedule_id,
            owner: owner.clone(),
            policy_id,
            next_due,
            interval,
            recurring: interval > 0,
            active: true,
            created_at: current_time,
            last_executed: None,
            missed_count: 0,
        };

        policy.schedule_id = Some(next_schedule_id);

        schedules.set(next_schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_PSCH"), &next_schedule_id);

        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleCreated),
            (next_schedule_id, owner),
        );

        next_schedule_id
    }

    pub fn modify_premium_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
        next_due: u64,
        interval: u64,
    ) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::MODIFY_SCHED);

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            panic!("Next due date must be in the future");
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules.get(schedule_id).expect("Schedule not found");

        if schedule.owner != caller {
            panic!("Only the schedule owner can modify it");
        }

        schedule.next_due = next_due;
        schedule.interval = interval;
        schedule.recurring = interval > 0;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleModified),
            (schedule_id, caller),
        );

        true
    }

    pub fn cancel_premium_schedule(env: Env, caller: Address, schedule_id: u32) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env, pause_functions::CANCEL_SCHED);
        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules.get(schedule_id).expect("Schedule not found");

        if schedule.owner != caller {
            panic!("Only the schedule owner can cancel it");
        }

        schedule.active = false;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);

        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::ScheduleCancelled),
            (schedule_id, caller),
        );

        true
    }

    pub fn execute_due_premium_schedules(env: Env) -> Vec<u32> {
        Self::extend_instance_ttl(&env);

        let current_time = env.ledger().timestamp();
        let mut executed = Vec::new(&env);

        let mut schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        for (schedule_id, mut schedule) in schedules.iter() {
            if !schedule.active || schedule.next_due > current_time {
                continue;
            }

            if let Some(mut policy) = policies.get(schedule.policy_id) {
                if policy.active {
                    policy.next_payment_date = current_time + (30 * 86400);
                    policies.set(schedule.policy_id, policy.clone());
                    env.events().publish(
                        (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
                        (schedule.policy_id, policy.owner),
                    );
                }
            }

            schedule.last_executed = Some(current_time);

            if schedule.recurring && schedule.interval > 0 {
                let mut missed = 0u32;
                let mut next = schedule.next_due + schedule.interval;
                while next <= current_time {
                    missed += 1;
                    next += schedule.interval;
                }
                schedule.missed_count += missed;
                schedule.next_due = next;

                if missed > 0 {
                    env.events().publish(
                        (symbol_short!("insure"), InsuranceEvent::ScheduleMissed),
                        (schedule_id, missed),
                    );
                }
            } else {
                schedule.active = false;
            }

            schedules.set(schedule_id, schedule);
            executed.push_back(schedule_id);

            env.events().publish(
                (symbol_short!("insure"), InsuranceEvent::ScheduleExecuted),
                schedule_id,
            );
        }

        env.storage()
            .instance()
            .set(&symbol_short!("PREM_SCH"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        executed
    }

    pub fn get_premium_schedules(env: Env, owner: Address) -> Vec<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));
        let mut result = Vec::new(&env);
        for (_, schedule) in schedules.iter() {
            if schedule.owner == owner {
                result.push_back(schedule);
            }
        }
        result
    }

    pub fn get_premium_schedule(env: Env, schedule_id: u32) -> Option<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));
        schedules.get(schedule_id)
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------
#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Events},
        Env, String,
    };

    fn make_env() -> Env {
        Env::default()
    }

    fn setup_policies(
        env: &Env,
        client: &InsuranceClient,
        owner: &Address,
        count: u32,
    ) -> Vec<u32> {
        let mut ids = Vec::new(env);
        for i in 0..count {
            let id = client.create_policy(
                owner,
                &String::from_str(env, "Policy"),
                &String::from_str(env, "health"),
                &(50i128 * (i as i128 + 1)),
                &(10000i128 * (i as i128 + 1)),
            );
            ids.push_back(id);
        }
        ids
    }

    // --- get_active_policies ---

    #[test]
    fn test_get_active_policies_empty() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        let page = client.get_active_policies(&owner, &0, &0);
        assert_eq!(page.count, 0);
        assert_eq!(page.next_cursor, 0);
    }

    #[test]
    fn test_get_active_policies_single_page() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        setup_policies(&env, &client, &owner, 5);

        let page = client.get_active_policies(&owner, &0, &10);
        assert_eq!(page.count, 5);
        assert_eq!(page.next_cursor, 0);
    }

    #[test]
    fn test_get_active_policies_multiple_pages() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        setup_policies(&env, &client, &owner, 7);

        let page1 = client.get_active_policies(&owner, &0, &3);
        assert_eq!(page1.count, 3);
        assert!(page1.next_cursor > 0);

        let page2 = client.get_active_policies(&owner, &page1.next_cursor, &3);
        assert_eq!(page2.count, 3);
        assert!(page2.next_cursor > 0);

        let page3 = client.get_active_policies(&owner, &page2.next_cursor, &3);
        assert_eq!(page3.count, 1);
        assert_eq!(page3.next_cursor, 0);
    }

    #[test]
    fn test_get_active_policies_excludes_inactive() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        let ids = setup_policies(&env, &client, &owner, 4);
        // Deactivate policy #2
        client.deactivate_policy(&owner, &ids.get(1).unwrap());

        let page = client.get_active_policies(&owner, &0, &10);
        assert_eq!(page.count, 3); // only 3 active
        for p in page.items.iter() {
            assert!(p.active, "only active policies should be returned");
        }
    }

    #[test]
    fn test_get_active_policies_multi_owner_isolation() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner_a = Address::generate(&env);
        let owner_b = Address::generate(&env);

        setup_policies(&env, &client, &owner_a, 3);
        setup_policies(&env, &client, &owner_b, 5);

        let page = client.get_active_policies(&owner_a, &0, &20);
        assert_eq!(page.count, 3);
        for p in page.items.iter() {
            assert_eq!(p.owner, owner_a);
        }
    }

    #[test]
    fn test_get_all_policies_for_owner_includes_inactive() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        let ids = setup_policies(&env, &client, &owner, 4);
        client.deactivate_policy(&owner, &ids.get(0).unwrap());
        client.deactivate_policy(&owner, &ids.get(2).unwrap());

        let page = client.get_all_policies_for_owner(&owner, &0, &10);
        assert_eq!(page.count, 4); // all 4 regardless of active status
    }

    // --- limit clamping ---

    #[test]
    fn test_limit_zero_uses_default() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        setup_policies(&env, &client, &owner, 3);
        let page = client.get_active_policies(&owner, &0, &0);
        assert_eq!(page.count, 3);
    }

    #[test]
    fn test_limit_clamped_to_max() {
        let env = make_env();
        env.mock_all_auths();
        let id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &id);
        let owner = Address::generate(&env);

        setup_policies(&env, &client, &owner, 55);
        let page = client.get_active_policies(&owner, &0, &9999);
        assert_eq!(page.count, MAX_PAGE_LIMIT);
        assert!(page.next_cursor > 0);
    }

    // --- existing event tests (unchanged) ---

    #[test]
    fn test_create_policy_emits_event() {
        let env = make_env();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Health Insurance"),
            &String::from_str(&env, "health"),
            &100,
            &50000,
        );
        assert_eq!(policy_id, 1);

        let events = env.events().all();
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn test_pay_premium_emits_event() {
        let env = make_env();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Emergency Coverage"),
            &String::from_str(&env, "emergency"),
            &75,
            &25000,
        );
        let events_before = env.events().all().len();

        let result = client.pay_premium(&owner, &policy_id);
        assert!(result);

        let events_after = env.events().all().len();
        assert_eq!(events_after - events_before, 2);
    }

    #[test]
    fn test_policy_lifecycle_emits_all_events() {
        let env = make_env();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, Insurance);
        let client = InsuranceClient::new(&env, &contract_id);
        let owner = Address::generate(&env);

        let policy_id = client.create_policy(
            &owner,
            &String::from_str(&env, "Complete Lifecycle"),
            &String::from_str(&env, "health"),
            &150,
            &75000,
        );

        client.pay_premium(&owner, &policy_id);
        client.deactivate_policy(&owner, &policy_id);

        let events = env.events().all();
        assert_eq!(events.len(), 6);
    }
}
