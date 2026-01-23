#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, vec, Address, Env, Vec};

// Storage TTL constants
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

/// Split configuration with owner tracking for access control
#[derive(Clone)]
#[contracttype]
pub struct SplitConfig {
    pub owner: Address,
    pub spending_percent: u32,
    pub savings_percent: u32,
    pub bills_percent: u32,
    pub insurance_percent: u32,
    pub initialized: bool,
}

/// Events emitted by the contract for audit trail
#[contracttype]
#[derive(Clone)]
pub enum SplitEvent {
    Initialized,
    Updated,
    Calculated,
}

#[contract]
pub struct RemittanceSplit;

#[contractimpl]
impl RemittanceSplit {
    /// Set or update the split percentages used to allocate remittances.
    ///
    /// # Arguments
    /// * `spending_percent` - Percent allocated to spending
    /// * `savings_percent` - Percent allocated to savings
    /// * `bills_percent` - Percent allocated to bills
    /// * `insurance_percent` - Percent allocated to insurance
    ///
    /// # Returns
    /// `true` when the inputs are valid and stored, `false` otherwise.
    pub fn initialize_split(
        env: Env,
        owner: Address,
        spending_percent: u32,
        savings_percent: u32,
        bills_percent: u32,
        insurance_percent: u32,
    ) -> bool {
        // Access control: require owner authorization
        owner.require_auth();

        // Check if already initialized
        let existing: Option<SplitConfig> = env.storage().instance().get(&symbol_short!("CONFIG"));

        if existing.is_some() {
            panic!("Split already initialized. Use update_split to modify.");
        }

        // Input validation: percentages must sum to 100
        let total = spending_percent + savings_percent + bills_percent + insurance_percent;
        if total != 100 {
            panic!("Percentages must sum to 100");
        }

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        let config = SplitConfig {
            owner: owner.clone(),
            spending_percent,
            savings_percent,
            bills_percent,
            insurance_percent,
            initialized: true,
        };

        env.storage()
            .instance()
            .set(&symbol_short!("CONFIG"), &config);

        // Also store the split vector for backward compatibility
        env.storage().instance().set(
            &symbol_short!("SPLIT"),
            &vec![
                &env,
                spending_percent,
                savings_percent,
                bills_percent,
                insurance_percent,
            ],
        );

        // Emit event for audit trail
        env.events()
            .publish((symbol_short!("split"), SplitEvent::Initialized), owner);

        true
    }

    /// Update an existing split configuration
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be the owner)
    /// * `spending_percent` - New percentage for spending (0-100)
    /// * `savings_percent` - New percentage for savings (0-100)
    /// * `bills_percent` - New percentage for bills (0-100)
    /// * `insurance_percent` - New percentage for insurance (0-100)
    ///
    /// # Returns
    /// True if update was successful
    ///
    /// # Panics
    /// - If caller is not the owner
    /// - If percentages don't sum to 100
    /// - If split is not initialized
    pub fn update_split(
        env: Env,
        caller: Address,
        spending_percent: u32,
        savings_percent: u32,
        bills_percent: u32,
        insurance_percent: u32,
    ) -> bool {
        // Access control: require caller authorization
        caller.require_auth();

        // Get existing config
        let mut config: SplitConfig = env
            .storage()
            .instance()
            .get(&symbol_short!("CONFIG"))
            .expect("Split not initialized");

        // Access control: verify caller is the owner
        if config.owner != caller {
            panic!("Only the owner can update the split configuration");
        }

        // Input validation: percentages must sum to 100
        let total = spending_percent + savings_percent + bills_percent + insurance_percent;
        if total != 100 {
            panic!("Percentages must sum to 100");
        }

        // Extend storage TTL
        Self::extend_instance_ttl(&env);

        // Update config
        config.spending_percent = spending_percent;
        config.savings_percent = savings_percent;
        config.bills_percent = bills_percent;
        config.insurance_percent = insurance_percent;

        env.storage()
            .instance()
            .set(&symbol_short!("CONFIG"), &config);

        // Also update the split vector for backward compatibility
        env.storage().instance().set(
            &symbol_short!("SPLIT"),
            &vec![
                &env,
                spending_percent,
                savings_percent,
                bills_percent,
                insurance_percent,
            ],
        );

        // Emit event for audit trail
        env.events()
            .publish((symbol_short!("split"), SplitEvent::Updated), caller);

        true
    }

    /// Get the current split configuration
    ///
    /// # Returns
    /// Vec containing [spending, savings, bills, insurance] percentages
    pub fn get_split(env: &Env) -> Vec<u32> {
        env.storage()
            .instance()
            .get(&symbol_short!("SPLIT"))
            .unwrap_or_else(|| vec![&env, 50, 30, 15, 5])
    }

    /// Get the full split configuration including owner
    ///
    /// # Returns
    /// SplitConfig or None if not initialized
    pub fn get_config(env: Env) -> Option<SplitConfig> {
        env.storage().instance().get(&symbol_short!("CONFIG"))
    }

    /// Calculate split amounts from a total remittance amount
    ///
    /// # Arguments
    /// * `total_amount` - The total amount to split (must be positive)
    ///
    /// # Returns
    /// Vec containing [spending, savings, bills, insurance] amounts
    ///
    /// # Panics
    /// - If total_amount is not positive
    pub fn calculate_split(env: Env, total_amount: i128) -> Vec<i128> {
        // Input validation
        if total_amount <= 0 {
            panic!("Total amount must be positive");
        }

        let split = Self::get_split(&env);

        let spending = (total_amount * split.get(0).unwrap() as i128) / 100;
        let savings = (total_amount * split.get(1).unwrap() as i128) / 100;
        let bills = (total_amount * split.get(2).unwrap() as i128) / 100;
        // Insurance gets the remainder to handle rounding
        let insurance = total_amount - spending - savings - bills;

        // Emit event for audit trail
        env.events().publish(
            (symbol_short!("split"), SplitEvent::Calculated),
            total_amount,
        );

        vec![&env, spending, savings, bills, insurance]
    }


    /// Extend the TTL of instance storage
    fn extend_instance_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }
}
