#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token::TokenClient, Address,
    Env, Map, Symbol, Vec,
};

// Storage TTL constants for active data
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const INSTANCE_BUMP_AMOUNT: u32 = 518400; // ~30 days

// Storage TTL constants for archived data (longer retention, less frequent access)
const ARCHIVE_LIFETIME_THRESHOLD: u32 = 17280; // ~1 day
const ARCHIVE_BUMP_AMOUNT: u32 = 2592000; // ~180 days (6 months)

// Signature expiration time (24 hours in seconds)
const SIGNATURE_EXPIRATION: u64 = 86400;

/// Transaction types that may require multi-signature approval
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum TransactionType {
    LargeWithdrawal = 1,
    SplitConfigChange = 2,
    RoleChange = 3,
    EmergencyTransfer = 4,
    PolicyCancellation = 5,
    RegularWithdrawal = 6, // Below threshold, no multi-sig needed
}

/// Family member roles (hierarchy: Owner > Admin > Member > Viewer)
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FamilyRole {
    Owner = 1,
    Admin = 2,
    Member = 3,
    Viewer = 4,
}

/// Multi-signature configuration for a transaction type
#[contracttype]
#[derive(Clone)]
pub struct MultiSigConfig {
    pub threshold: u32,        // Number of signatures required (e.g., 2 for 2-of-3)
    pub signers: Vec<Address>, // List of authorized signers
    pub spending_limit: i128,  // Amount threshold requiring multi-sig
}

/// Pending transaction awaiting signatures
#[contracttype]
#[derive(Clone)]
pub struct PendingTransaction {
    pub tx_id: u64,
    pub tx_type: TransactionType,
    pub proposer: Address,
    pub signatures: Vec<Address>, // Vec instead of Set (Soroban doesn't have Set)
    pub created_at: u64,
    pub expires_at: u64,
    pub data: TransactionData,
}

/// Transaction data payload - using tuple variants for Soroban compatibility
#[contracttype]
#[derive(Clone)]
pub enum TransactionData {
    Withdrawal(Address, Address, i128), // (token, recipient, amount)
    SplitConfigChange(u32, u32, u32, u32), // (spending, savings, bills, insurance)
    RoleChange(Address, FamilyRole),    // (member, new_role)
    EmergencyTransfer(Address, Address, i128), // (token, recipient, amount)
    PolicyCancellation(u32),            // (policy_id)
}

/// Family member information
#[contracttype]
#[derive(Clone)]
pub struct FamilyMember {
    pub address: Address,
    pub role: FamilyRole,
    /// Per-transaction cap in stroops. 0 = unlimited.
    pub spending_limit: i128,
    pub added_at: u64,
}

/// Emergency transfer configuration
#[contracttype]
#[derive(Clone)]
pub struct EmergencyConfig {
    /// Maximum amount allowed per emergency transfer
    pub max_amount: i128,
    /// Cooldown period in seconds between emergency transfers
    pub cooldown: u64,
    /// Required minimum balance remaining after emergency transfer
    pub min_balance: i128,
}

/// Events for emergency mode and transfers (for notifications / audit trail)
#[contracttype]
#[derive(Clone)]
pub enum EmergencyEvent {
    ModeOn,
    ModeOff,
    TransferInit,
    TransferExec,
}

/// Emitted by add_member.
#[contracttype]
#[derive(Clone)]
pub struct MemberAddedEvent {
    pub member: Address,
    pub role: FamilyRole,
    pub spending_limit: i128,
    pub timestamp: u64,
}

/// Emitted by update_spending_limit.
#[contracttype]
#[derive(Clone)]
pub struct SpendingLimitUpdatedEvent {
    pub member: Address,
    pub old_limit: i128,
    pub new_limit: i128,
    pub timestamp: u64,
}

/// Archived transaction - compressed record with essential fields only
#[contracttype]
#[derive(Clone)]
pub struct ArchivedTransaction {
    pub tx_id: u64,
    pub tx_type: TransactionType,
    pub proposer: Address,
    pub executed_at: u64,
    pub archived_at: u64,
}

