# [EPIC] Node Image Integrity Verification at Boot

**Labels:** `epic`, `200-points`, `security`, `compliance`, `phase-3`

## Epic Overview

Verify node image integrity and expected software bill of materials at boot before the node joins the cluster. This epic ensures that only verified, tamper-proof node images are permitted to join the Stellar cluster, preventing compromised infrastructure from participating in consensus.

## Business Value

- **Attack prevention**: Block tampered node images from cluster
- **Compliance**: Meet security audit requirements for node provenance
- **Transparency**: Software bill of materials published and verifiable
- **Recovery**: Clear failure reasons enable incident response

## Scope & Requirements

### Core Requirements

1. **Image Integrity Verification**
   - Verify cryptographic signatures on node images
   - Check image hash matches expected value
   - Fail closed if verification fails (node cannot join)
   - Support multiple signature algorithms (RSA, ECDSA)

2. **Software Bill of Materials (SBOM)**
   - Generate SBOM during image build
   - Embed SBOM in node image metadata
   - Verify SBOM contents match running system
   - Support CycloneDX and SPDX formats

3. **Boot-Time Verification**
   - Implement as systemd unit running before kubelet
   - Complete verification in < 15 seconds
   - Set node conditions explaining any refusal
   - Publish verification results to node status

4. **Expected Image Management**
   - Central registry of approved image hashes and SBOMs
   - Versioned image specifications
   - Configurable per-node overrides
   - Audit trail of image approvals

5. **Node Conditions & Alerting**
   - `ImageVerified=True/False` node condition
   - `ImageTampered=True` when verification fails
   - Clear reason/message explaining refusal
   - Alert on tamper detection attempts

6. **Image Update Pipeline**
   - Seamless rollout of expected image changes
   - Canary verification (test on small % first)
   - Rollback capability for bad images
   - Historical tracking of image versions

### Non-Functional Requirements

- **Boot impact**: Add < 15 seconds to node startup
- **False positives**: < 0.1% (prevent legitimate nodes from joining)
- **Verification coverage**: 100% of nodes joining cluster
- **SBOM accuracy**: Complete list of all system packages
- **Signature algorithm support**: RSA-2048, RSA-4096, ECDSA-256, ECDSA-384

## Technical Design

### Architecture Components

