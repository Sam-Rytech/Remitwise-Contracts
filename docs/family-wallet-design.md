# Family Wallet - Spending Limit Precision and Rollover Validation

## Overview

This document describes the enhanced spending limit system implemented in the `family_wallet` contract, focusing on precision handling and rollover behavior to prevent over-withdrawal due to precision or period reset edge cases.

## Enhanced Features

### 1. Precision Spending Limits

The contract now supports enhanced precision controls beyond basic per-transaction limits:

```rust
/// Enhanced spending limit with precision controls
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

**Key Security Features:**
- **Dust Attack Prevention**: `min_precision` prevents micro-transactions that could bypass limits
- **Single Transaction Limits**: `max_single_tx` prevents large withdrawals even within period limits
- **Overflow Protection**: All arithmetic uses `saturating_add()` to prevent overflow
- **Configuration Validation**: Strict parameter validation prevents invalid configurations

### 2. Rollover Behavior

The system implements daily spending periods with secure rollover handling:

```rust
/// Spending period configuration for rollover behavior
pub struct SpendingPeriod {
    /// Period type: 0=Daily, 1=Weekly, 2=Monthly
    pub period_type: u32,
    /// Period start timestamp (aligned to period boundary)
    pub period_start: u64,
    /// Period duration in seconds (86400 for daily)
    pub period_duration: u64,
}
```

**Rollover Security:**
- **UTC Alignment**: Periods align to 00:00 UTC to prevent timezone manipulation
- **Boundary Validation**: Inclusive boundary checks prevent edge case timing attacks
- **Legitimate Rollover**: Validates rollover conditions to prevent time manipulation

### 3. Cumulative Spending Tracking

```rust
/// Cumulative spending tracking for precision validation
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

**Tracking Features:**
- **Period Persistence**: Spending accumulates across transactions within the same period
- **Automatic Reset**: Counters reset to zero on legitimate period rollover
- **Audit Trail**: Transaction count and timestamps for monitoring
- **Overflow Protection**: Uses saturating arithmetic to prevent overflow attacks

## API Reference

### Configuration Functions

#### `set_precision_spending_limit`
```rust
pub fn set_precision_spending_limit(
    env: Env,
    caller: Address,           // Must be Owner or Admin
    member_address: Address,   // Target member
    precision_limit: PrecisionSpendingLimit,
) -> Result<bool, Error>
```

**Purpose**: Configure enhanced precision limits for a family member  
**Authorization**: Owner or Admin only  
**Validation**: Validates all precision parameters for security

### Validation Functions

#### `validate_precision_spending`
```rust
pub fn validate_precision_spending(
    env: Env,
    caller: Address,
    amount: i128,
) -> Result<(), Error>
```

**Purpose**: Comprehensive spending validation with precision and rollover checks  
**Flow**:
1. Basic validation (positive amount, valid member, role not expired)
2. Role-based bypass (Owner/Admin unlimited)
3. Precision validation (min_precision, max_single_tx)
4. Cumulative validation (period limits, rollover handling)

### Monitoring Functions

#### `get_spending_tracker`
```rust
pub fn get_spending_tracker(env: Env, member_address: Address) -> Option<SpendingTracker>
```

**Purpose**: Read-only access to current spending tracker for monitoring

## Security Assumptions

### 1. Precision Attack Prevention

**Dust Attack Mitigation:**
- `min_precision > 0` prevents micro-transactions
- Recommended minimum: 1 XLM (10^7 stroops) for meaningful amounts

**Overflow Protection:**
- All arithmetic uses `saturating_add()` and `saturating_sub()`
- Configuration validation prevents overflow conditions
- Boundary checks handle edge cases gracefully

### 2. Rollover Security

**Time Manipulation Prevention:**
- Period alignment to UTC boundaries prevents timezone exploitation
- Rollover validation ensures legitimate period transitions
- Inclusive boundary checks prevent timing attacks

**Example Rollover Validation:**
```rust
fn rollover_spending_period(
    old_tracker: SpendingTracker,
    current_time: u64,
) -> Result<SpendingTracker, Error> {
    let new_period = Self::get_current_period(current_time);
    
    // Validate rollover is legitimate (prevent manipulation)
    if current_time < old_tracker.period.period_start.saturating_add(old_tracker.period.period_duration) {
        return Err(Error::RolloverValidationFailed);
    }
    
    // Reset counters for new period
    Ok(SpendingTracker {
        current_spent: 0,
        last_tx_timestamp: current_time,
        tx_count: 0,
        period: new_period,
    })
}
```

