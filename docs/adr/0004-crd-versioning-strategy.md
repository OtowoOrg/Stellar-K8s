# ADR-0004: CRD Versioning Strategy

## Status
Accepted

## Context
Kubernetes Custom Resource Definitions (CRDs) require careful versioning to ensure:

1. **Backward Compatibility**: Existing StellarNodes continue to work after upgrades
2. **Forward Compatibility**: New features can be added without breaking existing clients
3. **Migration Support**: Smooth transitions between schema versions
4. **Storage Consistency**: Persistent storage format remains stable
5. **API Evolution**: Clear path for API changes and deprecations

The StellarNode CRD manages complex infrastructure configurations including:
- Node types (Validator, Horizon, SorobanRPC)
- Storage configurations
- Network settings
- Monitoring and alerting
- Autoscaling policies
- Canary deployment strategies

Poor versioning could lead to:
- Broken existing deployments during operator upgrades
- Data loss during schema migrations
- Inability to roll back to previous versions
- Complex upgrade procedures for users

## Decision
We adopted a **semantic versioning strategy with multiple API versions** for the StellarNode CRD, following Kubernetes best practices.

### Version Structure

#### Storage Version
- **v1**: Primary storage version in etcd
- Stable, backward-compatible schema
- Used for long-term persistence

#### Served Versions
- **v1**: Current stable API version
- **v1beta1**: Beta features (when needed)
- **v1alpha1**: Experimental features (when needed)

#### Version Lifecycle
```
v1alpha1 → v1beta1 → v1 (stable) → v2 (major version bump)
```

### Schema Evolution Strategy

#### Additive Changes (Preferred)
```yaml
# Adding new optional field
spec:
  existingField: string
  newOptionalField: string  # New field, optional
```

#### Backward-Compatible Changes
```yaml
# Adding new enum values
spec:
  nodeType: Validator | Horizon | SorobanRPC | NewType
```

#### Major Version Changes
- Require migration path
- Support both versions during transition
- Provide conversion webhooks if needed
- Clear deprecation timeline

### Implementation Details

#### CRD Definition Structure
```yaml
apiVersion: apiextensions.k8s.io/v1
kind: CustomResourceDefinition
metadata:
  name: stellarnodes.stellar.k8s.io
spec:
  group: stellar.k8s.io
  versions:
  - name: v1
    served: true
    storage: true
    schema:
      openAPIV3Schema: {...}
  - name: v1beta1
    served: true
    storage: false
    deprecated: true
    deprecationWarning: "stellar.k8s.io/v1beta1 StellarNode is deprecated"
```

#### Conversion Strategy
- **Same-version conversion**: Direct mapping
- **Cross-version conversion**: Webhook-based conversion when needed
- **Default values**: Sensible defaults for new fields
- **Validation**: Strict validation for all versions

#### Migration Support
```rust
// Webhook conversion handler
async fn convert(
    req: ConversionReview,
) -> Result<ConversionReview, ConversionError> {
    match req.request.desiredVersion {
        "v1" => convert_to_v1(req.request.objects),
        "v1beta1" => convert_to_v1beta1(req.request.objects),
        _ => Err(ConversionError::UnsupportedVersion),
    }
}
```

## Consequences

### Positive Consequences
- **Upgrade Safety**: Existing deployments remain functional during upgrades
- **Feature Development**: New features can be added in beta versions first
- **API Stability**: Clear contract for users and integrators
- **Rollback Support**: Can revert to previous operator versions
- **Multi-version Support**: Different clusters can use different versions

### Negative Consequences
- **Complexity**: Additional code for version handling and conversion
- **Testing Overhead**: Need to test all supported versions
- **Documentation**: Must maintain documentation for multiple versions
- **Maintenance**: Older versions require continued support

### Mitigations
- **Automated Testing**: Comprehensive test suite for all versions
- **Conversion Webhooks**: Automated conversion between versions
- **Deprecation Policy**: Clear timeline for version deprecation
- **Documentation**: Version-specific documentation and migration guides

## Version Management Process

### Adding New Features
1. **Alpha Stage**: Add as optional field in v1alpha1
2. **Beta Stage**: Promote to v1beta1 after testing
3. **Stable**: Add to v1 after production validation
4. **Default Values**: Provide sensible defaults for new fields

### Deprecation Process
1. **Announce**: Communicate upcoming deprecation
2. **Mark Deprecated**: Add deprecation warning to CRD
3. **Migration Path**: Provide clear migration instructions
4. **Remove Support**: Remove after deprecation period (minimum 6 months)

### Validation Rules
- **Strict Validation**: Reject invalid configurations at admission
- **Version-Specific Rules**: Different validation rules per version
- **Default Enforcement**: Ensure required fields have defaults
- **Cross-Field Validation**: Validate field dependencies

## Implementation Examples

### Field Addition Strategy
```rust
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StellarNodeSpec {
    pub node_type: NodeType,
    pub version: String,
    // New optional field with default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_feature: Option<NewFeatureConfig>,
}
```

### Conversion Logic
```rust
impl From<StellarNodeV1Beta1> for StellarNodeV1 {
    fn from(old: StellarNodeV1Beta1) -> Self {
        StellarNodeV1 {
            // Direct field mapping
            node_type: old.node_type,
            version: old.version,
            // New field with default
            new_feature: old.new_feature.or_else(|| Some(Default::default())),
        }
    }
}
```

## Alternatives Considered

### Single Version Strategy
**Pros**: Simpler implementation, less testing overhead
**Cons**:
- Breaking changes for every modification
- No beta testing for new features
- Difficult to maintain backward compatibility

### External Versioning
**Pros**: Separate versioning from Kubernetes API
**Cons**:
- Not idiomatic Kubernetes
- Poor tooling support
- Confusing for users

### Webhook-Only Conversion
**Pros**: Maximum flexibility in conversions
**Cons**:
- Performance overhead
- Complex deployment
- Single point of failure

## Best Practices Implemented

1. **Semantic Versioning**: Follow semver for API changes
2. **Backward Compatibility**: Never break existing functionality
3. **Gradual Rollout**: Beta → Stable progression
4. **Clear Deprecation**: Ample warning before removal
5. **Comprehensive Testing**: Test all version combinations
6. **Documentation**: Clear migration paths and examples

## Future Considerations

- **CRD Conversion Webhooks**: Implement for complex schema changes
- **Version Metrics**: Track usage of different API versions
- **Automated Migration**: Tools for automatic resource migration
- **Multi-CRD Versioning**: Coordinate versions across multiple CRDs

## References

- [Kubernetes CRD Versioning](https://kubernetes.io/docs/tasks/access-kubernetes-api/custom-resources/custom-resource-definition-versioning/)
- [Kubernetes API Deprecation Policy](https://kubernetes.io/docs/reference/using-api/deprecation-policy/)
- [Semantic Versioning](https://semver.org/)
