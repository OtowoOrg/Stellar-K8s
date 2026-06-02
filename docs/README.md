# Stellar-K8s Documentation

This directory contains the source files for the Stellar-K8s documentation portal.

## Documentation Structure

- **getting-started/** - Installation guides and quick start tutorials
- **deployment-guides/** - Comprehensive deployment guides for validators, Horizon, and Soroban RPC
- **configuration/** - Configuration references and CRD specifications
- **tutorials/** - Step-by-step tutorials for common tasks
- **troubleshooting/** - Solutions to common problems
- **api-reference/** - Auto-generated API documentation
- **contributing/** - Guidelines for contributors

## Building Documentation

### Prerequisites

- Python 3.9+
- pip

### Install Dependencies

```bash
pip install -r requirements.txt
```

### Local Development

```bash
mkdocs serve
```

Visit http://127.0.0.1:8000

### Build Static Site

```bash
mkdocs build
```

## Contributing

See [Development Setup](contributing/development-setup.md) for detailed instructions on contributing to the documentation.

## Live Documentation

The documentation is automatically deployed to GitHub Pages when changes are merged to the main branch.

Visit: https://m1s0g1.github.io/Stellar-K8s/
