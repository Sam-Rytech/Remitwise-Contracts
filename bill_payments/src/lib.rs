#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Map, String,
    Vec,
};

mod schedule;
use schedule::{Schedule, ScheduleEvent};

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

/// Bill data structure with owner tracking for access control
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

/// Events emitted by the contract for audit trail
#[contracttype]
#[derive(Clone)]
pub enum BillEvent {
    Created,
    Paid,
}

#[contract]
pub struct BillPayments;

#[contractimpl]
impl BillPayments {
    /// Create a new bill
    ///
    /// # Arguments
    /// * `owner` - Address of the bill owner (must authorize)
    /// * `name` - Name of the bill (e.g., "Electricity", "School Fees")
    /// * `amount` - Amount to pay (must be positive)
    /// * `due_date` - Due date as Unix timestamp
    /// * `recurring` - Whether this is a recurring bill
    /// * `frequency_days` - Frequency in days for recurring bills (must be > 0 if recurring)
    ///
    /// # Returns
    /// The ID of the created bill
    ///
    /// # Errors
    /// * `InvalidAmount` - If amount is zero or negative
    /// * `InvalidFrequency` - If recurring is true but frequency_days is 0
    pub fn create_bill(
        env: Env,
        owner: Address,
        name: String,
        amount: i128,
        due_date: u64,
        recurring: bool,
        frequency_days: u32,
    ) -> Result<u32, Error> {
        // Access control: require owner authorization
        owner.require_auth();

        // Validate inputs
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if recurring && frequency_days == 0 {
            return Err(Error::InvalidFrequency);
        }

        // Extend storage TTL
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

        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("bill"), BillEvent::Created),
            (next_id, bill_owner),
        );

        Ok(next_id)
    }

    /// Mark a bill as paid
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the bill owner)
    /// * `bill_id` - ID of the bill
    ///
    /// # Returns
    /// Ok(()) if payment was successful
    ///
    /// # Errors
    /// * `BillNotFound` - If bill with given ID doesn't exist
    /// * `BillAlreadyPaid` - If bill is already marked as paid
    /// * `Unauthorized` - If caller is not the bill owner
    pub fn pay_bill(env: Env, caller: Address, bill_id: u32) -> Result<(), Error> {
        // Access control: require caller authorization
        caller.require_auth();

        // Extend storage TTL
        Self::extend_instance_ttl(&env);
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut bill = bills.get(bill_id).ok_or(Error::BillNotFound)?;

        // Access control: verify caller is the owner
        if bill.owner != caller {
            return Err(Error::Unauthorized);
        }

        if bill.paid {
            return Err(Error::BillAlreadyPaid);
        }

        let current_time = env.ledger().timestamp();
        bill.paid = true;
        bill.paid_at = Some(current_time);

        // If recurring, create next bill
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

        bills.set(bill_id, bill);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);

        // Emit event for audit trail
        env.events()
            .publish((symbol_short!("bill"), BillEvent::Paid), (bill_id, caller));

        Ok(())
    }

    /// Get a bill by ID
    ///
    /// # Arguments
    /// * `bill_id` - ID of the bill
    ///
    /// # Returns
    /// Bill struct or None if not found
    pub fn get_bill(env: Env, bill_id: u32) -> Option<Bill> {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        bills.get(bill_id)
    }

    /// Get all unpaid bills for a specific owner
    ///
    /// # Arguments
    /// * `owner` - Address of the bill owner
    ///
    /// # Returns
    /// Vec of unpaid Bill structs belonging to the owner
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

    /// Get all overdue unpaid bills
    ///
    /// # Returns
    /// Vec of unpaid bills that are past their due date
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

    /// Get total amount of unpaid bills for a specific owner
    ///
    /// # Arguments
    /// * `owner` - Address of the bill owner
    ///
    /// # Returns
    /// Total amount of all unpaid bills belonging to the owner
    pub fn get_total_unpaid(env: Env, owner: Address) -> i128 {
        let mut total = 0i128;
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        for (_, bill) in bills.iter() {
            if !bill.paid && bill.owner == owner {
                total += bill.amount;
            }
        }
        total
    }

    /// Cancel/delete a bill
    ///
    /// # Arguments
    /// * `bill_id` - ID of the bill to cancel
    ///
    /// # Returns
    /// Ok(()) if cancellation was successful
    ///
    /// # Errors
    /// * `BillNotFound` - If bill with given ID doesn't exist
    pub fn cancel_bill(env: Env, bill_id: u32) -> Result<(), Error> {
        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        if bills.get(bill_id).is_none() {
            return Err(Error::BillNotFound);
        }

        bills.remove(bill_id);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);

        Ok(())
    }

    /// Get all bills (paid and unpaid)
    ///
    /// # Returns
    /// Vec of all Bill structs
    pub fn get_all_bills(env: Env) -> Vec<Bill> {
        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, bill) in bills.iter() {
            result.push_back(bill);
        }
        result
    }

    /// Extend the TTL of instance storage
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Create a schedule for automatic bill payment
    ///
    /// # Arguments
    /// * `owner` - Address of the schedule owner (must authorize)
    /// * `bill_id` - ID of the bill to schedule
    /// * `next_due` - Next execution timestamp
    /// * `interval` - Interval in seconds for recurring schedules (0 for one-time)
    ///
    /// # Returns
    /// The ID of the created schedule
    pub fn create_schedule(
        env: Env,
        owner: Address,
        bill_id: u32,
        next_due: u64,
        interval: u64,
    ) -> Result<u32, Error> {
        owner.require_auth();

        let bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut bill = bills.get(bill_id).ok_or(Error::BillNotFound)?;

        if bill.owner != owner {
            return Err(Error::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(Error::InvalidAmount);
        }

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        let next_schedule_id = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_SCH"))
            .unwrap_or(0u32)
            + 1;

        let schedule = Schedule {
            id: next_schedule_id,
            owner: owner.clone(),
            next_due,
            interval,
            recurring: interval > 0,
            active: true,
            created_at: current_time,
            last_executed: None,
            missed_count: 0,
        };

        bill.schedule_id = Some(next_schedule_id);

        schedules.set(next_schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SCHEDULES"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_SCH"), &next_schedule_id);

        let mut bills_mut = bills;
        bills_mut.set(bill_id, bill);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills_mut);

        env.events().publish(
            (symbol_short!("schedule"), ScheduleEvent::Created),
            (next_schedule_id, owner),
        );

        Ok(next_schedule_id)
    }

    /// Modify an existing schedule
    pub fn modify_schedule(
        env: Env,
        caller: Address,
        schedule_id: u32,
        next_due: u64,
        interval: u64,
    ) -> Result<(), Error> {
        caller.require_auth();

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules.get(schedule_id).ok_or(Error::BillNotFound)?;

        if schedule.owner != caller {
            return Err(Error::Unauthorized);
        }

        let current_time = env.ledger().timestamp();
        if next_due <= current_time {
            return Err(Error::InvalidAmount);
        }

        schedule.next_due = next_due;
        schedule.interval = interval;
        schedule.recurring = interval > 0;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SCHEDULES"), &schedules);

        env.events().publish(
            (symbol_short!("schedule"), ScheduleEvent::Modified),
            (schedule_id, caller),
        );

        Ok(())
    }

    /// Cancel a schedule
    pub fn cancel_schedule(env: Env, caller: Address, schedule_id: u32) -> Result<(), Error> {
        caller.require_auth();

        Self::extend_instance_ttl(&env);

        let mut schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut schedule = schedules.get(schedule_id).ok_or(Error::BillNotFound)?;

        if schedule.owner != caller {
            return Err(Error::Unauthorized);
        }

        schedule.active = false;

        schedules.set(schedule_id, schedule);
        env.storage()
            .instance()
            .set(&symbol_short!("SCHEDULES"), &schedules);

        env.events().publish(
            (symbol_short!("schedule"), ScheduleEvent::Cancelled),
            (schedule_id, caller),
        );

        Ok(())
    }

    /// Execute due schedules (public, callable by anyone - keeper pattern)
    pub fn execute_due_schedules(env: Env) -> Vec<u32> {
        Self::extend_instance_ttl(&env);

        let current_time = env.ledger().timestamp();
        let mut executed = Vec::new(&env);

        let mut schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut bills: Map<u32, Bill> = env
            .storage()
            .instance()
            .get(&symbol_short!("BILLS"))
            .unwrap_or_else(|| Map::new(&env));

        for (schedule_id, mut schedule) in schedules.iter() {
            if !schedule.active || schedule.next_due > current_time {
                continue;
            }

            let bill_id = Self::find_bill_by_schedule(&bills, schedule_id);
            if let Some(bid) = bill_id {
                if let Some(mut bill) = bills.get(bid) {
                    if !bill.paid {
                        bill.paid = true;
                        bill.paid_at = Some(current_time);

                        if bill.recurring {
                            let next_due_date =
                                bill.due_date + (bill.frequency_days as u64 * 86400);
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

                        bills.set(bid, bill);

                        env.events().publish(
                            (symbol_short!("bill"), BillEvent::Paid),
                            (bid, schedule.owner.clone()),
                        );
                    }
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
                        (symbol_short!("schedule"), ScheduleEvent::Missed),
                        (schedule_id, missed),
                    );
                }
            } else {
                schedule.active = false;
            }

            schedules.set(schedule_id, schedule);
            executed.push_back(schedule_id);

            env.events().publish(
                (symbol_short!("schedule"), ScheduleEvent::Executed),
                schedule_id,
            );
        }

        env.storage()
            .instance()
            .set(&symbol_short!("SCHEDULES"), &schedules);
        env.storage()
            .instance()
            .set(&symbol_short!("BILLS"), &bills);

        executed
    }

    /// Get all schedules for an owner
    pub fn get_schedules(env: Env, owner: Address) -> Vec<Schedule> {
        let schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (_, schedule) in schedules.iter() {
            if schedule.owner == owner {
                result.push_back(schedule);
            }
        }
        result
    }

    /// Get a specific schedule
    pub fn get_schedule(env: Env, schedule_id: u32) -> Option<Schedule> {
        let schedules: Map<u32, Schedule> = env
            .storage()
            .instance()
            .get(&symbol_short!("SCHEDULES"))
            .unwrap_or_else(|| Map::new(&env));

        schedules.get(schedule_id)
    }

    fn find_bill_by_schedule(bills: &Map<u32, Bill>, schedule_id: u32) -> Option<u32> {
        for (bill_id, bill) in bills.iter() {
            if bill.schedule_id == Some(schedule_id) {
                return Some(bill_id);
            }
        }
        None
    }
}

mod test;
