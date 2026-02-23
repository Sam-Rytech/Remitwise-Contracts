use soroban_sdk::{symbol_short, Env, IntoVal, Symbol, Val};

#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum EventCategory {
    Transaction = 0,
    State = 1,
    Alert = 2,
    System = 3,
    Access = 4,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum EventPriority {
    Low = 0,
    Medium = 1,
    High = 2,
}

impl EventCategory {
    pub fn to_u32(self) -> u32 {
        self as u32
    }
}
impl EventPriority {
    pub fn to_u32(self) -> u32 {
        self as u32
    }
}

pub struct RemitwiseEvents;

impl RemitwiseEvents {
    pub fn emit<T: IntoVal<Env, Val>>(
        e: &Env,
        category: EventCategory,
        priority: EventPriority,
        action: Symbol,
        data: T,
    ) {
        let topics = (
            symbol_short!("Remitwise"),
            category.to_u32(),
            priority.to_u32(),
            action,
        );
        e.events().publish(topics, data);
    }

    pub fn emit_batch(e: &Env, category: EventCategory, action: Symbol, count: u32) {
        let topics = (
            symbol_short!("Remitwise"),
            category.to_u32(),
            EventPriority::Low.to_u32(),
            symbol_short!("batch"),
        );
        let data = (action, count);
        e.events().publish(topics, data);
    }
}
