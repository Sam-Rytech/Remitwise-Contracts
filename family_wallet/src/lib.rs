#![no_std]
#![cfg_attr(not(test), deny(clippy::unwrap_used, clippy::expect_used))]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token::TokenClient, Address,
    Env, Map, Symbol, Vec,
};

use remitwise_common::FamilyRole;

// Storage TTL constants for active data
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280;
const INSTANCE_BUMP_AMOUNT: u32 = 518400;

// Storage TTL constants for archived data
const ARCHIVE_LIFETIME_THRESHOLD: u32 = 17280;
const ARCHIVE_BUMP_AMOUNT: u32 = 2592000;

// Signature expiration time (24 hours in seconds)
const SIGNATURE_EXPIRATION: u64 = 86400;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TransactionType {
    LargeWithdrawal = 1,
    SplitConfigChange = 2,
    RoleChange = 3,
    EmergencyTransfer = 4,
    PolicyCancellation = 5,
    RegularWithdrawal = 6,
}

#[contracttype]
#[derive(Clone)]
pub struct MultiSigConfig {
    pub threshold: u32,
    pub signers: Vec<Address>,
    pub spending_limit: i128,
}

#[contracttype]
#[derive(Clone)]
pub struct PendingTransaction {
    pub tx_id: u64,
    pub tx_type: TransactionType,
    pub proposer: Address,
    pub signatures: Vec<Address>,
    pub created_at: u64,
    pub expires_at: u64,
    pub data: TransactionData,
}

#[contracttype]
#[derive(Clone)]
pub enum TransactionData {
    Withdrawal(Address, Address, i128),
    SplitConfigChange(u32, u32, u32, u32),
    RoleChange(Address, FamilyRole),
    EmergencyTransfer(Address, Address, i128),
    PolicyCancellation(u32),
}

