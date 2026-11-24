# Internal API schema registry

`schemas/registry.json` is the single source of truth for every internal JSON/protobuf contract. The build fails when an inventory entry is absent or has no version, and `InternalApiSchema` resources apply the same exact version pins at deploy time.

## Compatibility

Each subject selects `backward`, `forward`, `full`, or the existing opt-out `none` policy. JSON checks cover required fields, removed properties, nested objects, type changes, and enum removals. The protobuf text representation is checked for removed message, enum, field, or type declarations. Compatibility is evaluated against the latest approved version **and every registered consumer's pinned version**.

A breaking registration is rejected atomically. It can proceed only with an unconsumed `RegistryOverride` naming the subject, reason, requester, and approver. Consumption and registration are recorded in the registry audit trail. Overrides are one-shot and cannot authorize a different subject.

## Pull-request gate

```bash
stellar-operator schema-compat \
  --subject stellar.ledger.events \
  --schema path/to/candidate.json \
  --report target/schema-impact.json
```

The command exits non-zero for a breaking change without a valid override. Reports list the pinned version and compatibility issues for every consumer. It is offline and requires neither a cluster nor network access. The deterministic unit timing test evaluates 1,000 consumers and enforces the 30-second service-level ceiling.

## Client and deployment pins

Consumers must register an immutable `subject -> version` map and a pinned generated-client reference. Floating references such as `:latest` or `/main` are rejected. `InternalApiSchema.spec.schemaVersion` repeats the exact pin and lists the consumers gated by that deployment policy.
