# Configuration Files

This directory contains Kubernetes manifests and configuration files for the Stellar-K8s operator.

## Directory Structure

```
config/
├── crd/              # Custom Resource Definitions
│   └── stellarnode-crd.yaml
├── samples/          # Example resources for testing
│   ├── test-stellarnode.yaml
│   └── example_nodeport_config.yaml
└── dev/              # Development configuration (not for production)
    └── kubeconfig-dev.yaml
```

## Usage

### Install CRDs
```bash
kubectl apply -f config/crd/
```

### Apply Sample Resources
```bash
kubectl apply -f config/samples/
```

### Development
```bash
# Use development kubeconfig
export KUBECONFIG=config/dev/kubeconfig-dev.yaml
```

## Important Notes

- **CRD files**: Define the StellarNode custom resource schema
- **Sample files**: Example configurations for testing and reference
- **Dev files**: Local development configurations (add to .gitignore if contains secrets)
