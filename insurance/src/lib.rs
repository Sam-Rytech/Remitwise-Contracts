#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Map, String, Vec,
};

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

/// Insurance policy data structure with owner tracking for access control
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

/// Schedule for automatic premium payments
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

/// Events emitted by the contract for audit trail
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
    /// Create a new insurance policy
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner (must authorize)
    /// * `name` - Name of the policy
    /// * `coverage_type` - Type of coverage (e.g., "health", "emergency")
    /// * `monthly_premium` - Monthly premium amount (must be positive)
    /// * `coverage_amount` - Total coverage amount (must be positive)
    ///
    /// # Returns
    /// The ID of the created policy
    ///
    /// # Panics
    /// - If owner doesn't authorize the transaction
    /// - If monthly_premium is not positive
    /// - If coverage_amount is not positive
    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: String,
        monthly_premium: i128,
        coverage_amount: i128,
    ) -> u32 {
        // Access control: require owner authorization
        owner.require_auth();

        // Input validation
        if monthly_premium <= 0 {
            panic!("Monthly premium must be positive");
        }
        if coverage_amount <= 0 {
            panic!("Coverage amount must be positive");
        }

        // Extend storage TTL
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

        // Set next payment date to 30 days from now
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

        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PolicyCreated),
            (next_id, policy_owner),
        );

        next_id
    }

    /// Pay monthly premium for a policy
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the policy owner)
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// True if payment was successful
    ///
    /// # Panics
    /// - If caller is not the policy owner
    /// - If policy is not found
    /// - If policy is not active
    pub fn pay_premium(env: Env, caller: Address, policy_id: u32) -> bool {
        // Access control: require caller authorization
        caller.require_auth();

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies.get(policy_id).expect("Policy not found");

        // Access control: verify caller is the owner
        if policy.owner != caller {
            panic!("Only the policy owner can pay premiums");
        }

        if !policy.active {
            panic!("Policy is not active");
        }

        // Update next payment date to 30 days from now
        policy.next_payment_date = env.ledger().timestamp() + (30 * 86400);

        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PremiumPaid),
            (policy_id, caller),
        );

        true
    }

    /// Get a policy by ID
    ///
    /// # Arguments
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// InsurancePolicy struct or None if not found
    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        policies.get(policy_id)
    }

    /// Get all active policies for a specific owner
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner
    ///
    /// # Returns
    /// Vec of active InsurancePolicy structs belonging to the owner
    pub fn get_active_policies(env: Env, owner: Address) -> Vec<InsurancePolicy> {
        let policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, policy) in policies.iter() {
            if policy.active && policy.owner == owner {
                result.push_back(policy);
            }
        }
        result
    }

    /// Get total monthly premium for all active policies of an owner
    ///
    /// # Arguments
    /// * `owner` - Address of the policy owner
    ///
    /// # Returns
    /// Total monthly premium amount for the owner's active policies
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

    /// Deactivate a policy
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the policy owner)
    /// * `policy_id` - ID of the policy
    ///
    /// # Returns
    /// True if deactivation was successful
    ///
    /// # Panics
    /// - If caller is not the policy owner
    /// - If policy is not found
    pub fn deactivate_policy(env: Env, caller: Address, policy_id: u32) -> bool {
        // Access control: require caller authorization
        caller.require_auth();

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let mut policies: Map<u32, InsurancePolicy> = env
            .storage()
            .instance()
            .get(&symbol_short!("POLICIES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut policy = policies.get(policy_id).expect("Policy not found");

        // Access control: verify caller is the owner
        if policy.owner != caller {
            panic!("Only the policy owner can deactivate this policy");
        }

        policy.active = false;
        policies.set(policy_id, policy);
        env.storage()
            .instance()
            .set(&symbol_short!("POLICIES"), &policies);

        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("insure"), InsuranceEvent::PolicyDeactivated),
            (policy_id, caller),
        );

        true
    }

    /// Extend the TTL of instance storage
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Create a schedule for automatic premium payments
    pub fn create_premium_schedule(
        env: Env,
        owner: Address,
        policy_id: u32,
        next_due: u64,
        interval: u64,
    ) -> u32 {
        owner.require_auth();

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

    /// Modify a premium schedule
    pub fn modify_premium_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
        next_due: u64,
        interval: u64,
    ) -> bool {
        caller.require_auth();

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

    /// Cancel a premium schedule
    pub fn cancel_premium_schedule(env: Env, caller: Address, schedule_id: u32) -> bool {
        caller.require_auth();

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

    /// Execute due premium schedules (public, callable by anyone - keeper pattern)
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

    /// Get all premium schedules for an owner
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

    /// Get a specific premium schedule
    pub fn get_premium_schedule(env: Env, schedule_id: u32) -> Option<PremiumSchedule> {
        let schedules: Map<u32, PremiumSchedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("PREM_SCH"))
            .unwrap_or_else(|| Map::new(&env));

        schedules.get(schedule_id)
    }
}

mod test;