#[contracttype]
#[derive(Clone)]
pub struct FamilyMember {
    pub address: Address,
    pub role: FamilyRole,
    /// Legacy per-transaction cap in stroops. 0 = unlimited.
    pub spending_limit: i128,
    /// Enhanced precision spending limit (optional)
    pub precision_limit: Option<PrecisionSpendingLimit>,
    pub added_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct EmergencyConfig {
    pub max_amount: i128,
    pub cooldown: u64,
    pub min_balance: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum EmergencyEvent {
    ModeOn,
    ModeOff,
    TransferInit,
    TransferExec,
}

#[contracttype]
#[derive(Clone)]
pub struct MemberAddedEvent {
    pub member: Address,
    pub role: FamilyRole,
    pub spending_limit: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct SpendingLimitUpdatedEvent {
    pub member: Address,
    pub old_limit: i128,
    pub new_limit: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct ArchivedTransaction {
    pub tx_id: u64,
    pub tx_type: TransactionType,
    pub proposer: Address,
    pub executed_at: u64,
    pub archived_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct StorageStats {
    pub pending_transactions: u32,
    pub archived_transactions: u32,
    pub total_members: u32,
    pub last_updated: u64,
}

#[contracttype]
#[derive(Clone)]
pub struct AccessAuditEntry {
    pub operation: Symbol,
    pub caller: Address,
    pub target: Option<Address>,
    pub timestamp: u64,
    pub success: bool,
}

const CONTRACT_VERSION: u32 = 1;
const MAX_ACCESS_AUDIT_ENTRIES: u32 = 100;
const MAX_BATCH_MEMBERS: u32 = 30;

#[contracttype]
#[derive(Clone)]
pub struct BatchMemberItem {
    pub address: Address,
    pub role: FamilyRole,
}

/// Spending period configuration for rollover behavior
#[contracttype]
#[derive(Clone)]
pub struct SpendingPeriod {
    /// Period type: 0=Daily, 1=Weekly, 2=Monthly
    pub period_type: u32,
    /// Period start timestamp (aligned to period boundary)
    pub period_start: u64,
    /// Period duration in seconds
    pub period_duration: u64,
}

/// Cumulative spending tracking for precision validation
#[contracttype]
#[derive(Clone)]
pub struct SpendingTracker {
    /// Current period spending amount
    pub current_spent: i128,
    /// Last transaction timestamp for precision validation
    pub last_tx_timestamp: u64,
    /// Transaction count in current period
    pub tx_count: u32,
    /// Period configuration
    pub period: SpendingPeriod,
}

/// Enhanced spending limit with precision controls
#[contracttype]
#[derive(Clone)]
pub struct PrecisionSpendingLimit {
    /// Base spending limit per period
    pub limit: i128,
    /// Minimum precision unit (prevents dust attacks)
    pub min_precision: i128,
    /// Maximum single transaction amount
    pub max_single_tx: i128,
    /// Enable rollover validation
    pub enable_rollover: bool,
}

#[contracttype]
#[derive(Clone)]
pub enum ArchiveEvent {
    TransactionsArchived,
    ExpiredCleaned,
}

#[contract]
pub struct FamilyWallet;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    Unauthorized = 1,
    InvalidThreshold = 2,
    InvalidSigner = 3,
    TransactionNotFound = 4,
    TransactionExpired = 5,
    InsufficientSignatures = 6,
    DuplicateSignature = 7,
    InvalidTransactionType = 8,
    InvalidAmount = 9,
    InvalidRole = 10,
    MemberNotFound = 11,
    TransactionAlreadyExecuted = 12,
    InvalidSpendingLimit = 13,
    /// Amount below minimum precision threshold
    AmountBelowPrecision = 14,
    /// Single transaction exceeds maximum allowed
    ExceedsMaxSingleTx = 15,
    /// Cumulative spending would exceed period limit
    ExceedsPeriodLimit = 16,
    /// Period rollover validation failed
    RolloverValidationFailed = 17,
    /// Invalid precision configuration
    InvalidPrecisionConfig = 18,
}

#[contractimpl]
impl FamilyWallet {
    pub fn init(env: Env, owner: Address, initial_members: Vec<Address>) -> bool {
        owner.require_auth();

        let existing: Option<Address> = env.storage().instance().get(&symbol_short!("OWNER"));
        if existing.is_some() {
            panic!("Wallet already initialized");
        }

        Self::extend_instance_ttl(&env);

        env.storage()
            .instance()
            .set(&symbol_short!("OWNER"), &owner);

        let mut members: Map<Address, FamilyMember> = Map::new(&env);
        let timestamp = env.ledger().timestamp();

        members.set(
            owner.clone(),
            FamilyMember {
                address: owner.clone(),
                role: FamilyRole::Owner,
                spending_limit: 0,
                added_at: timestamp,
            },
        );

        for member_addr in initial_members.iter() {
            members.set(
                member_addr.clone(),
                FamilyMember {
                    address: member_addr.clone(),
                    role: FamilyRole::Member,
                    spending_limit: 0,
                    added_at: timestamp,
                },
            );
        }

        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        let default_config = MultiSigConfig {
            threshold: 2,
            signers: Vec::new(&env),
            spending_limit: 1000_0000000,
        };

        for tx_type in [
            TransactionType::LargeWithdrawal,
            TransactionType::SplitConfigChange,
            TransactionType::RoleChange,
            TransactionType::EmergencyTransfer,
            TransactionType::PolicyCancellation,
        ] {
            env.storage()
                .instance()
                .set(&Self::get_config_key(tx_type), &default_config.clone());
        }

        env.storage().instance().set(
            &symbol_short!("PEND_TXS"),
            &Map::<u64, PendingTransaction>::new(&env),
        );

        env.storage()
            .instance()
            .set(&symbol_short!("EXEC_TXS"), &Map::<u64, bool>::new(&env));

        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_TX"), &1u64);

        let em_config = EmergencyConfig {
            max_amount: 1000_0000000,
            cooldown: 3600,
            min_balance: 0,
        };
        env.storage()
            .instance()
            .set(&symbol_short!("EM_CONF"), &em_config);
        env.storage()
            .instance()
            .set(&symbol_short!("EM_MODE"), &false);
        env.storage()
            .instance()
            .set(&symbol_short!("EM_LAST"), &0u64);

        true
    }

    pub fn add_member(
        env: Env,
        admin: Address,
        member_address: Address,
        role: FamilyRole,
        spending_limit: i128,
    ) -> Result<bool, Error> {
        admin.require_auth();
        Self::require_not_paused(&env);

        if role == FamilyRole::Owner {
            return Err(Error::InvalidRole);
        }
        if !Self::is_owner_or_admin(&env, &admin) {
            return Err(Error::Unauthorized);
        }
        if spending_limit < 0 {
            return Err(Error::InvalidSpendingLimit);
        }

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        if members.get(member_address.clone()).is_some() {
            return Err(Error::InvalidRole);
        }

        Self::extend_instance_ttl(&env);

        let now = env.ledger().timestamp();
        members.set(
            member_address.clone(),
            FamilyMember {
                address: member_address.clone(),
                role,
                spending_limit,
                precision_limit: None, // Default to legacy behavior
                added_at: now,
            },
        );
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        env.events().publish(
            (symbol_short!("added"), symbol_short!("member")),
            MemberAddedEvent {
                member: member_address,
                role,
                spending_limit,
                timestamp: now,
            },
        );

        Ok(true)
    }

    pub fn get_member(env: Env, member_address: Address) -> Option<FamilyMember> {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        members.get(member_address)
    }

    pub fn update_spending_limit(
        env: Env,
        caller: Address,
        member_address: Address,
        new_limit: i128,
    ) -> Result<bool, Error> {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            return Err(Error::Unauthorized);
        }
        if new_limit < 0 {
            return Err(Error::InvalidSpendingLimit);
        }

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        let mut record = members
            .get(member_address.clone())
            .ok_or(Error::MemberNotFound)?;

        let old_limit = record.spending_limit;
        record.spending_limit = new_limit;
        members.set(member_address.clone(), record);

        Self::extend_instance_ttl(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        let now = env.ledger().timestamp();
        env.events().publish(
            (symbol_short!("updated"), symbol_short!("limit")),
            SpendingLimitUpdatedEvent {
                member: member_address,
                old_limit,
                new_limit,
                timestamp: now,
            },
        );

        Ok(true)
    }

    /// Check if `caller` is allowed to spend `amount`.
    ///
    /// Rules (checked in order):
    /// 1. Unknown address → false
    /// 2. Negative amount → false
    /// 3. Owner / Admin → always true (unlimited)
    /// 4. Member with `spending_limit == 0` → unlimited → true
    /// 5. Member with `spending_limit > 0` → true iff `amount <= spending_limit`
    pub fn check_spending_limit(env: Env, caller: Address, amount: i128) -> bool {
        if amount < 0 {
            return false;
        }

        let members: Map<Address, FamilyMember> =
            match env.storage().instance().get(&symbol_short!("MEMBERS")) {
                Some(m) => m,
                None => return false,
            };

        let member = match members.get(caller) {
            Some(m) => m,
            None => return false,
        };

        // Expired roles are treated as having no permissions.
        if Self::role_has_expired(&env, &member.address) {
            return false;
        }

        // Owner and Admin are never restricted
        if member.role == FamilyRole::Owner || member.role == FamilyRole::Admin {
            return true;
        }

        // 0 means unlimited for regular members too
        if member.spending_limit == 0 {
            return true;
        }

        amount <= member.spending_limit
    }

    /// Configure precision spending limit for a member
    /// 
    /// # Arguments
    /// * `caller` - Must be Owner or Admin
    /// * `member_address` - Target member to configure
    /// * `precision_limit` - New precision limit configuration
    /// 
    /// # Security Assumptions
    /// - Validates precision parameters to prevent overflow/underflow
    /// - Ensures minimum precision is positive and reasonable
    /// - Validates period configuration for rollover behavior
    pub fn set_precision_spending_limit(
        env: Env,
        caller: Address,
        member_address: Address,
        precision_limit: PrecisionSpendingLimit,
    ) -> Result<bool, Error> {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            return Err(Error::Unauthorized);
        }

        // Validate precision configuration
        if precision_limit.limit < 0 {
            return Err(Error::InvalidPrecisionConfig);
        }
        if precision_limit.min_precision <= 0 {
            return Err(Error::InvalidPrecisionConfig);
        }
        if precision_limit.max_single_tx <= 0 || precision_limit.max_single_tx > precision_limit.limit {
            return Err(Error::InvalidPrecisionConfig);
        }

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        let mut member = members
            .get(member_address.clone())
            .ok_or(Error::MemberNotFound)?;

        member.precision_limit = Some(precision_limit);
        members.set(member_address.clone(), member);

        Self::extend_instance_ttl(&env);
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        Ok(true)
    }

    /// Enhanced spending validation with precision and rollover checks
    /// 
    /// # Arguments
    /// * `caller` - Address attempting to spend
    /// * `amount` - Amount to spend (in stroops)
    /// 
    /// # Returns
    /// * `Ok(())` if spending is allowed
    /// * `Err(Error)` with specific validation failure
    /// 
    /// # Security Assumptions
    /// - Prevents precision attacks via minimum precision validation
    /// - Enforces single transaction limits to prevent large withdrawals
    /// - Validates cumulative spending against period limits
    /// - Handles period rollover with proper boundary validation
    pub fn validate_precision_spending(
        env: Env,
        caller: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .ok_or(Error::MemberNotFound)?;

        let member = members.get(caller.clone()).ok_or(Error::MemberNotFound)?;

        // Check role expiry
        if Self::role_has_expired(&env, &member.address) {
            return Err(Error::Unauthorized);
        }

        // Owner and Admin bypass precision checks
        if member.role == FamilyRole::Owner || member.role == FamilyRole::Admin {
            return Ok(());
        }

        // If no precision limit configured, fall back to legacy check
        let precision_limit = match &member.precision_limit {
            Some(limit) => limit,
            None => {
                // Legacy behavior: check basic spending limit
                if member.spending_limit == 0 || amount <= member.spending_limit {
                    return Ok(());
                } else {
                    return Err(Error::ExceedsPeriodLimit);
                }
            }
        };

        // Precision validation: prevent dust attacks
        if amount < precision_limit.min_precision {
            return Err(Error::AmountBelowPrecision);
        }

        // Single transaction limit validation
        if amount > precision_limit.max_single_tx {
            return Err(Error::ExceedsMaxSingleTx);
        }

        // If rollover is disabled, only check single transaction limits
        if !precision_limit.enable_rollover {
            return Ok(());
        }

        // Get or initialize spending tracker
        let tracker_key = symbol_short!("TRACKER");
        let mut spending_trackers: Map<Address, SpendingTracker> = env
            .storage()
            .instance()
            .get(&tracker_key)
            .unwrap_or_else(|| Map::new(&env));

        let current_time = env.ledger().timestamp();
        let mut tracker = spending_trackers.get(caller.clone()).unwrap_or_else(|| {
            // Initialize new tracker with current period
            SpendingTracker {
                current_spent: 0,
                last_tx_timestamp: current_time,
                tx_count: 0,
                period: Self::get_current_period(current_time),
            }
        });

        // Check if we need to roll over to a new period
        if Self::should_rollover_period(&tracker.period, current_time) {
            tracker = Self::rollover_spending_period(tracker, current_time)?;
        }

        // Validate cumulative spending limit
        let new_total = tracker.current_spent.saturating_add(amount);
        if new_total > precision_limit.limit {
            return Err(Error::ExceedsPeriodLimit);
        }

        // Update tracker
        tracker.current_spent = new_total;
        tracker.last_tx_timestamp = current_time;
        tracker.tx_count = tracker.tx_count.saturating_add(1);

        // Save updated tracker
        spending_trackers.set(caller, tracker);
        env.storage().instance().set(&tracker_key, &spending_trackers);

        Ok(())
    }

    /// Get current spending period configuration
    /// 
    /// # Arguments
    /// * `timestamp` - Current timestamp to align period
    /// 
    /// # Returns
    /// * `SpendingPeriod` aligned to daily boundary (00:00 UTC)
    /// 
    /// # Security Assumptions
    /// - Uses daily periods (86400 seconds) for consistent rollover
    /// - Aligns to UTC midnight to prevent timezone manipulation
    /// - Handles timestamp overflow gracefully
    fn get_current_period(timestamp: u64) -> SpendingPeriod {
        const DAILY_SECONDS: u64 = 86400; // 24 hours
        
        // Align to daily boundary (00:00 UTC)
        let period_start = (timestamp / DAILY_SECONDS) * DAILY_SECONDS;
        
        SpendingPeriod {
            period_type: 0, // Daily
            period_start,
            period_duration: DAILY_SECONDS,
        }
    }

    /// Check if spending period should rollover
    /// 
    /// # Arguments
    /// * `period` - Current period configuration
    /// * `current_time` - Current timestamp
    /// 
    /// # Returns
    /// * `true` if period has expired and should rollover
    /// 
    /// # Security Assumptions
    /// - Uses inclusive boundary check to prevent edge case exploits
    /// - Handles timestamp overflow gracefully
    fn should_rollover_period(period: &SpendingPeriod, current_time: u64) -> bool {
        current_time >= period.period_start.saturating_add(period.period_duration)
    }

    /// Rollover spending period and reset counters
    /// 
    /// # Arguments
    /// * `old_tracker` - Previous period tracker
    /// * `current_time` - Current timestamp for new period
    /// 
    /// # Returns
    /// * `Result<SpendingTracker, Error>` with new period configuration
    /// 
    /// # Security Assumptions
    /// - Resets spending counters to prevent carryover attacks
    /// - Validates new period alignment
    /// - Preserves audit trail via transaction count reset
    fn rollover_spending_period(
        old_tracker: SpendingTracker,
        current_time: u64,
    ) -> Result<SpendingTracker, Error> {
        let new_period = Self::get_current_period(current_time);
        
        // Validate rollover is legitimate (prevent manipulation)
        if current_time < old_tracker.period.period_start.saturating_add(old_tracker.period.period_duration) {
            return Err(Error::RolloverValidationFailed);
        }

        Ok(SpendingTracker {
            current_spent: 0, // Reset spending for new period
            last_tx_timestamp: current_time,
            tx_count: 0, // Reset transaction count
            period: new_period,
        })
    }

    /// Get spending tracker for a member (read-only)
    /// 
    /// # Arguments
    /// * `member_address` - Address to get tracker for
    /// 
    /// # Returns
    /// * `Option<SpendingTracker>` if tracker exists
    pub fn get_spending_tracker(env: Env, member_address: Address) -> Option<SpendingTracker> {
        let tracker_key = symbol_short!("TRACKER");
        let spending_trackers: Map<Address, SpendingTracker> = env
            .storage()
            .instance()
            .get(&tracker_key)?;
        
        spending_trackers.get(member_address)
    }

    pub fn configure_multisig(
        env: Env,
        caller: Address,
        tx_type: TransactionType,
        threshold: u32,
        signers: Vec<Address>,
        spending_limit: i128,
    ) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);

        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        if !Self::is_owner_or_admin_in_members(&env, &members, &caller) {
            panic!("Only Owner or Admin can configure multi-sig");
        }

        // Validate threshold
        let signer_count = signers.len();
        if threshold == 0 || threshold > signer_count {
            panic!("Invalid threshold");
        }

        for signer in signers.iter() {
            if members.get(signer.clone()).is_none() {
                panic!("Signer must be a family member");
            }
        }

        if spending_limit < 0 {
            panic!("Spending limit must be non-negative");
        }

        Self::extend_instance_ttl(&env);

        let config = MultiSigConfig {
            threshold,
            signers,
            spending_limit,
        };

        env.storage()
            .instance()
            .set(&Self::get_config_key(tx_type), &config);

        true
    }

    pub fn propose_transaction(
        env: Env,
        proposer: Address,
        tx_type: TransactionType,
        data: TransactionData,
    ) -> u64 {
        proposer.require_auth();
        Self::require_not_paused(&env);
        Self::require_role_at_least(&env, &proposer, FamilyRole::Member);

        if !Self::is_family_member(&env, &proposer) {
            panic!("Only family members can propose transactions");
        }

        let config_key = match tx_type {
            TransactionType::RegularWithdrawal => {
                Self::get_config_key(TransactionType::LargeWithdrawal)
            }
            _ => Self::get_config_key(tx_type),
        };

        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&config_key)
            .unwrap_or_else(|| panic!("Multi-sig config not found"));

        let requires_multisig = match (&tx_type, &data) {
            (TransactionType::RegularWithdrawal, TransactionData::Withdrawal(_, _, amount)) => {
                *amount > config.spending_limit
            }
            (TransactionType::LargeWithdrawal, _) => true,
            (TransactionType::RegularWithdrawal, _) => false,
            _ => true,
        };

        if !requires_multisig {
            return Self::execute_transaction_internal(&env, &proposer, &tx_type, &data, false);
        }

        Self::extend_instance_ttl(&env);

        let mut next_tx_id: u64 = env
            .storage()
            .instance()
            .get(&symbol_short!("NEXT_TX"))
            .unwrap_or(1);

        let tx_id = next_tx_id;
        next_tx_id += 1;

        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_TX"), &next_tx_id);

        let timestamp = env.ledger().timestamp();
        let mut signatures = Vec::new(&env);
        signatures.push_back(proposer.clone());

        let pending_tx = PendingTransaction {
            tx_id,
            tx_type,
            proposer: proposer.clone(),
            signatures,
            created_at: timestamp,
            expires_at: timestamp + SIGNATURE_EXPIRATION,
            data: data.clone(),
        };

        let mut pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| panic!("Pending transactions map not initialized"));

