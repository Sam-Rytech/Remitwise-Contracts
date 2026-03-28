#![no_std]
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short,
    Address, Env, Map, String, Vec,
};

// ── Coverage type ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum CoverageType {
    Health    = 1,
    Life      = 2,
    Property  = 3,
    Auto      = 4,
    Liability = 5,
}

// ── Errors ────────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum InsuranceError {
    Unauthorized    = 1,
    InvalidAmount   = 2,
    PolicyNotFound  = 3,
    PolicyInactive  = 4,
    InvalidPremium  = 5,
    InvalidCoverage = 6,
}

// ── Storage types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub struct InsurancePolicy {
    pub id: u32,
    pub owner: Address,
    pub name: String,
    pub coverage_type: CoverageType,
    pub monthly_premium: i128,
    pub coverage_amount: i128,
    pub active: bool,
    pub next_payment_date: u64,
    pub external_ref: Option<String>,
    /// Policy tags — deduplicated, max 32 chars each.
    pub tags: Vec<String>,
}

// ── Storage keys ──────────────────────────────────────────────────────────────

const KEY_POLICIES: soroban_sdk::Symbol = symbol_short!("POLICIES");
const KEY_NEXT_ID:  soroban_sdk::Symbol = symbol_short!("NEXT_ID");
const KEY_ADMIN:    soroban_sdk::Symbol = symbol_short!("ADMIN");

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct Insurance;

#[contractimpl]
impl Insurance {

    // ── Admin ─────────────────────────────────────────────────────────────────