### 3. Boundary Validation

**Edge Case Handling:**
- Zero and negative amounts explicitly rejected
- Maximum single transaction enforced before cumulative checks
- Period boundary calculations handle timestamp overflow
- Configuration parameters validated for consistency

## Error Handling

### New Error Types

| Error | Description | Prevention |
|-------|-------------|------------|
| `AmountBelowPrecision` | Amount below minimum precision threshold | Set appropriate `min_precision` |
| `ExceedsMaxSingleTx` | Single transaction exceeds maximum | Configure reasonable `max_single_tx` |
| `ExceedsPeriodLimit` | Cumulative spending exceeds period limit | Monitor via `get_spending_tracker` |
| `RolloverValidationFailed` | Period rollover validation failed | System prevents time manipulation |
| `InvalidPrecisionConfig` | Invalid precision configuration | Validate parameters before setting |

### Error Prevention Strategies

**Configuration Validation:**
```rust
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
```

## Migration and Compatibility

### Backward Compatibility

**Legacy Support:**
- Existing members without `precision_limit` use legacy validation
- Legacy `spending_limit` field preserved
- New features are opt-in per member

**Migration Path:**
1. Deploy enhanced contract
2. Existing members continue with legacy limits
3. Gradually migrate via `set_precision_spending_limit`
4. Monitor through `get_spending_tracker`

### Configuration Examples

**Production Configuration:**
```rust
PrecisionSpendingLimit {
    limit: 10000_0000000,      // 10,000 XLM per day
    min_precision: 1_0000000,  // 1 XLM minimum (prevents dust)
    max_single_tx: 5000_0000000, // 5,000 XLM max per transaction
    enable_rollover: true,     // Enable cumulative tracking
}
```

**Conservative Configuration:**
```rust
PrecisionSpendingLimit {
    limit: 1000_0000000,       // 1,000 XLM per day
    min_precision: 5_0000000,  // 5 XLM minimum
    max_single_tx: 500_0000000, // 500 XLM max per transaction
    enable_rollover: true,
}
```

## Testing Strategy

### Test Coverage Areas

1. **Precision Validation**
   - Configuration parameter validation
   - Minimum precision enforcement
   - Maximum single transaction limits
   - Authorization checks

2. **Rollover Behavior**
   - Period alignment and boundaries
   - Spending tracker persistence
   - Legitimate rollover validation
   - Counter reset behavior

3. **Security Edge Cases**
   - Dust attack prevention
   - Overflow protection
   - Time manipulation resistance
   - Boundary condition handling

4. **Compatibility**
   - Legacy limit fallback
   - Owner/Admin bypass
   - Mixed configurations
   - Migration scenarios

### Running Tests

```bash
# Run all family wallet tests
cargo test -p family_wallet

# Run precision-specific tests
cargo test -p family_wallet test_precision
cargo test -p family_wallet test_rollover
cargo test -p family_wallet test_cumulative

# Run with detailed output
cargo test -p family_wallet -- --nocapture
```

## Performance Considerations

### Storage Efficiency

- **Minimal Footprint**: One `SpendingTracker` per member with precision limits
- **Automatic Cleanup**: Trackers reset on period rollover
- **Efficient Access**: O(1) lookups for validation

### Gas Optimization

- **Early Exits**: Owner/Admin bypass all precision checks
- **Conditional Logic**: Legacy members skip precision validation
- **Batch Operations**: Minimize storage reads/writes

## Conclusion

The enhanced spending limit system provides robust protection against precision attacks and rollover edge cases while maintaining backward compatibility. The implementation follows security best practices with comprehensive validation, overflow protection, and audit trails.

Key benefits:
- **Prevents over-withdrawal** through precision and cumulative validation
- **Secure rollover behavior** with time manipulation resistance  
- **Comprehensive testing** covering security edge cases
- **Backward compatible** with existing configurations
- **Well-documented** security assumptions and validation logic

This implementation ensures that family wallet spending limits are enforced securely and precisely, preventing both accidental and malicious attempts to bypass spending controls.