/// Storage statistics for monitoring
#[contracttype]
#[derive(Clone)]
pub struct StorageStats {
    pub pending_transactions: u32,
    pub archived_transactions: u32,
    pub total_members: u32,
    pub last_updated: u64,
}

/// Access audit entry for role/access changes (audit logging)
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

/// Item for batch_add_family_members
#[contracttype]
#[derive(Clone)]
pub struct BatchMemberItem {
    pub address: Address,
    pub role: FamilyRole,
}

/// Events for archival operations
#[contracttype]
#[derive(Clone)]
pub enum ArchiveEvent {
    TransactionsArchived,
    ExpiredCleaned,
}

/// Multi-signature wallet contract
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
}

#[contractimpl]
impl FamilyWallet {
    /// Initialize the family wallet
    pub fn init(env: Env, owner: Address, initial_members: Vec<Address>) -> bool {
        owner.require_auth();

        // Check if already initialized
        let existing: Option<Address> = env.storage().instance().get(&symbol_short!("OWNER"));

        if existing.is_some() {
            panic!("Wallet already initialized");
        }

        Self::extend_instance_ttl(&env);

        // Store owner
        env.storage()
            .instance()
            .set(&symbol_short!("OWNER"), &owner);

        // Initialize members map
        let mut members: Map<Address, FamilyMember> = Map::new(&env);
        let timestamp = env.ledger().timestamp();

        // Add owner as Owner role
        members.set(
            owner.clone(),
            FamilyMember {
                address: owner.clone(),
                role: FamilyRole::Owner,
                spending_limit: 0,
                added_at: timestamp,
            },
        );

        // Add initial members as Member role
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

        // Initialize multi-sig configs with defaults
        let default_config = MultiSigConfig {
            threshold: 2,
            signers: Vec::new(&env),
            spending_limit: 1000_0000000, // 1000 tokens (assuming 7 decimals)
        };

        // Set default configs for each transaction type
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

        // Initialize pending transactions map
        env.storage().instance().set(
            &symbol_short!("PEND_TXS"),
            &Map::<u64, PendingTransaction>::new(&env),
        );

        // Initialize executed transactions map (for replay prevention)
        env.storage()
            .instance()
            .set(&symbol_short!("EXEC_TXS"), &Map::<u64, bool>::new(&env));

        // Initialize next transaction ID
        env.storage()
            .instance()
            .set(&symbol_short!("NEXT_TX"), &1u64);

        // Initialize default emergency configuration
        let em_config = EmergencyConfig {
            max_amount: 1000_0000000, // default: 1000 tokens
            cooldown: 3600,           // default: 1 hour
            min_balance: 0,           // default: no minimum balance
        };
        env.storage()
            .instance()
            .set(&symbol_short!("EM_CONF"), &em_config);
        // Emergency mode off by default
        env.storage()
            .instance()
            .set(&symbol_short!("EM_MODE"), &false);
        // No emergency transfer has happened yet
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
            .expect("Wallet not initialized");

        if members.get(member_address.clone()).is_some() {
            return Err(Error::InvalidRole); // member already exists
        }

        Self::extend_instance_ttl(&env);

        let now = env.ledger().timestamp();
        members.set(
            member_address.clone(),
            FamilyMember {
                address: member_address.clone(),
                role: role.clone(),
                spending_limit,
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
            .expect("Wallet not initialized");

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
            .expect("Wallet not initialized");

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

    pub fn check_spending_limit(env: Env, member_address: Address, amount: i128) -> bool {
        if amount < 0 {
            return false;
        }

        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");

        match members.get(member_address) {
            None => false,
            Some(record) => record.spending_limit == 0 || amount <= record.spending_limit,
        }
    }

    /// Configure multi-signature settings for a transaction type
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
            .expect("Wallet not initialized");

        // Verify caller is Owner or Admin
        if !Self::is_owner_or_admin_in_members(&members, &caller) {
            panic!("Only Owner or Admin can configure multi-sig");
        }

        // Validate threshold
        if threshold == 0 || threshold > signers.len() {
            panic!("Invalid threshold");
        }

        // Validate signers are family members
        for signer in signers.iter() {
            if members.get(signer.clone()).is_none() {
                panic!("Signer must be a family member");
            }
        }

        // Validate spending limit
        if spending_limit < 0 {
            panic!("Spending limit must be non-negative");
        }

        Self::extend_instance_ttl(&env);

        let config = MultiSigConfig {
            threshold,
            signers: signers.clone(),
            spending_limit,
        };

        env.storage()
            .instance()
            .set(&Self::get_config_key(tx_type), &config);

        true
    }

    /// Propose a transaction requiring multi-signature approval
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

        // For withdrawals, use LargeWithdrawal config to check spending limit
        // For other types, use their own config
        let config_key = match tx_type {
            TransactionType::RegularWithdrawal => {
                Self::get_config_key(TransactionType::LargeWithdrawal)
            }
            _ => Self::get_config_key(tx_type),
        };

        // Check if transaction requires multi-sig
        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&config_key)
            .expect("Multi-sig config not found");

        // For withdrawals, check if amount exceeds spending limit
        let requires_multisig = match (&tx_type, &data) {
            (TransactionType::RegularWithdrawal, TransactionData::Withdrawal(_, _, amount)) => {
                *amount > config.spending_limit
            }
            (TransactionType::LargeWithdrawal, _) => true,
            (TransactionType::RegularWithdrawal, _) => false,
            _ => true, // All other types require multi-sig
        };

        if !requires_multisig {
            // Execute immediately for regular withdrawals below threshold
            // Auth already required in propose_transaction, so don't require again
            return Self::execute_transaction_internal(&env, &proposer, &tx_type, &data, false);
        }

        Self::extend_instance_ttl(&env);

        // Get next transaction ID
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

        // Create pending transaction
        let timestamp = env.ledger().timestamp();
        let mut signatures = Vec::new(&env);
        signatures.push_back(proposer.clone()); // Proposer auto-signs

        let pending_tx = PendingTransaction {
            tx_id,
            tx_type,
            proposer: proposer.clone(),
            signatures,
            created_at: timestamp,
            expires_at: timestamp + SIGNATURE_EXPIRATION,
            data: data.clone(),
        };

        // Store pending transaction
        let mut pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .expect("Pending transactions map not initialized");

        pending_txs.set(tx_id, pending_tx);
        env.storage()
            .instance()
            .set(&symbol_short!("PEND_TXS"), &pending_txs);

        tx_id
    }

