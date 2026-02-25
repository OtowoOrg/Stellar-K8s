The src/controller/peer_discovery.rs module has no dedicated unit tests. Peer discovery is a critical path for validator nodes.

✅ Acceptance Criteria
Add a peer_discovery_test.rs file with unit tests covering:
Peer list building from StellarNode CRDs
DNS lookups for peer addresses (mock the DNS client)
The peer scoring/selection algorithm
Edge cases: empty peer list, all peers unreachable
Run cargo test and confirm all new tests pass.
📚 Resources
src/controller/peer_discovery.rs
