🟡 Difficulty: Medium (100 Points)
Improve cluster security by automatically generating NetworkPolicies that restrict traffic only to necessary ports (e.g., SCP port, DB port).

✅ Acceptance Criteria
Operator should create default-deny policies.
Add allow-rules only for specific stellar-core and horizon communication.
Provide a way to disable this via CRD flag.
...................................................
🟡 Difficulty: Medium (100 Points)
Increase test coverage by adding specific e2e tests that simulate node crashes, disk failures, and network partitions.

✅ Acceptance Criteria
Use kind or minikube for local cluster setup.
Simulate failures and verify the operator auto-recovers the nodes.
Ensure tests pass reliably in CI.
 Acceptance Criteria
Add volumes field to StellarNode spec
Add volumeMounts field to StellarNode spec
Support ConfigMap and Secret volume sources
Support projected volumes for combining sources
Add validation for volume name conflicts
Add examples for common volume mount scenarios
Add unit tests for volume mounting logic
Document volume mount best practices

........................................................
Acceptance Criteria
Add serviceAnnotations field to StellarNode spec
Add serviceLabels field to StellarNode spec
Apply annotations to all generated Services
Support annotation templates with variable substitution
Preserve operator-managed annotations
Add examples for AWS/GCP/Azure load balancer annotations
Add unit tests for annotation merging logic
Document common annotation patterns