```
┌─────────────────────────────────────────────────────────────┐
│              Node Image Verification Layer                   │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Verification Systemd Unit (runs before kubelet)         ││
│  │ ┌─────────────────────────────────────────────────────┐ ││
│  │ │ 1. Fetch expected image config from cluster         │ ││
│  │ │ 2. Calculate running image hash                     │ ││
│  │ │ 3. Verify crypto signature                          │ ││
│  │ │ 4. Parse and validate SBOM                          │ ││
│  │ │ 5. Set node conditions based on result              │ ││
│  │ └─────────────────────────────────────────────────────┘ ││
│  └─────────────────────────────────────────────────────────┘│
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Cluster-Wide Expected Image Registry (ConfigMap/CRD)   ││
│  │ ┌─────────────────────────────────────────────────────┐ ││
│  │ │ expectedImages:                                     │ ││
│  │ │   - hash: sha256:abc123...                          │ ││
│  │ │     sbomHash: sha256:def456...                      │ ││
│  │ │     signature: <base64>                            │ ││
│  │ │     publicKey: <PEM>                               │ ││
│  │ │     created: 2024-11-20                            │ ││
│  │ └─────────────────────────────────────────────────────┘ ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### Implementation Approach

1. **Expected Image CRD**
   ```yaml
   apiVersion: stellar.org/v1alpha1
   kind: ExpectedNodeImage
   metadata:
     name: node-image-v1.25.0
   spec:
     imageHash: sha256:abc123def456...
     sbomHash: sha256:def456abc123...
     signature: "<base64-encoded-signature>"
     signatureAlgorithm: "RSA-2048"
     publicKey: "<PEM-encoded-public-key>"
     sbom:
       format: "cyclonedx"
       version: "1.4"
       components:
         - name: "kernel"
           version: "5.15.0-1234"
           purl: "pkg:rpm/kernel@5.15.0-1234"
         - name: "kubelet"
           version: "1.25.0"
           purl: "pkg:deb/kubelet@1.25.0"
     approval:
       approvedBy: "security-team"
       timestamp: 2024-11-20T10:00:00Z
       justification: "Regular security patch release"
   status:
     active: true
     firstSeen: 2024-11-20T10:00:00Z
     nodeCount: 42
     verificationFailureCount: 0
   ```

2. **Verification Systemd Unit**
   ```ini
   # /etc/systemd/system/stellar-node-image-verify.service
   [Unit]
   Description=Verify Stellar Node Image Integrity
   Before=kubelet.service
   ConditionFileNotEmpty=/etc/stellar/image-info.json
   
   [Service]
   Type=oneshot
   ExecStart=/usr/local/bin/stellar-verify-image
   StandardOutput=journal
   StandardError=journal
   SyslogIdentifier=stellar-verify-image
   
   [Install]
   WantedBy=multi-user.target
   ```

3. **Verification Script Logic**
   ```bash
   # Pseudocode for /usr/local/bin/stellar-verify-image
   1. Fetch ExpectedNodeImage from cluster API
   2. Calculate SHA256 of running image (from /etc/stellar/image-info.json)
   3. Verify signature using public key in ExpectedNodeImage
   4. Extract SBOM from node image metadata
   5. Verify SBOM hash matches expected value
   6. Parse SBOM and validate package versions against policy
   7. If all checks pass:
      - Set `ImageVerified=True` node condition
      - Exit 0
   8. If any check fails:
      - Set `ImageVerified=False` node condition
      - Set `ImageTampered=True` if signature invalid
      - Set clear reason explaining failure
      - Exit 2 (fail node startup)
   ```

4. **Node Condition Updates**
   ```yaml
   # Node status after verification
   status:
     conditions:
       - type: ImageVerified
         status: "True"
         reason: "ImageIntegrityVerified"
         message: "Node image signature and SBOM verified successfully"
         lastHeartbeatTime: 2024-11-20T10:05:00Z
       - type: ImageTampered
         status: "False"
         reason: "NoTamperingDetected"
         message: ""
         lastHeartbeatTime: 2024-11-20T10:05:00Z
   ```

5. **Expected Image Update Pipeline**
   - Build process generates image with embedded metadata
   - Security team approves and signs new image
   - Create `ExpectedNodeImage` CRD in cluster
   - Optionally run canary on small % of nodes first
   - Monitor verification success rate
   - Trigger node rollouts to verify new image
   - Rollback by updating ExpectedNodeImage back to previous version

## Acceptance Criteria

- [ ] Systemd verification unit runs before kubelet with < 15s overhead
- [ ] Node image signature verification using RSA-2048 and ECDSA
- [ ] SBOM embedded in image and verified at boot
- [ ] `ExpectedNodeImage` CRD implementation with full validation
- [ ] Tampered node image prevented from joining in all test scenarios
- [ ] Node conditions clearly explain any verification failure
- [ ] Zero false positives in 100-node production test
- [ ] Boot time impact measured and < 15 seconds
- [ ] kubectl commands to view node image verification status
- [ ] Audit log of all node image verification attempts and results
- [ ] Automated alerting on tamper detection attempts
- [ ] Documentation for image signing and approval process
- [ ] E2E tests: verified image joins, tampered image rejected

## Dependencies & Blockers

- Requires node image to include verification metadata
- Depends on secure key management for signing keys
- Needs cluster API access from boot (may require early bootstrap)
- May require TPM/secure boot integration for maximum security

## Testing Strategy

### Unit Tests
- Image hash calculation and verification
- Signature verification algorithms
- SBOM parsing and validation
- Node condition generation logic

### Integration Tests
- Real systemd unit execution
- Cluster API bootstrap during verification
- Key distribution and management

### E2E Tests
- Successful verification with valid image
- Rejection with modified image
- Rejection with invalid signature
- Rejection with missing SBOM
- Multiple image versions in cluster
- Image update rollout and canary
- Node startup timing measurements
- Audit log verification

### Security Tests
- Attempt to modify image (verify detected)
- Attempt to forge signature (verify rejected)
- Attempt to bypass verification (verify fails)
- Replay attack scenarios
- Private key compromise scenarios

## Estimated Effort

**200 Story Points** (~4-6 weeks for 2 engineers)

## Related Issues

- #TBD: Secure key management and distribution
- #TBD: Image build pipeline integration
- #TBD: Node health dashboard with verification status

## References

- [Secure Boot and Measured Boot](https://en.wikipedia.org/wiki/Secure_boot)
- [CycloneDX SBOM Standard](https://cyclonedx.org/)
- [SPDX Software Bill of Materials](https://spdx.dev/)
- [Systemd Service Files](https://www.freedesktop.org/software/systemd/man/systemd.service.html)

