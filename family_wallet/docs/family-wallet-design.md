# Family Wallet Design (Enhanced with Precision & Rollover Validation)

## Overview

The `FamilyWallet` contract provides policy controls for shared-family spending with enhanced precision handling and rollover behavior. This document describes the current implementation including the new spending limit precision and rollover validation features.

---

## Enhanced Spending Limit System

### Legacy vs Precision Limits

The contract supports both legacy per-transaction limits and enhanced precision limits:

| Feature | Legacy Limits | Precision Limits |
|---------|---------------|------------------|
| Scope | Per-transaction only | Per-transaction + cumulative |
| Precision | Basic i128 validation | Minimum precision + overflow protection |
| Rollover | None | Daily period rollover |
| Rate Limiting | None | Transaction count tracking |
| Security | Basic amount checks | Comprehensive boundary validation |

### Precision Spending Limit Configuration

```rust
pub struct PrecisionSpendingLimit {
    /// Base spending limit per period (in stroops)
    pub limit: i128,
    /// Minimum precision unit - prevents dust attacks (in stroops)
    pub min_precision: i128,
    /// Maximum single transaction amount (in stroops)
    pub max_single_tx: i128,
    /// Enable rollover validation and cumulative tracking
    pub enable_rollover: bool,
}
```

**Security Assumptions:**
- `limit >= 0` - Prevents negative spending limits
- `min_precision > 0` - Prevents dust/precision attacks
- `max_single_tx > 0 && max_single_tx <= limit` - Prevents single large withdrawals
- `enable_rollover` controls cumulative vs per-transaction validation

### Spending Period & Rollover Behavior

```rust
pub struct SpendingPeriod {
    /// Period type: 0=Daily, 1=Weekly, 2=Monthly
    pub period_type: u32,
    /// Period start timestamp (aligned to period boundary)
    pub period_start: u64,
    /// Period duration in seconds
    pub period_duration: u64,
}
```

**Period Alignment:**
- Daily periods align to 00:00 UTC to prevent timezone manipulation
- Period boundaries use `(timestamp / 86400) * 86400` for consistent alignment
- Rollover occurs at `period_start + period_duration` (inclusive boundary)

### Cumulative Spending Tracking

```rust
pub struct SpendingTracker {
    /// Current period spending amount (in stroops)
    pub current_spent: i128,
    /// Last transaction timestamp for audit trail
    pub last_tx_timestamp: u64,
    /// Transaction count in current period
    pub tx_count: u32,
    /// Period configuration
    pub period: SpendingPeriod,
}
```

**Tracking Behavior:**
- Resets to zero on period rollover
- Uses `saturating_add()` to prevent overflow
- Maintains transaction count for rate limiting analysis
- Persists across contract calls within the same period

---

## Validation Flow

### Enhanced Spending Validation Process

```
1. Basic Validation
   ├── amount > 0 ✓
   ├── caller is family member ✓
   └── role not expired ✓

2. Role-Based Bypass
   ├── Owner → Allow (unlimited) ✓
   ├── Admin → Allow (unlimited) ✓
   └── Member → Continue to precision checks

3. Precision Configuration Check
   ├── No precision_limit → Use legacy validation
   └── Has precision_limit → Continue to precision validation

4. Precision Validation
   ├── amount >= min_precision ✓
   ├── amount <= max_single_tx ✓
   └── rollover_enabled → Continue to cumulative checks

5. Cumulative Validation (if rollover enabled)
   ├── Check period rollover → Reset if needed
   ├── current_spent + amount <= limit ✓
   └── Update spending tracker
```

### Rollover Validation Security

**Period Rollover Conditions:**
```rust
fn should_rollover_period(period: &SpendingPeriod, current_time: u64) -> bool {
    current_time >= period.period_start.saturating_add(period.period_duration)
}
```

**Rollover Security Checks:**
- Validates rollover is legitimate (prevents time manipulation)
- Resets spending counters to prevent carryover attacks
- Maintains audit trail through transaction count reset
- Uses inclusive boundary (`>=`) to prevent edge case exploits

---

## API Reference

### New Functions

#### `set_precision_spending_limit`
```rust
pub fn set_precision_spending_limit(
    env: Env,
    caller: Address,
    member_address: Address,
    precision_limit: PrecisionSpendingLimit,
) -> Result<bool, Error>
```

**Authorization:** Owner or Admin only  
**Purpose:** Configure enhanced precision limits for a family member  
**Validation:** Validates all precision parameters for security

#### `validate_precision_spending`
```rust
pub fn validate_precision_spending(
    env: Env,
    caller: Address,
    amount: i128,
) -> Result<(), Error>
```

**Purpose:** Comprehensive spending validation with precision and rollover checks  
**Returns:** `Ok(())` if allowed, specific `Error` if validation fails

#### `get_spending_tracker`
```rust
pub fn get_spending_tracker(env: Env, member_address: Address) -> Option<SpendingTracker>
```

