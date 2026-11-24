# Quorum Set Validation Engine Documentation

## Overview

The Quorum Set Validation Engine is a Wasm-based policy evaluator that validates Stellar SCP quorum set configurations on-chain. It enables validator operators to verify node trustworthiness programmatically before bringing nodes into active status, preventing misconfigured nodes from degrading network consensus health.

## Key Features

- **Threshold Analysis**: Validates that quorum thresholds are safe and achievable
- **Centralization Detection**: Identifies single organization or validator dominance risks
- **Transitive Trust Analysis**: Validates nested quorum set complexity and depth
- **Cycle Detection**: Prevents circular trust dependencies
- **Performance Optimized**: All validations complete in under 15ms for reconciliation safety
- **Customizable Policies**: Organizations can define their own trust validation rules

## Validation Rules

### 1. Threshold Validation
Ensures the quorum threshold is logically valid:
- Threshold must be > 0
- Threshold must be ≤ total number of validators in the set
- Threshold must allow for quorum intersection safety

**Example:**
```json
{
  "t": 2,
  "v": ["v1", "v2", "v3"],
  "innerSets": []
}
```
✓ Valid: threshold is 2 out of 3 validators
✗ Invalid: threshold is 5 out of 3 validators

### 2. Centralization Risk Detection
Prevents over-concentration of consensus power:
- Identifies single organization dominance
- Calculates organization percentage of quorum
- Compares against configurable maximum threshold (default: 70%)

**Example:**
```json
{
  "t": 3,
  "v": ["v1", "v2", "v3", "v4"],
  "innerSets": [],
  "validator_orgs": {
    "v1": "CentralBank",
    "v2": "CentralBank",
    "v3": "CentralBank",
    "v4": "SmallExchange"
  }
}
```
✗ Invalid: CentralBank controls 75% of quorum (exceeds 70% default)

### 3. Single Validator Dependency
Detects critical single-point failures:
- Flags quorums where a single validator achieves consensus
- Indicates extreme centralization risk

### 4. Transitive Trust Depth Analysis
Controls the complexity of nested quorum structures:
- Validates inner set nesting levels
- Prevents excessive complexity that hinders understanding and debugging
- Default max depth: 5 levels
- Helps avoid byzantine risk from deep dependencies

### 5. Cycle Detection
Prevents circular trust dependencies:
- Analyzes validator trust relationships
- Rejects configurations that could create Byzantine failures through cycles

## Configuration Policy

The `ValidationPolicy` struct controls validation behavior:

```rust
pub struct ValidationPolicy {
    /// Maximum allowed centralization percentage (0-100)
    pub max_centralization_pct: u32,  // Default: 70

    /// Enable transitive trust depth analysis
    pub check_transitive_depth: bool,  // Default: true

    /// Enable cycle detection
    pub check_cycles: bool,  // Default: true

    /// Enable quorum intersection validation
    pub check_intersection: bool,  // Default: true

    /// Maximum allowed transitive depth
    pub max_transitive_depth: u32,  // Default: 5

    /// Minimum quorum intersection ratio (0.0-1.0)
    pub min_intersection_ratio: f32,  // Default: 0.66
}
```

## Customizing Policies

### Preset Policies

#### Conservative (Enterprise)
For organizations requiring highest security:
```json
{
  "max_centralization_pct": 30,
  "check_transitive_depth": true,
  "check_cycles": true,
  "check_intersection": true,
  "max_transitive_depth": 3,
  "min_intersection_ratio": 0.75
}
```

#### Balanced (Recommended)
Good compromise between security and flexibility:
```json
{
  "max_centralization_pct": 50,
  "check_transitive_depth": true,
  "check_cycles": true,
  "check_intersection": true,
  "max_transitive_depth": 4,
  "min_intersection_ratio": 0.67
}
```

#### Permissive
For development or testing environments:
```json
{
  "max_centralization_pct": 70,
  "check_transitive_depth": true,
  "check_cycles": false,
  "check_intersection": true,
  "max_transitive_depth": 5,
  "min_intersection_ratio": 0.60
}
```

### Creating Custom Policies

1. **Determine Acceptable Centralization**
   - What percentage of consensus power is acceptable for a single organization?
   - Consider regulatory requirements and trust assumptions
   - Typical range: 30% (strict) to 70% (permissive)