        pending_txs.set(tx_id, pending_tx);
        env.storage()
            .instance()
            .set(&symbol_short!("PEND_TXS"), &pending_txs);

        tx_id
    }

    pub fn sign_transaction(env: Env, signer: Address, tx_id: u64) -> bool {
        signer.require_auth();
        Self::require_not_paused(&env);
        Self::require_role_at_least(&env, &signer, FamilyRole::Member);

        if !Self::is_family_member(&env, &signer) {
            panic!("Only family members can sign transactions");
        }

        Self::extend_instance_ttl(&env);

        let mut pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| panic!("Pending transactions map not initialized"));

        let mut pending_tx = pending_txs.get(tx_id).unwrap_or_else(|| panic!("Transaction not found"));

        let current_time = env.ledger().timestamp();
        if current_time > pending_tx.expires_at {
            panic!("Transaction expired");
        }

        for sig in pending_tx.signatures.iter() {
            if sig.clone() == signer {
                panic!("Already signed this transaction");
            }
        }

        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&Self::get_config_key(pending_tx.tx_type))
            .unwrap_or_else(|| panic!("Multi-sig config not found"));

        let mut is_authorized = false;
        for authorized_signer in config.signers.iter() {
            if authorized_signer.clone() == signer {
                is_authorized = true;
                break;
            }
        }

        if !is_authorized {
            panic!("Signer not authorized for this transaction type");
        }

        pending_tx.signatures.push_back(signer.clone());

        if pending_tx.signatures.len() >= config.threshold {
            let executed = Self::execute_transaction_internal(
                &env,
                &pending_tx.proposer,
                &pending_tx.tx_type,
                &pending_tx.data,
                true,
            );

            if executed == 0 {
                pending_txs.remove(tx_id);
                env.storage()
                    .instance()
                    .set(&symbol_short!("PEND_TXS"), &pending_txs);

                let mut executed_txs: Map<u64, bool> = env
                    .storage()
                    .instance()
                    .get(&symbol_short!("EXEC_TXS"))
                    .unwrap_or_else(|| panic!("Executed transactions map not initialized"));

                executed_txs.set(tx_id, true);
                env.storage()
                    .instance()
                    .set(&symbol_short!("EXEC_TXS"), &executed_txs);
            }

            return true;
        }

        pending_txs.set(tx_id, pending_tx);
        env.storage()
            .instance()
            .set(&symbol_short!("PEND_TXS"), &pending_txs);

        true
    }

    pub fn withdraw(
        env: Env,
        proposer: Address,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> u64 {
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        // Enhanced precision and rollover validation
        if let Err(error) = Self::validate_precision_spending(env.clone(), proposer.clone(), amount) {
            panic_with_error!(&env, error);
        }

        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&Self::get_config_key(TransactionType::LargeWithdrawal))
            .unwrap_or_else(|| panic!("Multi-sig config not found"));

        let tx_type = if amount > config.spending_limit {
            TransactionType::LargeWithdrawal
        } else {
            TransactionType::RegularWithdrawal
        };

        Self::propose_transaction(
            env,
            proposer,
            tx_type,
            TransactionData::Withdrawal(token, recipient, amount),
        )
    }

    pub fn propose_split_config_change(
        env: Env,
        proposer: Address,
        spending_percent: u32,
        savings_percent: u32,
        bills_percent: u32,
        insurance_percent: u32,
    ) -> u64 {
        if spending_percent + savings_percent + bills_percent + insurance_percent != 100 {
            panic!("Percentages must sum to 100");
        }

        Self::propose_transaction(
            env,
            proposer,
            TransactionType::SplitConfigChange,
            TransactionData::SplitConfigChange(
                spending_percent,
                savings_percent,
                bills_percent,
                insurance_percent,
            ),
        )
    }

    pub fn propose_role_change(
        env: Env,
        proposer: Address,
        member: Address,
        new_role: FamilyRole,
    ) -> u64 {
        Self::propose_transaction(
            env,
            proposer,
            TransactionType::RoleChange,
            TransactionData::RoleChange(member, new_role),
        )
    }

    pub fn propose_emergency_transfer(
        env: Env,
        proposer: Address,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> u64 {
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let em_mode: bool = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_MODE"))
            .unwrap_or(false);

        if em_mode {
            return Self::execute_emergency_transfer_now(env, proposer, token, recipient, amount);
        }

        let pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut active_proposals = 0;
        for (_, tx) in pending_txs.iter() {
            if tx.proposer == proposer && tx.tx_type == TransactionType::EmergencyTransfer {
                if let TransactionData::EmergencyTransfer(t, r, a) = &tx.data {
                    if t == &token && r == &recipient && *a == amount {
                        panic!("Identical emergency transfer proposal already pending");
                    }
                }
                active_proposals += 1;
            }
        }

        if active_proposals >= 1 {
            panic!("Maximum pending emergency proposals reached");
        }

        Self::propose_transaction(
            env,
            proposer,
            TransactionType::EmergencyTransfer,
            TransactionData::EmergencyTransfer(token, recipient, amount),
        )
    }

    pub fn propose_policy_cancellation(env: Env, proposer: Address, policy_id: u32) -> u64 {
        Self::propose_transaction(
            env,
            proposer,
            TransactionType::PolicyCancellation,
            TransactionData::PolicyCancellation(policy_id),
        )
    }

    pub fn configure_emergency(
        env: Env,
        caller: Address,
        max_amount: i128,
        cooldown: u64,
        min_balance: i128,
    ) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            panic!("Only Owner or Admin can configure emergency settings");
        }
        if max_amount <= 0 {
            panic!("Emergency max amount must be positive");
        }
        if min_balance < 0 {
            panic!("Emergency min balance must be non-negative");
        }

        Self::extend_instance_ttl(&env);

        env.storage().instance().set(
            &symbol_short!("EM_CONF"),
            &EmergencyConfig {
                max_amount,
                cooldown,
                min_balance,
            },
        );

        true
    }

    pub fn set_emergency_mode(env: Env, caller: Address, enabled: bool) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            panic!("Only Owner or Admin can change emergency mode");
        }

        Self::extend_instance_ttl(&env);

        env.storage()
            .instance()
            .set(&symbol_short!("EM_MODE"), &enabled);

        let event = if enabled {
            EmergencyEvent::ModeOn
        } else {
            EmergencyEvent::ModeOff
        };
        env.events()
            .publish((symbol_short!("emerg"), event), caller);

        true
    }

    pub fn add_family_member(env: Env, caller: Address, member: Address, role: FamilyRole) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);
        if role == FamilyRole::Owner {
            panic!("Cannot add Owner via add_family_member");
        }
        if !Self::is_owner_or_admin(&env, &caller) {
            panic!("Only Owner or Admin can add family members");
        }

        Self::extend_instance_ttl(&env);

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        let timestamp = env.ledger().timestamp();
        members.set(
            member.clone(),
            FamilyMember {
                address: member.clone(),
                role,
                spending_limit: 0,
                precision_limit: None, // Default to legacy behavior
                added_at: timestamp,
            },
        );

        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        Self::append_access_audit(&env, symbol_short!("add_mem"), &caller, Some(member), true);
        true
    }

    pub fn remove_family_member(env: Env, caller: Address, member: Address) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);

        let owner: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        if Self::role_has_expired(&env, &caller) {
            panic!("Role has expired");
        }
        if caller != owner {
            panic!("Only Owner can remove family members");
        }
        if member == owner {
            panic!("Cannot remove owner");
        }

        Self::extend_instance_ttl(&env);

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        members.remove(member.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        Self::append_access_audit(&env, symbol_short!("rem_mem"), &caller, Some(member), true);
        true
    }

    pub fn get_pending_transaction(env: Env, tx_id: u64) -> Option<PendingTransaction> {
        let pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| panic!("Pending transactions map not initialized"));

        pending_txs.get(tx_id)
    }

    pub fn get_multisig_config(env: Env, tx_type: TransactionType) -> Option<MultiSigConfig> {
        env.storage().instance().get(&Self::get_config_key(tx_type))
    }

    pub fn get_family_member(env: Env, member: Address) -> Option<FamilyMember> {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));

        members.get(member)
    }

    pub fn get_owner(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .unwrap_or_else(|| panic!("Wallet not initialized"))
    }

    pub fn get_emergency_config(env: Env) -> Option<EmergencyConfig> {
        env.storage().instance().get(&symbol_short!("EM_CONF"))
    }

    pub fn is_emergency_mode(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("EM_MODE"))
            .unwrap_or(false)
    }

    pub fn get_last_emergency_at(env: Env) -> Option<u64> {
        let ts: u64 = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_LAST"))
            .unwrap_or(0u64);
        if ts == 0 {
            None
        } else {
            Some(ts)
        }
    }

    pub fn archive_old_transactions(env: Env, caller: Address, before_timestamp: u64) -> u32 {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            panic!("Only Owner or Admin can archive transactions");
        }

        Self::extend_instance_ttl(&env);

        let executed_txs: Map<u64, bool> = env
            .storage()
            .instance()
            .get(&symbol_short!("EXEC_TXS"))
            .unwrap_or_else(|| Map::new(&env));

        let mut archived: Map<u64, ArchivedTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_TX"))
            .unwrap_or_else(|| Map::new(&env));

        let current_time = env.ledger().timestamp();
        let mut archived_count = 0u32;

        for (tx_id, _) in executed_txs.iter() {
            let archived_tx = ArchivedTransaction {
                tx_id,
                tx_type: TransactionType::RegularWithdrawal,
                proposer: caller.clone(),
                executed_at: before_timestamp,
                archived_at: current_time,
            };
            archived.set(tx_id, archived_tx);
            archived_count += 1;
        }

        if archived_count > 0 {
            env.storage()
                .instance()
                .set(&symbol_short!("EXEC_TXS"), &Map::<u64, bool>::new(&env));
        }

        env.storage()
            .instance()
            .set(&symbol_short!("ARCH_TX"), &archived);

        Self::extend_archive_ttl(&env);
        Self::update_storage_stats(&env);

        env.events().publish(
            (symbol_short!("wallet"), ArchiveEvent::TransactionsArchived),
            (archived_count, caller),
        );

        archived_count
    }

    pub fn get_archived_transactions(env: Env, limit: u32) -> Vec<ArchivedTransaction> {
        let archived: Map<u64, ArchivedTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_TX"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        for (count, (_, tx)) in archived.iter().enumerate() {
            if count as u32 >= limit {
                break;
            }
            result.push_back(tx);
        }
        result
    }

    pub fn cleanup_expired_pending(env: Env, caller: Address) -> u32 {
        caller.require_auth();
        Self::require_not_paused(&env);

        if !Self::is_owner_or_admin(&env, &caller) {
            panic!("Only Owner or Admin can cleanup expired transactions");
        }

        Self::extend_instance_ttl(&env);

        let mut pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| Map::new(&env));

        let current_time = env.ledger().timestamp();
        let mut removed_count = 0u32;
        let mut to_remove: Vec<u64> = Vec::new(&env);

        for (tx_id, tx) in pending_txs.iter() {
            if tx.expires_at < current_time {
                to_remove.push_back(tx_id);
                removed_count += 1;
            }
        }

        for i in 0..to_remove.len() {
            if let Some(id) = to_remove.get(i) {
                pending_txs.remove(id);
            }
        }

        env.storage()
            .instance()
            .set(&symbol_short!("PEND_TXS"), &pending_txs);

        Self::update_storage_stats(&env);

        env.events().publish(
            (symbol_short!("wallet"), ArchiveEvent::ExpiredCleaned),
            (removed_count, caller),
        );

        removed_count
    }

    pub fn get_storage_stats(env: Env) -> StorageStats {
        env.storage()
            .instance()
            .get(&symbol_short!("STOR_STAT"))
            .unwrap_or(StorageStats {
                pending_transactions: 0,
                archived_transactions: 0,
                total_members: 0,
                last_updated: 0,
            })
    }

    /// @notice Set or clear a role-expiry timestamp for an existing family member.
    /// @dev Expiry is inclusive: at `ledger.timestamp() >= expires_at` the member is treated as expired.
    /// @param caller Admin/Owner authorizing the change.
    /// @param member Target family member.
    /// @param expires_at Unix timestamp in seconds; `None` clears expiry.
    pub fn set_role_expiry(
        env: Env,
        caller: Address,
        member: Address,
        expires_at: Option<u64>,
    ) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Admin);
        Self::require_not_paused(&env);
        Self::extend_instance_ttl(&env);

        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));
        if members.get(member.clone()).is_none() {
            panic!("Member not found");
        }

        let mut m: Map<Address, u64> = env
            .storage()
            .instance()
            .get(&symbol_short!("ROLE_EXP"))
            .unwrap_or_else(|| Map::new(&env));
        match expires_at {
            Some(t) => m.set(member.clone(), t),
            None => {
                m.remove(member.clone());
            }
        }
        env.storage().instance().set(&symbol_short!("ROLE_EXP"), &m);
        Self::append_access_audit(&env, symbol_short!("role_exp"), &caller, Some(member), true);
        true
    }

    pub fn get_role_expiry_public(env: Env, address: Address) -> Option<u64> {
        Self::get_role_expiry(&env, &address)
    }

    pub fn pause(env: Env, caller: Address) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Admin);
        let admin = Self::get_pause_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .unwrap_or_else(|| panic!("Wallet not initialized"))
        });
        if admin != caller {
            panic!("Only pause admin can pause");
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &true);
        env.events()
            .publish((symbol_short!("wallet"), symbol_short!("paused")), ());
        true
    }

    pub fn unpause(env: Env, caller: Address) -> bool {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .unwrap_or_else(|| panic!("Wallet not initialized"))
        });
        if admin != caller {
            panic!("Only pause admin can unpause");
        }
        if Self::role_has_expired(&env, &caller) {
            panic!("Role has expired");
        }
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSED"), &false);
        env.events()
            .publish((symbol_short!("wallet"), symbol_short!("unpaused")), ());
        true
    }

    pub fn set_pause_admin(env: Env, caller: Address, new_admin: Address) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Owner);
        env.storage()
            .instance()
            .set(&symbol_short!("PAUSE_ADM"), &new_admin);
        true
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

    /// Set or transfer the upgrade admin role.
    /// 
    /// # Security Requirements
    /// - Only wallet owners can set or transfer upgrade admin role
    /// - Caller must be authenticated via require_auth()
    /// - Caller must have at least Owner role in the family wallet
    /// 
    /// # Parameters
    /// - `caller`: The address attempting to set the upgrade admin
    /// - `new_admin`: The address to become the new upgrade admin
    /// 
    /// # Returns
    /// - `true` on successful admin transfer
    /// 
    /// # Panics
    /// - If caller lacks Owner role or higher
    pub fn set_upgrade_admin(env: Env, caller: Address, new_admin: Address) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Owner);
        
        let current_upgrade_admin = Self::get_upgrade_admin(&env);
        
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
        
        // Emit admin transfer event for audit trail
        env.events().publish(
            (symbol_short!("family"), symbol_short!("adm_xfr")),
            (current_upgrade_admin, new_admin.clone()),
        );
        
        true
    }

    /// Get the current upgrade admin address.
    /// 
    /// # Returns
    /// - `Some(Address)` if upgrade admin is set
    /// - `None` if no upgrade admin has been configured
    pub fn get_upgrade_admin_public(env: Env) -> Option<Address> {
        Self::get_upgrade_admin(&env)
    }

    pub fn set_version(env: Env, caller: Address, new_version: u32) -> bool {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .unwrap_or_else(|| panic!("Wallet not initialized"))
        });
        if admin != caller {
            panic!("Only upgrade admin can set version");
        }
        if Self::role_has_expired(&env, &caller) {
            panic!("Role has expired");
        }
        let prev = Self::get_version(env.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("VERSION"), &new_version);
        env.events().publish(
            (symbol_short!("wallet"), symbol_short!("upgraded")),
            (prev, new_version),
        );
        true
    }

    pub fn batch_add_family_members(
        env: Env,
        caller: Address,
        members: Vec<BatchMemberItem>,
    ) -> u32 {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Admin);
        Self::require_not_paused(&env);
        if members.len() > MAX_BATCH_MEMBERS {
            panic!("Batch too large");
        }
        Self::extend_instance_ttl(&env);
        let mut members_map: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));
        let timestamp = env.ledger().timestamp();
        let mut count = 0u32;
        for item in members.iter() {
            if item.role == FamilyRole::Owner {
                panic!("Cannot add Owner via batch");
            }
            members_map.set(
                item.address.clone(),
                FamilyMember {
                    address: item.address.clone(),
                    role: item.role,
                    spending_limit: 0,
                    precision_limit: None, // Default to legacy behavior
                    added_at: timestamp,
                },
            );
            Self::append_access_audit(
                &env,
                symbol_short!("add_mem"),
                &caller,
                Some(item.address.clone()),
                true,
            );
            count += 1;
        }
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members_map);
        Self::update_storage_stats(&env);
        count
    }

    pub fn batch_remove_family_members(env: Env, caller: Address, addresses: Vec<Address>) -> u32 {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Owner);
        let owner: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));
        if caller != owner {
            panic!("Only Owner can remove members");
        }
        Self::require_not_paused(&env);
        if addresses.len() > MAX_BATCH_MEMBERS {
            panic!("Batch too large");
        }
        Self::extend_instance_ttl(&env);
        let mut members_map: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));
        let mut count = 0u32;
        for addr in addresses.iter() {
            if addr.clone() == owner {
                panic!("Cannot remove owner");
            }
            if members_map.get(addr.clone()).is_some() {
                members_map.remove(addr.clone());
                Self::append_access_audit(
                    &env,
                    symbol_short!("rem_mem"),
                    &caller,
                    Some(addr.clone()),
                    true,
                );
                count += 1;
            }
        }
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members_map);
        Self::update_storage_stats(&env);
        count
    }

    pub fn get_access_audit(env: Env, limit: u32) -> Vec<AccessAuditEntry> {
        let entries: Vec<AccessAuditEntry> = env
            .storage()
            .instance()
            .get(&symbol_short!("ACC_AUDIT"))
            .unwrap_or_else(|| Vec::new(&env));
        let n = entries.len().min(limit);
        let mut out = Vec::new(&env);
        for i in (entries.len().saturating_sub(n))..entries.len() {
            if let Some(e) = entries.get(i) {
                out.push_back(e);
            }
        }
        out
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn execute_emergency_transfer_now(
        env: Env,
        proposer: Address,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> u64 {
        let config: EmergencyConfig = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_CONF"))
            .unwrap_or_else(|| panic!("Emergency config not set"));

        if amount > config.max_amount {
            panic!("Emergency amount exceeds maximum allowed");
        }

        let now = env.ledger().timestamp();
        let last_ts: u64 = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_LAST"))
            .unwrap_or(0u64);
        if last_ts != 0 && now < last_ts.saturating_add(config.cooldown) {
            panic!("Emergency transfer cooldown period not elapsed");
        }

        let token_client = TokenClient::new(&env, &token);
        let current_balance = token_client.balance(&proposer);
        if current_balance - amount < config.min_balance {
            panic!("Emergency transfer would violate minimum balance requirement");
        }

        env.events().publish(
            (symbol_short!("emerg"), EmergencyEvent::TransferInit),
            (proposer.clone(), recipient.clone(), amount),
        );

        proposer.require_auth();
        let _ = Self::execute_transaction_internal(
            &env,
            &proposer,
            &TransactionType::EmergencyTransfer,
            &TransactionData::EmergencyTransfer(token.clone(), recipient.clone(), amount),
            false,
        );

        let store_ts: u64 = if now == 0 { 1u64 } else { now };
        env.storage()
            .instance()
            .set(&symbol_short!("EM_LAST"), &store_ts);

        env.events().publish(
            (symbol_short!("emerg"), EmergencyEvent::TransferExec),
            (proposer, recipient, amount),
        );

        0
    }

    fn execute_transaction_internal(
        env: &Env,
        proposer: &Address,
        tx_type: &TransactionType,
        data: &TransactionData,
        require_auth: bool,
    ) -> u64 {
        match (tx_type, data) {
            (
                TransactionType::RegularWithdrawal,
                TransactionData::Withdrawal(token, recipient, amount),
            )
            | (
                TransactionType::LargeWithdrawal,
                TransactionData::Withdrawal(token, recipient, amount),
            ) => {
                if require_auth {
                    proposer.require_auth();
                }
                let token_client = TokenClient::new(env, token);
                token_client.transfer(proposer, recipient, amount);
                0
            }
            (TransactionType::SplitConfigChange, TransactionData::SplitConfigChange(..)) => 0,
            (TransactionType::RoleChange, TransactionData::RoleChange(member, new_role)) => {
                let mut members: Map<Address, FamilyMember> = env
                    .storage()
                    .instance()
                    .get(&symbol_short!("MEMBERS"))
                    .unwrap_or_else(|| panic!("Wallet not initialized"));

                if let Some(mut member_data) = members.get(member.clone()) {
                    member_data.role = *new_role;
                    members.set(member.clone(), member_data);
                    env.storage()
                        .instance()
                        .set(&symbol_short!("MEMBERS"), &members);
                    Self::append_access_audit(
                        env,
                        symbol_short!("role_chg"),
                        proposer,
                        Some(member.clone()),
                        true,
                    );
                }
                0
            }
            (
                TransactionType::EmergencyTransfer,
                TransactionData::EmergencyTransfer(token, recipient, amount),
            ) => {
                if require_auth {
                    proposer.require_auth();
                }
                let token_client = TokenClient::new(env, token);
                token_client.transfer(proposer, recipient, amount);
                0
            }
            (TransactionType::PolicyCancellation, TransactionData::PolicyCancellation(..)) => 0,
            _ => panic!("Invalid transaction type or data mismatch"),
        }
    }

    fn get_config_key(tx_type: TransactionType) -> Symbol {
        match tx_type {
            TransactionType::LargeWithdrawal => symbol_short!("MS_WDRAW"),
            TransactionType::SplitConfigChange => symbol_short!("MS_SPLIT"),
            TransactionType::RoleChange => symbol_short!("MS_ROLE"),
            TransactionType::EmergencyTransfer => symbol_short!("MS_EMERG"),
            TransactionType::PolicyCancellation => symbol_short!("MS_POL"),
            TransactionType::RegularWithdrawal => symbol_short!("MS_REG"),
        }
    }

    fn is_family_member(env: &Env, address: &Address) -> bool {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| Map::new(env));

        members.get(address.clone()).is_some()
    }

    fn is_owner_or_admin(env: &Env, address: &Address) -> bool {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| Map::new(env));

        Self::is_owner_or_admin_in_members(env, &members, address)
    }

    fn is_owner_or_admin_in_members(
        env: &Env,
        members: &Map<Address, FamilyMember>,
        address: &Address,
    ) -> bool {
        if let Some(member) = members.get(address.clone()) {
            if Self::role_has_expired(env, address) {
                false
            } else {
                matches!(member.role, FamilyRole::Owner | FamilyRole::Admin)
            }
        } else {
            false
        }
    }

    fn role_ordinal(role: FamilyRole) -> u32 {
        role as u32
    }

    fn get_role_expiry(env: &Env, address: &Address) -> Option<u64> {
        env.storage()
            .instance()
            .get::<_, Map<Address, u64>>(&symbol_short!("ROLE_EXP"))
            .unwrap_or_else(|| Map::new(env))
            .get(address.clone())
    }

    fn role_has_expired(env: &Env, address: &Address) -> bool {
        if let Some(exp) = Self::get_role_expiry(env, address) {
            env.ledger().timestamp() >= exp
        } else {
            false
        }
    }

    fn require_role_at_least(env: &Env, caller: &Address, min_role: FamilyRole) {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| panic!("Wallet not initialized"));
        let member = members.get(caller.clone()).unwrap_or_else(|| panic!("Not a family member"));
        if Self::role_has_expired(env, caller) {
            panic!("Role has expired");
        }
        if Self::role_ordinal(member.role) > Self::role_ordinal(min_role) {
            panic!("Insufficient role");
        }
    }

    fn append_access_audit(
        env: &Env,
        operation: Symbol,
        caller: &Address,
        target: Option<Address>,
        success: bool,
    ) {
        let mut entries: Vec<AccessAuditEntry> = env
            .storage()
            .instance()
            .get(&symbol_short!("ACC_AUDIT"))
            .unwrap_or_else(|| Vec::new(env));
        entries.push_back(AccessAuditEntry {
            operation,
            caller: caller.clone(),
            target,
            timestamp: env.ledger().timestamp(),
            success,
        });
        let n = entries.len();
        if n > MAX_ACCESS_AUDIT_ENTRIES {
            let mut v = Vec::new(env);
            let start = n - MAX_ACCESS_AUDIT_ENTRIES;
            for i in start..n {
                v.push_back(entries.get(i).unwrap_or_else(|| panic!("Item not found")));
            }
            entries = v;
        }
        env.storage()
            .instance()
            .set(&symbol_short!("ACC_AUDIT"), &entries);
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

    fn require_not_paused(env: &Env) {
        if Self::get_global_paused(env) {
            panic!("Contract is paused");
        }
    }

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
        let pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .unwrap_or_else(|| Map::new(env));

        let archived: Map<u64, ArchivedTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_TX"))
            .unwrap_or_else(|| Map::new(env));

        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .unwrap_or_else(|| Map::new(env));

        let mut pending_count = 0u32;
        for _ in pending_txs.iter() {
            pending_count += 1;
        }

        let mut archived_count = 0u32;
        for _ in archived.iter() {
            archived_count += 1;
        }

        let mut member_count = 0u32;
        for _ in members.iter() {
            member_count += 1;
        }

        let stats = StorageStats {
            pending_transactions: pending_count,
            archived_transactions: archived_count,
            total_members: member_count,
            last_updated: env.ledger().timestamp(),
        };

        env.storage()
            .instance()
            .set(&symbol_short!("STOR_STAT"), &stats);
    }
}

#[cfg(test)]
mod test;
