# Stellar-K8s Helm Lint Fix TODO

## Plan Steps
- [x] 1. Edit charts/stellar-operator/templates/prometheusrule.yaml (escape {{ $value }} literals)
- [ ] 2. Run \`helm lint charts/stellar-operator\` to verify fix
- [ ] 3. Run \`helm template charts/stellar-operator > rendered.yaml\` (optional validation)
- [ ] 4. Mark complete
