# [EPIC] Multi-Party Approval Workflow for Privileged Changes

**Labels:** `epic`, `200-points`, `security`, `governance`, `phase-3`

## Epic Overview

Require multi-party approval for privileged cluster changes with cryptographically verified approver identities and tamper-proof decision records. This epic ensures that critical infrastructure modifications undergo proper authorization and governance, with non-repudiable records of who approved what and when.

## Business Value

- **Governance**: Enforce policy-based approval requirements for risky changes
- **Accountability**: Non-repudiable signed records of approvals
- **Risk reduction**: Prevent unilateral privileged changes
- **Compliance**: Meet regulatory requirements for change management
- **Auditability**: Complete chain of custody for every approval decision

## Scope & Requirements

### Core Requirements

1. **Change Request CRD**
   - Define all privileged changes as `ChangeRequest` custom resources
   - Support multiple change types (RBAC, networking, core configs)
   - Track approval state and decision records
   - Immutable audit trail in CR history

2. **Multi-Party Approval Workflow**
   - Configurable approval quorum per change class (e.g., 2 of 3, 3 of 5)
   - Support role-based approval requirements
   - Expiration of pending requests if not approved
   - Auto-escalation for critical changes

3. **Admission Controller Integration**
   - Block unapproved privileged changes 100% of the time
   - Allow only changes with sufficient approvals
   - Validate approval signatures
   - Reject changes after expiration

4. **Cryptographic Verification**
   - Sign approvals with approver's private key
   - Verify signatures using approver's certificate
   - Use standard formats (JWS, X.509)
   - Support hardware security modules (HSM) integration

5. **Decision Records**
   - Each approval is signed and immutable
   - Records include: who, when, what, reason, signature
   - Aggregate approval states in CR status
   - Prevent modification of past decisions

6. **Expiration & Auto-Escalation**
   - ChangeRequests auto-expire if not approved within TTL
   - Critical changes auto-escalate to management
   - Configurable expiration per severity
   - Alert on about-to-expire requests

### Non-Functional Requirements

- **Approval coverage**: 100% of privileged changes require approval
- **False negatives**: 0% (must never allow unapproved changes)
- **Response time**: Approval submitted and verified within < 2 seconds
- **Availability**: Approval workflow remains available during cluster issues
- **Signature algorithm**: Support RSA-2048, ECDSA-256, Ed25519
- **Quorum flexibility**: Support configurations from 1 to 10 approvers

## Technical Design

### Architecture Components

