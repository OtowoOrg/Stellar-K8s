# Admission Webhook API

The Stellar-K8s admission webhook validates and mutates `StellarNode` CRD objects before they are accepted by the API server.

## Request/Response Flow

1. Kubernetes API Server sends an `AdmissionReview` request to the webhook.
2. The webhook validates the object, evaluates policies, and optionally mutates the request.
3. The webhook responds with an `AdmissionReview` response indicating allowed/denied status.

## Typical Validation Rules

- Required `StellarNode` fields are present.
- Network configuration, storage, and resource limits are consistent.
- Pod templates do not request unsupported host networking or forbidden capabilities.
- Sidecar and service mesh annotations are only allowed in supported contexts.

## Example AdmissionReview Response

```json
{
  "response": {
    "uid": "1234-abcd",
    "allowed": true,
    "status": {
      "code": 200,
      "message": "Validation passed"
    }
  }
}
```

## Error Handling

If validation fails, the webhook returns `allowed: false` with a descriptive status message.

- `400` — invalid request payload
- `422` — schema validation error
- `500` — internal validation failure

## Notes

The webhook is configured as a Kubernetes `ValidatingWebhookConfiguration` and may be extended for policy enforcement, admission transformations, and custom admission controls.
