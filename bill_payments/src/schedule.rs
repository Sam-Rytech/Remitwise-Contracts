#![allow(dead_code)]
use soroban_sdk::{contracttype, Address};

/// Schedule data structure for time-based operations
#[derive(Clone)]
#[contracttype]
pub struct Schedule {
    pub id: u32,
    pub owner: Address,
    pub next_due: u64,
    pub interval: u64,
    pub recurring: bool,
    pub active: bool,
    pub created_at: u64,
    pub last_executed: Option<u64>,
    pub missed_count: u32,
}

/// Schedule event types
#[contracttype]
#[derive(Clone)]
pub enum ScheduleEvent {
    Created,
    Executed,
    Missed,
    Modified,
    Cancelled,
}
