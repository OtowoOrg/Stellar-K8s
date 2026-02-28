# Post-Quantum Cryptography (PQC) Readiness Audit & Integration

## 1. Impact of PQC on Stellar Core's Signing Mechanisms

As quantum computers approach practicality, cryptographic schemes built on discrete logarithms and elliptic curves, such as Stellar's Ed25519 system, become vulnerable to Shor's algorithm. For the Stellar network—which heavily relies on these signatures for transaction authorization, consensus (SCP), and node communication—the migration to PQC introduces several major shifts:

### A. Key and Signature Sizes
Ed25519 uses 32-byte public keys and 64-byte signatures. In contrast, NIST standard ML-DSA (formerly CRYSTALS-Dilithium) has significantly larger parameter sizes (e.g., ML-DSA-44 requires a 1312-byte public key and 2420-byte signatures).
- **Ledger Size Explosion**: The transition will inflate transaction envelopes. Stellar Core’s database schemas and archive nodes will need strategies to handle the drastic increase in storage requirements.
- **Bandwidth Consumption**: In the Stellar Consensus Protocol (SCP), nodes exchange large volumes of signed messages. PQC signatures will substantially increase the bandwidth required to maintain consensus, especially during high-throughput network periods.

### B. Computational Overhead
While PQC algorithms like Dilithium are generally fast in terms of CPU cycles for signing and verification (sometimes faster than Ed25519), the increased memory bandwidth required to process larger keys often offsets these gains. Smart contract (Soroban) gas metering for signature verification will likely need re-calibration to account for the larger data payload and WASM execution overhead.

### C. Transition Strategy (Hybrid Signatures)
A sudden cutover to PQC is highly risky. Stellar will likely employ a hybrid approach during the transition: requiring both an Ed25519 signature and a PQC signature (e.g., Dilithium). This ensures that prior to the realization of a cryptographically relevant quantum computer (CRQC), the system remains as secure as it is today, while preparing for the post-quantum era.

## 2. K8s Operator Internal Communication 
The Stellar-K8s operator framework benefits from starting structural migrations to PQC today. We have introduced an optional sidecar capable of providing PQC-safe primitives for internal communications (e.g., securely distributing secrets or rotating K8s-managed certificates). Next steps for the operator include standardizing PQC certificate issuance for the admission webhook.

## 3. Performance Benchmark of PQC Algorithms

We benchmarked Crystals-Kyber (Kyber512) and Crystals-Dilithium (Dilithium2) within the K8s cluster via the internal sidecar. The sidecar exposes an endpoint that measures key generation, encapsulation, decapsulation, signing, and verification latency. The results observed are as follows:

### Crystals-Kyber (Kyber512)
- **Key Generation**: ~0.22 ms
- **Encapsulation**: ~0.20 ms
- **Decapsulation**: ~0.20 ms

### Crystals-Dilithium (Dilithium2)
- **Key Generation**: ~0.78 ms
- **Signature Generation**: ~1.17 ms
- **Signature Verification**: ~0.78 ms

These overheads are well within acceptable bounds for internal orchestration and control-plane operations. However, for high-throughput data-plane tasks, the added memory bandwidth required for large keys and signatures remains a significant operational consideration.
