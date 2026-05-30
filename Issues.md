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