2. **Set Transitive Depth Limits**
   - How many levels of nested quorum sets can validators understand?
   - Deeper nesting increases complexity and risk
   - Recommended max: 3-5 levels

3. **Define Intersection Requirements**
   - What minimum overlap is required for consensus safety?
   - Default 66% ensures Byzantine fault tolerance
   - Only adjust if you have formal verification

4. **Apply Custom Policy**
   ```rust
   let policy = ValidationPolicy {
       max_centralization_pct: 40,
       check_transitive_depth: true,
       check_cycles: true,
       check_intersection: true,
       max_transitive_depth: 4,
       min_intersection_ratio: 0.70,
   };
   
   let validator = QuorumValidator::new(policy);
   let result = validator.validate(&quorum_config);
   ```

## Validation Result Structure

```rust
pub struct ValidationResult {
    /// Whether the quorum configuration is valid
    pub is_valid: bool,

    /// List of validation errors (may contain multiple issues)
    pub errors: Vec<ValidationError>,

    /// Warnings (configuration is valid but has risk factors)
    pub warnings: Vec<String>,

    /// Execution time in milliseconds
    pub execution_time_ms: u64,

    /// Metadata about the validation
    pub validation_metadata: ValidationMetadata,
}

pub struct ValidationMetadata {
    /// Total validators in the quorum
    pub total_validators: usize,

    /// Number of unique organizations detected
    pub unique_organizations: usize,

    /// Maximum validator dependency level
    pub max_dependency_level: u32,

    /// Estimated quorum robustness (0-100)
    pub robustness_score: u32,
}
```

## Error Types

### InvalidThreshold
Threshold is invalid for the given validator set.
```rust
ValidationError::InvalidThreshold {
    threshold: 5,
    validator_count: 3,
}
```

### CentralizationRisk
An organization has too much consensus power.
```rust
ValidationError::CentralizationRisk {
    organization: "CentralBank".to_string(),
    percentage: 75,
    max_allowed: 70,
}
```

### SingleValidatorDependency
A single validator can achieve consensus.
```rust
ValidationError::SingleValidatorDependency {
    validator_id: "v1".to_string(),
}
```

### ExcessiveTransitiveDepth
Quorum nesting is too deep.
```rust
ValidationError::ExcessiveTransitiveDepth {
    current_depth: 6,
    max_allowed: 5,
}
```

### CyclicDependency
Circular trust relationship detected.
```rust
ValidationError::CyclicDependency {
    validators_in_cycle: vec!["v1", "v2", "v3"],
}
```

### InsufficientIntersection
Quorum sets don't have enough overlap for safety.
```rust
ValidationError::InsufficientIntersection {
    ratio: 0.50,
    minimum_required: 0.66,
}
```

### EmptyQuorum
No validators or inner sets provided.
```rust
ValidationError::EmptyQuorum
```

### InnerSetValidationFailed
A nested quorum set is invalid.
```rust
ValidationError::InnerSetValidationFailed {
    set_index: 0,
    reason: Box::new(/* nested error */),
}
```

## Usage Examples

### Basic Validation

```rust
use stellar_wasm_cache::{QuorumSetConfig, QuorumValidator, ValidationPolicy};

let quorum = QuorumSetConfig {
    threshold: 2,
    validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
    inner_sets: Vec::new(),
    validator_orgs: None,
};

let validator = QuorumValidator::new(ValidationPolicy::default());
let result = validator.validate(&quorum);

if result.is_valid {
    println!("✓ Quorum is valid");
    println!("  Robustness Score: {}", result.validation_metadata.robustness_score);
} else {
    for error in &result.errors {
        eprintln!("✗ {}", error);
    }
}
```

### With Organization Tracking

```rust
use std::collections::HashMap;

let mut org_map = HashMap::new();
org_map.insert("v1".to_string(), "OrgA".to_string());
org_map.insert("v2".to_string(), "OrgB".to_string());
org_map.insert("v3".to_string(), "OrgC".to_string());

let quorum = QuorumSetConfig {
    threshold: 2,
    validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
    inner_sets: Vec::new(),
    validator_orgs: Some(org_map),
};

let validator = QuorumValidator::new(ValidationPolicy::default());
let result = validator.validate(&quorum);

println!("Unique Organizations: {}", 
    result.validation_metadata.unique_organizations);
```