```
┌─────────────────────────────────────────────────────────────┐
│              Multi-Party Approval System                     │
│  ┌──────────────────────────────────────────────────────────┐│
│  │ Approval Admission Controller                            ││
│  │ (intercepts all privileged operations)                  ││
│  └──────────────────────────────────────────────────────────┘│
│  ┌──────────────────────────────────────────────────────────┐│
│  │ ChangeRequest CR + Approval Signatures                   ││
│  │ (immutable decision records)                             ││
│  └──────────────────────────────────────────────────────────┘│
│  ┌──────────────────────────────────────────────────────────┐│
│  │ Approval Policy Engine                                   ││
│  │ (determines required quorum and approvers)               ││
│  └──────────────────────────────────────────────────────────┘│
│  ┌──────────────────────────────────────────────────────────┐│
│  │ Cryptographic Verification                              ││
│  │ (validates signatures using certs/keys)                 ││
│  └──────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### Implementation Approach

1. **ChangeRequest CRD**
   ```yaml
   apiVersion: stellar.org/v1alpha1
   kind: ChangeRequest
   metadata:
     name: privilege-escalate-john-2024-11-20
     namespace: stellar-system
   spec:
     changeType: "ClusterRole"
     changeClass: "critical"  # critical, high, medium, low
     requestor: "john@stellar.io"
     requestTime: 2024-11-20T10:00:00Z
     changes:
       - resource:
           apiVersion: rbac.authorization.k8s.io/v1
           kind: ClusterRole
           name: stellar-admin
         operation: update
         patch:
           rules:
             - apiGroups: ["*"]
               resources: ["*"]
               verbs: ["*"]
     description: |
       Grant full admin access to on-call engineer for emergency response
       to database corruption incident. Temporary for 4 hours.
     expiresAt: 2024-11-20T14:00:00Z
     
     # Approval policy for this change
     approvalPolicy:
       requiredApprovers: 2
       minSeniorApprovers: 1
       approverRoles:
         - role: security-lead
           count: 1
         - role: platform-lead
           count: 1
       ttl: 3600  # Expire if not approved in 1 hour
   
   status:
     state: "pending"  # pending, approved, rejected, expired
     approvals:
       - approver: "alice@stellar.io"
         timestamp: 2024-11-20T10:05:00Z
         reason: "Emergency justification verified. Context switch loss acceptable."
         signature: "<JWS-signature>"
         certificateHash: "sha256:abc123..."
       - approver: "bob@stellar.io"
         timestamp: 2024-11-20T10:08:00Z
         reason: "Approved per security policy. 4-hour limit enforced."
         signature: "<JWS-signature>"
         certificateHash: "sha256:def456..."
     approvalMetadata:
       requiredApprovers: 2
       approvalsReceived: 2
       seniorApprovalsReceived: 2
       allConditionsMet: true
     conditions:
       - type: ApprovalQuorumMet
         status: "True"
         reason: "2 of 2 required approvals received"
       - type: SignaturesValid
         status: "True"
         reason: "All signatures verified using valid certificates"
       - type: NotExpired
         status: "True"
         reason: "Request expires in 55 minutes"
   ```

2. **Approval Policy CRD**
   ```yaml
   apiVersion: stellar.org/v1alpha1
   kind: ApprovalPolicy
   metadata:
     name: default-approval-policy
   spec:
     changeClasses:
       - class: critical
         description: "Affects cluster security, availability, or data"
         requiredApprovals: 3
         requiredRoles:
           - security-lead
           - platform-lead
           - cto
         ttl: 1800  # 30 minutes
       - class: high
         description: "Significant operational impact"
         requiredApprovals: 2
         requiredRoles:
           - senior-engineer
           - team-lead
         ttl: 3600  # 1 hour
       - class: medium
         description: "Standard changes"
         requiredApprovals: 1
         requiredRoles:
           - engineer
         ttl: 7200  # 2 hours
       - class: low
         description: "Low-risk operational tasks"
         requiredApprovals: 0  # Auto-approved
         ttl: 0
     
     privilegedOperations:
       - apiGroup: rbac.authorization.k8s.io
         resource: clusterrolebindings
         operation: "*"
         changeClass: critical
       - apiGroup: rbac.authorization.k8s.io
         resource: clusterroles
         operation: "*"
         changeClass: critical
       - apiGroup: stellar.org
         resource: stellarnodes
         operation: delete
         changeClass: high
       - apiGroup: ""
         resource: secrets
         operation: "*"
         changeClass: critical
   ```

3. **Approval Admission Controller**
   - Webhook intercepts API requests
   - Checks if operation requires approval
   - Looks for matching approved ChangeRequest
   - Verifies all approvals and signatures
   - Rejects if no valid ChangeRequest exists
   - Returns clear error messages

4. **Cryptographic Signing**
   ```yaml
   # Approval signature in JWS format
   eyJhbGciOiJSUzI1NiIsImtpZCI6ImFsaWNlQGV4YW1wbGUuY29tIn0.
   eyJhcHByb3ZlciI6ImFsaWNlQGV4YW1wbGUuY29tIiwicmVhc29uIjoiI
   kdFc...
   <signature-bytes>
   
   # Approver certificate stored in Secret
   apiVersion: v1
   kind: Secret
   metadata:
     name: approver-alice-cert
     namespace: stellar-system
   type: kubernetes.io/tls
   data:
     tls.crt: <base64-encoded-X.509-cert>
     tls.key: <base64-encoded-private-key>
   ```

5. **Decision Record Immutability**
   - Use etcd to store ChangeRequest with immutable history
   - Prevent deletion of ChangeRequests (immutable retention)
   - Record all state transitions with timestamps
   - Audit webhook logs all approval events

6. **Approval Expiration & Escalation**
   - Background job checks ChangeRequests daily
   - Expire requests older than TTL
   - Alert on about-to-expire requests
   - Auto-escalate critical changes to management

## Acceptance Criteria

- [ ] `ChangeRequest` CRD implemented with full validation
- [ ] `ApprovalPolicy` CRD for defining approval requirements
- [ ] Admission controller blocks unapproved privileged changes 100% of time
- [ ] Cryptographic signature verification using JWS format
- [ ] All approvals signed and non-repudiable
- [ ] Approval records immutable (no modification or deletion)
- [ ] Quorum configurable per change class (1 to 10 approvers)
- [ ] TTL-based expiration with < 5 minute enforcement lag
- [ ] Auto-escalation of critical changes to management
- [ ] kubectl commands to submit, approve, and view ChangeRequests
- [ ] Audit log of all approval decisions
- [ ] Grafana dashboard showing approval metrics
- [ ] E2E tests: approved changes allowed, unapproved blocked, signatures verified
- [ ] Documentation on approval workflow and policy configuration

## Dependencies & Blockers

- Requires admission webhook infrastructure (ValidatingWebhookConfiguration)
- Depends on certificate management for approver identities
- Needs role-based access control (RBAC) for approver authorization
- May require HSM integration for high-security environments

## Testing Strategy

### Unit Tests
- Quorum calculation logic
- JWS signature generation and verification
- Policy matching for change types

### Integration Tests
- Admission controller webhook behavior
- ChangeRequest state transitions
- TTL-based expiration
- Certificate validation

### E2E Tests
- Submit ChangeRequest for privileged operation
- Approve with single approver (verify decision record)
- Approve with quorum (verify all signatures)
- Reject with insufficient approvals
- Reject with invalid signatures
- Reject after TTL expiration
- Verify unapproved operations blocked at admission
- Verify approved operations allowed through
- Tamper with approval signature (verify detected)
- Auto-escalation of critical changes
- Audit log completeness

### Security Tests
- Attempt to forge approval signature
- Attempt to bypass admission controller
- Replay attack on old approvals
- Approver certificate revocation
- Private key compromise scenario

## Estimated Effort

**200 Story Points** (~4-6 weeks for 2 engineers)

## Related Issues

- #TBD: Certificate management and rotation
- #TBD: Approval audit dashboard
- #TBD: Integration with identity provider (OIDC/SAML)

## References

- [JSON Web Signature (JWS) - RFC 7515](https://tools.ietf.org/html/rfc7515)
- [X.509 Digital Certificates](https://en.wikipedia.org/wiki/X.509)
- [Kubernetes Admission Controllers](https://kubernetes.io/docs/reference/access-authn-authz/admission-controllers/)
- [Non-repudiation](https://en.wikipedia.org/wiki/Non-repudiation)

