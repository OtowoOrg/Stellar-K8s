# Operator Diagnostic Runbooks

> Command-centric runbooks for incident responders. Each section includes expected vs abnormal output characteristics.

---

## 1) Reconciliation Loop Diagnostics

### Commands

```bash
kubectl -n stellar-system get pods -l app.kubernetes.io/name=stellar-operator
kubectl -n stellar-system logs deploy/stellar-operator --since=20m | tail -n 200
kubectl get stellarnodes.stellar.org -A -o yaml | grep -E 'generation|observedGeneration|conditions' -n
```

### Expected Output Shape

- Operator logs show bounded reconcile retries with increasing backoff.
- `observedGeneration` converges to `metadata.generation`.
- Conditions move to stable `Ready=True` when dependencies are healthy.

### Abnormal Output Shape

- Same object key appears every few milliseconds with unchanged condition payload.
- Repeated webhook timeout or conflict errors without backoff growth.

---

## 2) Runtime Container Diagnostics (crictl)

```bash
crictl ps -a | grep stellar
crictl logs <container-id> | tail -n 120
crictl inspect <container-id> | egrep 'runtimeType|mounts|privileged|seccomp' -n
```

Expected:

- Image digest pinned and matches approved release.
- Seccomp profile and capabilities align with policy baseline.

---

## 3) Node-Level Debugging (kubectl-debug)

```bash
kubectl debug node/<node-name> -it --image=nicolaka/netshoot -- chroot /host
ip route
ss -antp | head -n 100
ethtool -S eth0 | egrep 'err|drop|miss'
exit
```

Expected:

- Stable routes to Pod CIDR and service VIPs.
- No sustained growth in NIC drop/error counters.

---

## 4) Plugin-Assisted Checks (kubectl stellar)

```bash
kubectl stellar status
kubectl stellar diagnose validator <name> -n stellar-validator
kubectl stellar quorum check -n stellar-validator
```

Expected structure:

- `status`: cluster health summary by component.
- `diagnose`: peer count, ledger age, sync status, quorum membership.
- `quorum check`: configured slices, missing peers, and risk flags.

---

## 5) Secret Access Failure Runbook

```bash
kubectl -n stellar-validator describe pod <pod-name> | egrep -n 'MountVolume|Forbidden|secret'
kubectl auth can-i get secret/<secret-name> -n stellar-validator --as system:serviceaccount:stellar-validator:validator
kubectl -n stellar-validator get role,rolebinding,serviceaccount | grep validator
```

Expected:

- Event stream identifies explicit RBAC denial or missing object.
- `can-i` aligns with intended least-privilege model.