    /// Sign a pending transaction
    pub fn sign_transaction(env: Env, signer: Address, tx_id: u64) -> bool {
        signer.require_auth();
        Self::require_not_paused(&env);
        Self::require_role_at_least(&env, &signer, FamilyRole::Member);

        if !Self::is_family_member(&env, &signer) {
            panic!("Only family members can sign transactions");
        }

        Self::extend_instance_ttl(&env);

        // Get pending transaction
        let mut pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .expect("Pending transactions map not initialized");

        let mut pending_tx = pending_txs.get(tx_id).expect("Transaction not found");

        // Check if transaction expired
        let current_time = env.ledger().timestamp();
        if current_time > pending_tx.expires_at {
            panic!("Transaction expired");
        }

        // Check if already signed (check Vec for duplicates)
        for sig in pending_tx.signatures.iter() {
            if sig.clone() == signer {
                panic!("Already signed this transaction");
            }
        }

        // Get multi-sig config
        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&Self::get_config_key(pending_tx.tx_type))
            .expect("Multi-sig config not found");

        // Verify signer is authorized
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

        // Add signature
        pending_tx.signatures.push_back(signer.clone());

        // Check if threshold met
        if pending_tx.signatures.len() >= config.threshold {
            // Execute transaction - require proposer auth since we're executing from sign_transaction
            let executed = Self::execute_transaction_internal(
                &env,
                &pending_tx.proposer,
                &pending_tx.tx_type,
                &pending_tx.data,
                true, // Require auth since proposer hasn't authorized in this call
            );

            if executed == 0 {
                // Remove from pending
                pending_txs.remove(tx_id);
                env.storage()
                    .instance()
                    .set(&symbol_short!("PEND_TXS"), &pending_txs);

                // Add to executed map (for replay prevention)
                let mut executed_txs: Map<u64, bool> = env
                    .storage()
                    .instance()
                    .get(&symbol_short!("EXEC_TXS"))
                    .expect("Executed transactions map not initialized");

                executed_txs.set(tx_id, true);
                env.storage()
                    .instance()
                    .set(&symbol_short!("EXEC_TXS"), &executed_txs);
            }

            return true;
        }