    /// @notice Set or transfer the contract admin role.
    ///
    /// @dev On first call (no admin set) `caller` must equal `new_admin` —
    ///      this bootstraps the admin without a separate init step.
    ///      On subsequent calls the existing admin must sign.
    ///
    /// @param caller    The address authorising this call. Must be the current
    ///                  admin, or equal to `new_admin` if no admin is set yet.
    /// @param new_admin The address to install as the new admin.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) {
        caller.require_auth();
        let current: Option<Address> = env.storage().instance().get(&KEY_ADMIN);
        if let Some(ref admin) = current {
            if *admin != caller {
                panic!("unauthorized");
            }
        }
        env.storage().instance().set(&KEY_ADMIN, &new_admin);
    }

    fn get_admin(env: &Env) -> Option<Address> {
        env.storage().instance().get(&KEY_ADMIN)
    }

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn load_policies(env: &Env) -> Map<u32, InsurancePolicy> {
        env.storage()
            .instance()
            .get(&KEY_POLICIES)
            .unwrap_or_else(|| Map::new(env))
    }

    fn save_policies(env: &Env, m: &Map<u32, InsurancePolicy>) {
        env.storage().instance().set(&KEY_POLICIES, m);
    }

    fn bump(env: &Env) {
        env.storage().instance().extend_ttl(17280, 518400);
    }

    // ── Policy CRUD ───────────────────────────────────────────────────────────

    /// @notice Create a new insurance policy for the caller.
    ///
    /// @dev Assigns a monotonically increasing `u32` ID starting at 1.
    ///      The policy is stored in instance storage and the TTL is bumped.
    ///      `tags` is initialised to an empty vector.
    ///
    /// @param owner           The policyholder address. Must sign the transaction.
    /// @param name            Human-readable policy label (1–64 bytes).
    /// @param coverage_type   One of the `CoverageType` enum variants.
    /// @param monthly_premium Monthly cost in stroops. Must be > 0.
    /// @param coverage_amount Insured value in stroops. Must be > 0.
    ///
    /// @return The newly assigned policy ID (`u32`).
    pub fn create_policy(
        env: Env,
        owner: Address,
        name: String,
        coverage_type: CoverageType,
        monthly_premium: i128,
        coverage_amount: i128,
    ) -> u32 {
        owner.require_auth();
        if monthly_premium <= 0 {
            panic!("Monthly premium must be positive");
        }
        if coverage_amount <= 0 {
            panic!("Coverage amount must be positive");
        }
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let next_id: u32 = env
            .storage()
            .instance()
            .get(&KEY_NEXT_ID)
            .unwrap_or(0u32)
            + 1;

        let policy = InsurancePolicy {
            id: next_id,
            owner: owner.clone(),
            name,
            coverage_type,
            monthly_premium,
            coverage_amount,
            active: true,
            next_payment_date: env.ledger().timestamp() + 30 * 86400,
            external_ref: None,
            tags: Vec::new(&env),
        };

        policies.set(next_id, policy);
        Self::save_policies(&env, &policies);
        env.storage().instance().set(&KEY_NEXT_ID, &next_id);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("created")),
            (next_id, owner),
        );

        next_id
    }

    /// @notice Record a premium payment and advance the next-due date by 30 days.
    ///
    /// @dev Only the policy owner may pay. Paying on an inactive policy returns
    ///      `InsuranceError::PolicyInactive` rather than panicking.
    ///
    /// @param caller    The address making the payment. Must be the policy owner.
    /// @param policy_id The ID of the policy to pay.
    ///
    /// @return `Ok(())` on success.
    ///
    /// @custom:error `PolicyNotFound` — `policy_id` does not exist.
    /// @custom:error `Unauthorized`   — `caller` is not the policy owner.
    /// @custom:error `PolicyInactive` — the policy has been deactivated.
    pub fn pay_premium(env: Env, caller: Address, policy_id: u32) -> Result<(), InsuranceError> {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }
        if !policy.active {
            return Err(InsuranceError::PolicyInactive);
        }

        policy.next_payment_date = env.ledger().timestamp() + 30 * 86400;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("paid")),
            (policy_id, caller),
        );
        Ok(())
    }

    /// @notice Deactivate a policy, preventing future premium payments.
    ///
    /// @dev Sets `policy.active = false`. The record is retained in storage
    ///      for historical queries. Only the policy owner may deactivate.
    ///
    /// @param caller    The address requesting deactivation. Must be the policy owner.
    /// @param policy_id The ID of the policy to deactivate.
    ///
    /// @return `Ok(true)` on success.
    ///
    /// @custom:error `PolicyNotFound` — `policy_id` does not exist.
    /// @custom:error `Unauthorized`   — `caller` is not the policy owner.
    pub fn deactivate_policy(
        env: Env,
        caller: Address,
        policy_id: u32,
    ) -> Result<bool, InsuranceError> {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .ok_or(InsuranceError::PolicyNotFound)?;

        if policy.owner != caller {
            return Err(InsuranceError::Unauthorized);
        }

        policy.active = false;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        env.events().publish(
            (symbol_short!("insure"), symbol_short!("deactive")),
            (policy_id, caller),
        );
        Ok(true)
    }

    /// @notice Retrieve a policy record by its ID.
    ///
    /// @param policy_id The ID of the policy to look up.
    ///
    /// @return `Some(InsurancePolicy)` if found, `None` otherwise.
    pub fn get_policy(env: Env, policy_id: u32) -> Option<InsurancePolicy> {
        Self::load_policies(&env).get(policy_id)
    }

    /// @notice Return all active policies belonging to a given owner.
    ///
    /// @dev Performs a full scan of instance storage. For large portfolios
    ///      consider paginating at the application layer.
    ///
    /// @param owner The address whose active policies are requested.
    ///
    /// @return A `Vec<InsurancePolicy>` containing only active policies for `owner`.
    pub fn get_active_policies(env: Env, owner: Address) -> Vec<InsurancePolicy> {
        let policies = Self::load_policies(&env);
        let mut result = Vec::new(&env);
        for (_, p) in policies.iter() {
            if p.active && p.owner == owner {
                result.push_back(p);
            }
        }
        result
    }

    /// @notice Sum the monthly premiums of all active policies for an owner.
    ///
    /// @dev Uses `saturating_add` to prevent overflow on very large portfolios.
    ///      Inactive policies are excluded from the sum.
    ///
    /// @param owner The address whose premium total is requested.
    ///
    /// @return Total monthly premium in stroops (`i128`). Returns 0 if the
    ///         owner has no active policies.
    pub fn get_total_monthly_premium(env: Env, owner: Address) -> i128 {
        let policies = Self::load_policies(&env);
        let mut total = 0i128;
        for (_, p) in policies.iter() {
            if p.active && p.owner == owner {
                total = total.saturating_add(p.monthly_premium);
            }
        }
        total
    }

    // ── Tag management ────────────────────────────────────────────────────────

    /// @notice Attach a string label (tag) to a policy.
    ///
    /// @dev **Authorization** — `caller` must be either the policy owner or the
    ///      contract admin. Both roles are checked after `require_auth()`.
    ///
    ///      **Deduplication** — before writing, the function performs a linear
    ///      scan of the policy's existing `tags` vector:
    ///
    ///      ```text
    ///      for existing in policy.tags.iter() {
    ///          if existing == tag { return; }  // early exit, no write, no event
    ///      }
    ///      ```
    ///
    ///      If the tag is already present the function returns immediately.
    ///      No storage write occurs and no event is emitted. This prevents
    ///      unbounded vector growth from repeated calls with the same value.
    ///      The check is byte-exact and case-sensitive.
    ///
    /// @param caller    The address requesting the operation. Must be the policy
    ///                  owner or the contract admin, and must sign the transaction.
    /// @param policy_id The ID of the policy to tag.
    /// @param tag       The label to attach. Must be 1-32 bytes (UTF-8).
    ///
    /// @custom:event `("insure", "tag_added")` — emitted with data `(policy_id, tag)`
    ///               only when the tag is new. Duplicate calls produce no event.
    ///
    /// @custom:panics `"policy not found"`           — `policy_id` does not exist.
    /// @custom:panics `"unauthorized"`               — caller is neither owner nor admin.
    /// @custom:panics `"tag must be 1-32 characters"` — tag length out of range.
    pub fn add_tag(env: Env, caller: Address, policy_id: u32, tag: String) {
        caller.require_auth();

        if tag.len() == 0 || tag.len() > 32 {
            panic!("tag must be 1-32 characters");
        }

        Self::bump(&env);
        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .unwrap_or_else(|| panic!("policy not found"));

        // Authorization: policy owner OR admin
        let is_owner = policy.owner == caller;
        let is_admin = Self::get_admin(&env).map(|a| a == caller).unwrap_or(false);
        if !is_owner && !is_admin {
            panic!("unauthorized");
        }

        // Deduplication: compare by reference so `tag` is not moved
        for existing in policy.tags.iter() {
            if existing == tag {
                return; // already exists — no write, no event
            }
        }

        // `tag` is still owned here — move it into storage then into the event
        policy.tags.push_back(tag.clone());
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        // Move `tag` into the event payload — only first (and only) move
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("tag_added")),
            (policy_id, tag),
        );
    }

    /// @notice Remove a string label from a policy.
    ///
    /// @dev **Authorization** — same two-role model as `add_tag`: caller must
    ///      be the policy owner or the contract admin.
    ///
    ///      **Graceful removal** — if the tag is not present the function does
    ///      NOT panic. Instead it emits a `tag_miss` event and returns. This
    ///      prevents a front-running denial-of-service where an attacker removes
    ///      a tag just before a legitimate call, causing that call to fail.
    ///
    ///      The removal rebuilds the tags vector, excluding the matched entry.
    ///      All other tags are preserved in their original order.
    ///
    /// @param caller    The address requesting the operation. Must be the policy
    ///                  owner or the contract admin, and must sign the transaction.
    /// @param policy_id The ID of the policy to modify.
    /// @param tag       The label to remove. Match is byte-exact and case-sensitive.
    ///
    /// @custom:event `("insure", "tag_rmvd")` — emitted with data `(policy_id, tag)`
    ///               when the tag was found and removed.
    /// @custom:event `("insure", "tag_miss")` — emitted with data `(policy_id, tag)`
    ///               when the tag was not present (graceful no-op path).
    ///
    /// @custom:panics `"policy not found"` — `policy_id` does not exist.
    /// @custom:panics `"unauthorized"`     — caller is neither owner nor admin.
    pub fn remove_tag(env: Env, caller: Address, policy_id: u32, tag: String) {
        caller.require_auth();
        Self::bump(&env);

        let mut policies = Self::load_policies(&env);
        let mut policy = policies
            .get(policy_id)
            .unwrap_or_else(|| panic!("policy not found"));

        // Authorization: policy owner OR admin
        let is_owner = policy.owner == caller;
        let is_admin = Self::get_admin(&env).map(|a| a == caller).unwrap_or(false);
        if !is_owner && !is_admin {
            panic!("unauthorized");
        }

        // Find and remove the tag — compare by reference so `tag` is not moved
        let mut found = false;
        let mut new_tags = Vec::new(&env);
        for existing in policy.tags.iter() {
            if existing == tag {
                found = true; // skip — this is the removal
            } else {
                new_tags.push_back(existing);
            }
        }

        if !found {
            // Graceful: emit Tag Not Found event, do not panic.
            // Move `tag` here — this is the only move on the not-found path.
            env.events().publish(
                (symbol_short!("insure"), symbol_short!("tag_miss")),
                (policy_id, tag),
            );
            return;
        }

        policy.tags = new_tags;
        policies.set(policy_id, policy);
        Self::save_policies(&env, &policies);

        // Move `tag` here — only move on the found path
        env.events().publish(
            (symbol_short!("insure"), symbol_short!("tag_rmvd")),
            (policy_id, tag),
        );
    }
}

#[cfg(test)]
mod test;
