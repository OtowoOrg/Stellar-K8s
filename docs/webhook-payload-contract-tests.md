# Webhook Payload Contract Tests

Introduced for [issue #1152](https://github.com/OtowoOrg/Stellar-K8s/issues/1152).

Hermetic HTTP contract tests for the admission webhook's `/validate` and
`/mutate` endpoints. These complement the CRD-focused suite in
[`admission-webhook-conformance-tests.md`](admission-webhook-conformance-tests.md)
by exercising **malformed and boundary wire payloads** through the real Axum
router (`WebhookServer::into_router`).

## Running

```bash
cargo test --test webhook_payload_contract
cargo test --test webhook_payload_contract -- --nocapture
```

## Coverage

### Malformed HTTP bodies

| Test | Expectation |
|------|-------------|
| `contract_validate_empty_body_is_rejected` | 4xx |
| `contract_mutate_empty_body_is_rejected` | 4xx |
| `contract_validate_non_json_body_is_rejected` | 4xx |
| `contract_validate_truncated_json_is_rejected` | 4xx |
| `contract_validate_wrong_content_type_plain_text_is_rejected` | 4xx |
| `contract_validate_array_body_is_rejected` | 4xx |

### Boundary / malformed AdmissionReview shapes

| Test | Expectation |
|------|-------------|
| `contract_validate_empty_object_review_is_bad_request` | 4xx |
| `contract_validate_missing_request_is_bad_request` | 4xx (+ invalid messaging when 400) |
| `contract_validate_null_request_is_bad_request` | 4xx |
| `contract_mutate_missing_request_is_bad_request` | 4xx |
| `contract_validate_wrong_kind_payload_is_client_error_or_denied` | 4xx or `allowed:false` |

### Boundary sizes

| Test | Expectation |
|------|-------------|
| `contract_validate_oversized_body_is_rejected` | not a successful silent admission |
| `contract_validate_boundary_empty_object_field_is_handled` | deny or 4xx (no panic) |
| `contract_validate_boundary_long_name_is_handled` | no panic |

### Smoke

| Test | Expectation |
|------|-------------|
| `contract_validate_valid_review_returns_admission_response` | 200 + AdmissionReview |
| `contract_mutate_valid_review_returns_ok` | 200 |
| `contract_health_is_ok` | 200 |

## Related files

| File | Purpose |
|------|---------|
| `tests/webhook_payload_contract.rs` | This suite |
| `src/webhook/server.rs` | `into_router`, HTTP handlers |
| `tests/admission_webhook_conformance.rs` | CRD validation conformance |