        // Update pending transaction
        pending_txs.set(tx_id, pending_tx);
        env.storage()
            .instance()
            .set(&symbol_short!("PEND_TXS"), &pending_txs);

        true
    }

    /// Execute a large withdrawal (requires multi-sig)
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

        let config: MultiSigConfig = env
            .storage()
            .instance()
            .get(&Self::get_config_key(TransactionType::LargeWithdrawal))
            .expect("Multi-sig config not found");

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

    /// Execute a split configuration change (requires multi-sig)
    pub fn propose_split_config_change(
        env: Env,
        proposer: Address,
        spending_percent: u32,
        savings_percent: u32,
        bills_percent: u32,
        insurance_percent: u32,
    ) -> u64 {
        // Validate percentages sum to 100
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

    /// Propose a family member role change (requires multi-sig)
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

    /// Propose an emergency transfer (requires multi-sig)
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

        // If emergency mode is enabled, execute with simplified approval
        let em_mode: bool = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_MODE"))
            .unwrap_or(false);

        if em_mode {
            return Self::execute_emergency_transfer_now(env, proposer, token, recipient, amount);
        }

        Self::propose_transaction(
            env,
            proposer,
            TransactionType::EmergencyTransfer,
            TransactionData::EmergencyTransfer(token, recipient, amount),
        )
    }

    /// Propose a policy cancellation (requires multi-sig)
    pub fn propose_policy_cancellation(env: Env, proposer: Address, policy_id: u32) -> u64 {
        Self::propose_transaction(
            env,
            proposer,
            TransactionType::PolicyCancellation,
            TransactionData::PolicyCancellation(policy_id),
        )
    }

    /// Configure emergency transfer limits and rules
    ///
    /// Only Owner or Admin can update emergency configuration.
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

        let config = EmergencyConfig {
            max_amount,
            cooldown,
            min_balance,
        };

        env.storage()
            .instance()
            .set(&symbol_short!("EM_CONF"), &config);

        true
    }

    /// Activate or deactivate emergency mode
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

        // Emit event to notify all family members
        let event = if enabled {
            EmergencyEvent::ModeOn
        } else {
            EmergencyEvent::ModeOff
        };
        env.events()
            .publish((symbol_short!("emerg"), event), caller);

        true
    }

    /// Add a new family member (Owner or Admin only)
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
            .expect("Wallet not initialized");

        let timestamp = env.ledger().timestamp();
        members.set(
            member.clone(),
            FamilyMember {
                address: member.clone(),
                role,
                spending_limit: 0,
                added_at: timestamp,
            },
        );

        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        Self::append_access_audit(&env, symbol_short!("add_mem"), &caller, Some(member), true);
        true
    }

    /// Remove a family member (Owner only)
    pub fn remove_family_member(env: Env, caller: Address, member: Address) -> bool {
        caller.require_auth();
        Self::require_not_paused(&env);

        // Verify caller is Owner
        let owner: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .expect("Wallet not initialized");

        if caller != owner {
            panic!("Only Owner can remove family members");
        }

        // Cannot remove owner
        if member == owner {
            panic!("Cannot remove owner");
        }

        Self::extend_instance_ttl(&env);

        let mut members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");

        members.remove(member.clone());
        env.storage()
            .instance()
            .set(&symbol_short!("MEMBERS"), &members);

        Self::append_access_audit(&env, symbol_short!("rem_mem"), &caller, Some(member), true);
        true
    }

    /// Get pending transaction
    pub fn get_pending_transaction(env: Env, tx_id: u64) -> Option<PendingTransaction> {
        let pending_txs: Map<u64, PendingTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("PEND_TXS"))
            .expect("Pending transactions map not initialized");

        pending_txs.get(tx_id)
    }

    /// Get multi-sig configuration for a transaction type
    pub fn get_multisig_config(env: Env, tx_type: TransactionType) -> Option<MultiSigConfig> {
        env.storage().instance().get(&Self::get_config_key(tx_type))
    }

    /// Get family member information
    pub fn get_family_member(env: Env, member: Address) -> Option<FamilyMember> {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");

        members.get(member)
    }

    /// Get wallet owner
    pub fn get_owner(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .expect("Wallet not initialized")
    }

    /// Get current emergency configuration
    pub fn get_emergency_config(env: Env) -> Option<EmergencyConfig> {
        env.storage().instance().get(&symbol_short!("EM_CONF"))
    }

    /// Check if emergency mode is currently enabled
    pub fn is_emergency_mode(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("EM_MODE"))
            .unwrap_or(false)
    }

    /// Get timestamp of last emergency transfer, if any
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
    /// Check if a caller has permission to spend a given amount
    ///
    /// This function validates spending limits for orchestrator integration.
    /// It checks the caller's role and compares the amount against the configured
    /// spending limit for large withdrawals.
    ///
    /// # Arguments
    /// * `env` - The contract environment
    /// * `caller` - Address requesting permission to spend
    /// * `amount` - Amount to validate
    ///
    /// # Returns
    /// true if the caller can spend the amount, false otherwise
    ///
    /// # Logic
    /// - Owner and Admin: Can spend any amount
    /// - Members: Can spend up to the configured spending limit
    /// - Non-members: Cannot spend (returns false)
    pub fn check_spending_limit(env: Env, caller: Address, amount: i128) -> bool {
        // Get members map
        let members: Map<Address, FamilyMember> =
            match env.storage().instance().get(&symbol_short!("MEMBERS")) {
                Some(m) => m,
                None => return false, // Wallet not initialized
            };

        // Check if caller is a member
        let member = match members.get(caller.clone()) {
            Some(m) => m,
            None => return false, // Not a family member
        };

        // Owner and Admin have unlimited spending
        if member.role == FamilyRole::Owner || member.role == FamilyRole::Admin {
            return true;
        }

        // For regular members, check against spending limit
        let config: MultiSigConfig = match env
            .storage()
            .instance()
            .get(&Self::get_config_key(TransactionType::LargeWithdrawal))
        {
            Some(c) => c,
            None => return false, // Config not found
        };

        // Return true if amount is within spending limit
        amount <= config.spending_limit
    }

    /// Archive old executed transactions before the specified timestamp.
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be Owner or Admin)
    /// * `before_timestamp` - Archive transactions executed before this timestamp
    ///
    /// # Returns
    /// Number of transactions archived
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

        // Archive executed transactions (we don't have detailed data, just the fact they were executed)
        for (tx_id, _) in executed_txs.iter() {
            // Since we only have tx_id and executed status, create minimal archive
            let archived_tx = ArchivedTransaction {
                tx_id,
                tx_type: TransactionType::RegularWithdrawal, // Default type as we don't store this
                proposer: caller.clone(), // Use caller as we don't have original proposer
                executed_at: before_timestamp,
                archived_at: current_time,
            };
            archived.set(tx_id, archived_tx);
            archived_count += 1;
        }

        // Clear executed transactions map after archiving
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

    /// Get archived transactions with limit
    ///
    /// # Arguments
    /// * `limit` - Maximum number of transactions to return
    ///
    /// # Returns
    /// Vec of ArchivedTransaction structs
    pub fn get_archived_transactions(env: Env, limit: u32) -> Vec<ArchivedTransaction> {
        let archived: Map<u64, ArchivedTransaction> = env
            .storage()
            .instance()
            .get(&symbol_short!("ARCH_TX"))
            .unwrap_or_else(|| Map::new(&env));

        let mut result = Vec::new(&env);
        let mut count = 0u32;
        for (_, tx) in archived.iter() {
            if count >= limit {
                break;
            }
            result.push_back(tx);
            count += 1;
        }
        result
    }

    /// Cleanup expired pending transactions
    ///
    /// # Arguments
    /// * `caller` - Address of the caller (must be Owner or Admin)
    ///
    /// # Returns
    /// Number of expired transactions removed
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

    /// Get storage usage statistics
    ///
    /// # Returns
    /// StorageStats struct with current storage metrics
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

    /// Set optional role expiry for time-based access (Owner/Admin only).
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

    /// Pause contract (Owner or Admin only).
    pub fn pause(env: Env, caller: Address) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Admin);
        let admin = Self::get_pause_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .expect("Wallet not initialized")
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

    /// Unpause (pause admin only).
    pub fn unpause(env: Env, caller: Address) -> bool {
        caller.require_auth();
        let admin = Self::get_pause_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .expect("Wallet not initialized")
        });
        if admin != caller {
            panic!("Only pause admin can unpause");
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

    pub fn set_upgrade_admin(env: Env, caller: Address, new_admin: Address) -> bool {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Owner);
        env.storage()
            .instance()
            .set(&symbol_short!("UPG_ADM"), &new_admin);
        true
    }

    pub fn set_version(env: Env, caller: Address, new_version: u32) -> bool {
        caller.require_auth();
        let admin = Self::get_upgrade_admin(&env).unwrap_or_else(|| {
            env.storage()
                .instance()
                .get(&symbol_short!("OWNER"))
                .expect("Wallet not initialized")
        });
        if admin != caller {
            panic!("Only upgrade admin can set version");
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

    /// Batch add family members (Owner/Admin only). Atomic.
    pub fn batch_add_family_members(
        env: Env,
        caller: Address,
        members: Vec<BatchMemberItem>,
    ) -> u32 {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Admin);
        Self::require_not_paused(&env);
        if members.len() as u32 > MAX_BATCH_MEMBERS {
            panic!("Batch too large");
        }
        Self::extend_instance_ttl(&env);
        let mut members_map: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");
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
                    role: item.role.clone(),
                    spending_limit: 0,
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

    /// Batch remove family members (Owner only). Atomic.
    pub fn batch_remove_family_members(env: Env, caller: Address, addresses: Vec<Address>) -> u32 {
        caller.require_auth();
        Self::require_role_at_least(&env, &caller, FamilyRole::Owner);
        let owner: Address = env
            .storage()
            .instance()
            .get(&symbol_short!("OWNER"))
            .expect("Wallet not initialized");
        if caller != owner {
            panic!("Only Owner can remove members");
        }
        Self::require_not_paused(&env);
        if addresses.len() as u32 > MAX_BATCH_MEMBERS {
            panic!("Batch too large");
        }
        Self::extend_instance_ttl(&env);
        let mut members_map: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");
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

    /// Get recent access audit entries (read-only).
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

    // Internal helper functions

    /// Execute an emergency transfer immediately (emergency mode only)
    fn execute_emergency_transfer_now(
        env: Env,
        proposer: Address,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> u64 {
        // Load emergency configuration
        let config: EmergencyConfig = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_CONF"))
            .expect("Emergency config not set");

        if amount > config.max_amount {
            panic!("Emergency amount exceeds maximum allowed");
        }

        // Cooldown check
        let now = env.ledger().timestamp();
        let last_ts: u64 = env
            .storage()
            .instance()
            .get(&symbol_short!("EM_LAST"))
            .unwrap_or(0u64);
        if last_ts != 0 && now < last_ts.saturating_add(config.cooldown) {
            panic!("Emergency transfer cooldown period not elapsed");
        }

        // Balance check - ensure minimum remaining balance after transfer
        let token_client = TokenClient::new(&env, &token);
        let current_balance = token_client.balance(&proposer);
        if current_balance - amount < config.min_balance {
            panic!("Emergency transfer would violate minimum balance requirement");
        }

        // Emit initiation event (notification + audit)
        env.events().publish(
            (symbol_short!("emerg"), EmergencyEvent::TransferInit),
            (proposer.clone(), recipient.clone(), amount),
        );

        // Execute transfer with proposer authorization
        proposer.require_auth();
        let _ = Self::execute_transaction_internal(
            &env,
            &proposer,
            &TransactionType::EmergencyTransfer,
            &TransactionData::EmergencyTransfer(token.clone(), recipient.clone(), amount),
            false,
        );

        // Update last emergency timestamp
        let store_ts: u64 = if now == 0 { 1u64 } else { now };
        env.storage()
            .instance()
            .set(&symbol_short!("EM_LAST"), &store_ts);

        // Emit execution event
        env.events().publish(
            (symbol_short!("emerg"), EmergencyEvent::TransferExec),
            (proposer, recipient, amount),
        );

        // No pending transaction (one-click emergency)
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
                // Execute withdrawal - require proposer to authorize the transfer if needed
                if require_auth {
                    proposer.require_auth();
                }
                let token_client = TokenClient::new(env, token);
                token_client.transfer(proposer, recipient, amount);
                0 // Return 0 for immediate execution
            }
            (TransactionType::SplitConfigChange, TransactionData::SplitConfigChange(..)) => {
                // Split config changes would be handled by the remittance_split contract
                // This is a placeholder - in a real implementation, you'd call the split contract
                0
            }
            (TransactionType::RoleChange, TransactionData::RoleChange(member, new_role)) => {
                let mut members: Map<Address, FamilyMember> = env
                    .storage()
                    .instance()
                    .get(&symbol_short!("MEMBERS"))
                    .expect("Wallet not initialized");

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
                // Execute emergency transfer - require proposer to authorize the transfer if needed
                if require_auth {
                    proposer.require_auth();
                }
                let token_client = TokenClient::new(env, token);
                token_client.transfer(proposer, recipient, amount);
                0
            }
            (TransactionType::PolicyCancellation, TransactionData::PolicyCancellation(..)) => {
                // Policy cancellations would be handled by the insurance contract
                // This is a placeholder
                0
            }
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

        Self::is_owner_or_admin_in_members(&members, address)
    }

    fn is_owner_or_admin_in_members(
        members: &Map<Address, FamilyMember>,
        address: &Address,
    ) -> bool {
        if let Some(member) = members.get(address.clone()) {
            matches!(member.role, FamilyRole::Owner | FamilyRole::Admin)
        } else {
            false
        }
    }

    /// Role hierarchy: Owner(1) > Admin(2) > Member(3) > Viewer(4). Lower ordinal = higher privilege.
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
    /// Panics if caller does not have at least min_role or role has expired.
    fn require_role_at_least(env: &Env, caller: &Address, min_role: FamilyRole) {
        let members: Map<Address, FamilyMember> = env
            .storage()
            .instance()
            .get(&symbol_short!("MEMBERS"))
            .expect("Wallet not initialized");
        let member = members.get(caller.clone()).expect("Not a family member");
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
                v.push_back(entries.get(i).unwrap());
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

    /// Extend the TTL of archive storage with longer duration
    fn extend_archive_ttl(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(ARCHIVE_LIFETIME_THRESHOLD, ARCHIVE_BUMP_AMOUNT);
    }

    /// Update storage statistics
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