**Purpose:** Read-only access to spending tracker for monitoring  
**Returns:** Current spending tracker if exists

### Enhanced Error Types

| Error | Code | Description |
|-------|------|-------------|
| `AmountBelowPrecision` | 14 | Amount below minimum precision threshold |
| `ExceedsMaxSingleTx` | 15 | Single transaction exceeds maximum allowed |
| `ExceedsPeriodLimit` | 16 | Cumulative spending would exceed period limit |
| `RolloverValidationFailed` | 17 | Period rollover validation failed |
| `InvalidPrecisionConfig` | 18 | Invalid precision configuration parameters |

---

## Security Considerations

### Precision Attack Prevention

**Dust Attack Mitigation:**
- `min_precision` prevents micro-transactions that could bypass limits
- Minimum precision should be set to meaningful amounts (e.g., 1 XLM = 10^7 stroops)

**Overflow Protection:**
- Uses `saturating_add()` for all arithmetic operations
- Validates configuration parameters to prevent overflow conditions
- Checks cumulative spending before updating tracker

### Rollover Security

**Time Manipulation Prevention:**
- Period alignment to UTC boundaries prevents timezone exploitation
- Rollover validation ensures legitimate period transitions
- Inclusive boundary checks prevent edge case timing attacks

**Cumulative Limit Bypass Prevention:**
- Spending tracker persists across transactions within period
- Period rollover resets counters only at legitimate boundaries
- Transaction count tracking enables rate limiting analysis

### Boundary Validation

**Edge Case Handling:**
- Zero and negative amounts explicitly rejected
- Maximum single transaction enforced before cumulative checks
- Period boundary calculations handle timestamp overflow gracefully

---

## Migration & Compatibility

### Legacy Compatibility

**Backward Compatibility:**
- Existing members without `precision_limit` use legacy validation
- Legacy `spending_limit` field preserved for compatibility
- New precision features are opt-in per member

**Migration Path:**
1. Deploy enhanced contract
2. Existing members continue with legacy limits
3. Gradually migrate members to precision limits via `set_precision_spending_limit`
4. Monitor spending patterns through `get_spending_tracker`

### Configuration Recommendations

**Production Settings:**
```rust
PrecisionSpendingLimit {
    limit: 10000_0000000,      // 10,000 XLM per day
    min_precision: 1_0000000,  // 1 XLM minimum (prevents dust)
    max_single_tx: 5000_0000000, // 5,000 XLM max per transaction
    enable_rollover: true,     // Enable cumulative tracking
}
```

**Testing Settings:**
```rust
PrecisionSpendingLimit {
    limit: 100_0000000,        // 100 XLM per day
    min_precision: 0_1000000,  // 0.1 XLM minimum
    max_single_tx: 50_0000000, // 50 XLM max per transaction
    enable_rollover: true,
}
```

---

## Testing Coverage

### Precision Validation Tests
- ✅ Configuration validation (invalid parameters)
- ✅ Authorization checks (Owner/Admin only)
- ✅ Minimum precision enforcement
- ✅ Maximum single transaction limits
- ✅ Cumulative spending validation

### Rollover Behavior Tests
- ✅ Period alignment to UTC boundaries
- ✅ Spending tracker persistence
- ✅ Period rollover and counter reset
- ✅ Rollover validation security
- ✅ Edge case boundary handling

### Compatibility Tests
- ✅ Legacy limit fallback behavior
- ✅ Owner/Admin bypass functionality
- ✅ Mixed legacy and precision configurations
- ✅ Migration scenarios

### Security Tests
- ✅ Dust attack prevention
- ✅ Overflow protection
- ✅ Time manipulation resistance
- ✅ Boundary condition validation
- ✅ Authorization bypass attempts

---

## Performance Considerations

### Storage Efficiency

**Spending Tracker Storage:**
- One `SpendingTracker` per member with precision limits
- Automatic cleanup on period rollover
- Minimal storage footprint (5 fields per tracker)

**Computation Efficiency:**
- Period calculations use simple integer arithmetic
- Rollover detection is O(1) operation
- Spending validation is O(1) with early exits

### Gas Optimization

**Validation Shortcuts:**
- Owner/Admin bypass all precision checks
- Legacy members skip precision validation
- Disabled rollover skips cumulative tracking

**Storage Access Patterns:**
- Single read for member configuration
- Single read/write for spending tracker
- Batch updates minimize storage operations

---

## Running Tests

```bash
# Run all family wallet tests
cargo test -p family_wallet

# Run only precision and rollover tests
cargo test -p family_wallet test_precision
cargo test -p family_wallet test_rollover
cargo test -p family_wallet test_cumulative

# Run with output for debugging
cargo test -p family_wallet -- --nocapture
```

Expected: All tests pass with comprehensive coverage of precision validation, rollover behavior, and security edge cases.