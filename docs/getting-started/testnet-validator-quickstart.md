# Deploy a Testnet Validator (One-Page Guide)

Get a Stellar Testnet validator running on Kubernetes with the Stellar-K8s operator. For the full checklist, see [Testnet Onboarding](testnet-onboarding.md).

## Prerequisites

- Kubernetes 1.28+ with at least 4 CPU and 8 GB RAM free
- `kubectl` authenticated to the cluster
- Stellar-K8s operator installed ([Installation](installation.md))
- Outbound TCP `3510` and `11625` open to Testnet peers ([egress ports](../networking/testnet-egress-ports.md))

## 1. Generate a validator seed

Create a keypair with `stellar-core gen-seed` or `stellar keys generate`. Keep the `S...` secret seed private; never commit it.

## 2. Store the seed in a Kubernetes Secret

The operator reads the seed from the `STELLAR_CORE_SEED` key:

```bash
kubectl create namespace stellar
kubectl create secret generic validator-seed-testnet \
  --from-literal=STELLAR_CORE_SEED='S...' \
  -n stellar
```

## 3. Deploy the validator

Use the canonical Testnet example, [`examples/validator-testnet.yaml`](https://github.com/OtowoOrg/Stellar-K8s/blob/main/examples/validator-testnet.yaml), as-is for a first boot:

```bash
kubectl apply -f https://raw.githubusercontent.com/OtowoOrg/Stellar-K8s/main/examples/validator-testnet.yaml
```

Setting `network: testnet` selects the SDF Test Network passphrase (`Test SDF Network ; September 2015`) and the default history archives. Do not hardcode the passphrase; see [Testnet Onboarding](testnet-onboarding.md#what-is-testnet) and [Network Isolation](../network-isolation.md) for why mixing networks is dangerous.

## 4. Verify

```bash
kubectl get stellarnode validator-testnet -n stellar
kubectl get pods -n stellar -l app.kubernetes.io/instance=validator-testnet
kubectl logs -n stellar <pod> -f
```

The pod should reach `Running` within 1–2 minutes. Check peers once it is up:

```bash
kubectl exec -n stellar <pod> -- curl -s http://localhost:11626/peers | jq '.authenticated_peers | length'
```

Expect at least one peer within 2–5 minutes. If the count is `0`, follow the [zero-peers checklist](../networking/testnet-egress-ports.md#8-symptom-checklist-zero-peers--joining-scp).

## Next steps

- [Testnet Onboarding](testnet-onboarding.md): full checklist and sync validation
- [Deploy a Testnet Validator tutorial](../tutorials/deploy-testnet-validator.md)
- [Troubleshooting FAQ](../faq.md)
