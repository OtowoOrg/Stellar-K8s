# Stellar-K8s developer resource templates
#
# Use `_base-stellarnode.yaml` as the shared baseline, then apply a workflow overlay:
#
#   kubectl apply -f dev/templates/_base-stellarnode.yaml
#   kubectl apply -f dev/templates/horizon-dev.yaml
#
# Overlays only contain fields that differ from the base (name, nodeType, etc.).