### Custom Policy for Enterprise

```rust
let enterprise_policy = ValidationPolicy {
    max_centralization_pct: 30,
    check_transitive_depth: true,
    check_cycles: true,
    check_intersection: true,
    max_transitive_depth: 3,
    min_intersection_ratio: 0.75,
};

let validator = QuorumValidator::new(enterprise_policy);
let result = validator.validate(&quorum);
```

### Nested Quorum Sets

```rust
let inner_set = QuorumSetConfig {
    threshold: 1,
    validators: vec!["v4".to_string(), "v5".to_string()],
    inner_sets: Vec::new(),
    validator_orgs: None,
};

let quorum = QuorumSetConfig {
    threshold: 2,
    validators: vec!["v1".to_string(), "v2".to_string(), "v3".to_string()],
    inner_sets: vec![inner_set],
    validator_orgs: None,
};

let validator = QuorumValidator::new(ValidationPolicy::default());
let result = validator.validate(&quorum);

println!("Max Dependency Level: {}", 
    result.validation_metadata.max_dependency_level);
```

## Performance Characteristics

All validation operations complete within the 15ms reconciliation budget:

- **Simple quorums** (< 10 validators): 1-3ms
- **Complex nested structures** (> 20 validators, 3+ levels): 5-12ms
- **Worst case scenario**: < 15ms guaranteed

This ensures the controller's reconciliation loop is never blocked by validation.

## Testing

The validation engine includes comprehensive test coverage:

- **Unit tests**: Core validation logic for each rule
- **Integration tests**: Real-world fragile quorum configurations
- **Performance tests**: Ensure < 15ms execution time
- **Coverage**: > 90% code coverage

Run tests:
```bash
cargo test -p stellar-wasm-cache
```

## Integration with Controller

The validation engine integrates with the Kubernetes operator controller:

1. Controller detects StellarNode resource creation/update
2. Extracts quorum configuration from node manifest
3. Runs policy validation via Wasm ABI
4. Rejects nodes with invalid configurations
5. Updates node status with validation results

Example controller integration:

```rust
// In controller reconciliation loop
let validation_result = validate_quorum_config(&node.spec.quorum_set)?;

if !validation_result.is_valid {
    return Err(ReconciliationError::InvalidQuorumConfig {
        errors: validation_result.errors,
    });
}

// Proceed with node deployment
```

## Security Considerations

1. **Deterministic Validation**: Same config always produces same result
2. **Bounded Computation**: < 15ms prevents denial-of-service via reconciliation loop
3. **Sandboxed Execution**: Wasm runtime prevents arbitrary code execution
4. **Policy Transparency**: All validation rules are explicit and configurable
5. **Audit Trail**: Validation metadata enables investigation of policy violations

## Best Practices

1. **Start Conservative**: Use enterprise policy initially, relax over time
2. **Monitor Rejections**: Track which configurations fail validation
3. **Regular Audits**: Review active quorum configurations quarterly
4. **Documentation**: Document your organization's custom policies
5. **Testing**: Test new policies in dev environment first
6. **Version Control**: Track policy changes in git

## Troubleshooting

### Quorum Rejected for Centralization
- **Cause**: Single organization has too much power
- **Solution**: Add validators from different organizations or reduce threshold

### Excessive Transitive Depth Error
- **Cause**: Quorum sets are nested too deeply
- **Solution**: Flatten inner set structures or increase max_transitive_depth policy

### Invalid Threshold Error
- **Cause**: Threshold doesn't match validator count
- **Solution**: Ensure threshold ≤ validator count and > 0

### Performance Issues
- **Cause**: Very large quorum sets (> 50 validators)
- **Solution**: Limit validators per quorum or use inner sets for organization

## References

- [Stellar Consensus Protocol (SCP)](https://stellar.org/papers/stellar-consensus-protocol.pdf)
- [Quorum Optimization](quorum-optimization.md)
- [Byzantine Fault Tolerance](https://en.wikipedia.org/wiki/Byzantine_fault)
- [Threshold Cryptography](https://en.wikipedia.org/wiki/Threshold_cryptography)
