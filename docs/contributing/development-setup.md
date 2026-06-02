# Development Setup

Learn how to set up a local development environment for building and testing Stellar-K8s documentation.

## Prerequisites

- Python 3.9 or later
- pip (Python package manager)
- Git

## Installation

### 1. Clone the Repository

```bash
git clone https://github.com/OtowoOrg/Stellar-K8s.git
cd Stellar-K8s
```

### 2. Install Dependencies

```bash
pip install -r requirements.txt
```

### 3. Verify Installation

```bash
mkdocs --version
```

## Local Development Server

### Start the Development Server

```bash
mkdocs serve
```

The documentation will be available at http://127.0.0.1:8000

The server includes:
- Live reload - changes are reflected immediately
- Search functionality
- All theme features enabled

### Custom Port

```bash
mkdocs serve --dev-addr=127.0.0.1:8080
```

## Building Documentation

### Build Static Site

```bash
mkdocs build
```

Output will be in the `site/` directory.

### Clean Build

```bash
mkdocs build --clean
```

### Strict Mode

Build with strict mode to catch warnings:

```bash
mkdocs build --strict
```

## Project Structure

```
Stellar-K8s/
├── docs/                       # Documentation source files
│   ├── index.md               # Homepage
│   ├── getting-started/       # Getting started guides
│   ├── deployment-guides/     # Deployment documentation
│   ├── configuration/         # Configuration references
│   ├── tutorials/             # Step-by-step tutorials
│   ├── troubleshooting/       # Troubleshooting guides
│   ├── api-reference/         # API documentation
│   └── contributing/          # Contributing guides
├── mkdocs.yml                 # MkDocs configuration
├── requirements.txt           # Python dependencies
└── site/                      # Built documentation (generated)
```

## Writing Documentation

### Create New Page

1. Create a new Markdown file in the appropriate directory:
   ```bash
   touch docs/tutorials/my-new-tutorial.md
   ```

2. Add frontmatter (optional):
   ```markdown
   ---
   title: My New Tutorial
   description: A comprehensive tutorial for doing something
   ---
   ```

3. Write content using Markdown

4. Add to navigation in `mkdocs.yml`:
   ```yaml
   nav:
     - Tutorials:
       - My New Tutorial: tutorials/my-new-tutorial.md
   ```

### Preview Changes

```bash
mkdocs serve
```

Navigate to your new page to preview.

## Code Examples

### Add Syntax Highlighting

Use triple backticks with language identifier:

````markdown
```yaml
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarValidator
```
````

### Add Filename

Use title attribute:

````markdown
```yaml title="validator.yaml"
apiVersion: stellar.k8s.io/v1alpha1
kind: StellarValidator
```
````

### Add Line Numbers

````markdown
```python linenums="1"
def hello_world():
    print("Hello, World!")
```
````

## Using Admonitions

### Info Box

```markdown
!!! info "Information"
    This is an informational message.
```

### Warning Box

```markdown
!!! warning "Warning"
    This is a warning message.
```

### Tip Box

```markdown
!!! tip "Pro Tip"
    This is a helpful tip.
```

### Collapsible Admonition

```markdown
??? note "Click to expand"
    This content is collapsible.
```

## Testing Documentation

### Check for Broken Links

```bash
mkdocs build --strict
```

### Validate Markdown

```bash
# Install markdownlint
npm install -g markdownlint-cli

# Run lint
markdownlint docs/
```

### Spell Check

```bash
# Install aspell
sudo apt-get install aspell

# Run spell check
find docs/ -name "*.md" -exec aspell check {} \;
```

## Troubleshooting

### Port Already in Use

```bash
# Kill process on port 8000
lsof -ti:8000 | xargs kill -9

# Or use a different port
mkdocs serve --dev-addr=127.0.0.1:8080
```

### Module Not Found Errors

```bash
# Reinstall dependencies
pip install --upgrade -r requirements.txt
```

### Theme Not Loading

```bash
# Clear pip cache
pip cache purge

# Reinstall mkdocs-material
pip install --force-reinstall mkdocs-material
```

## Next Steps

- [Documentation Testing](testing.md)
- [Documentation Standards](code-standards.md)
- [Contribute to Stellar-K8s](https://github.com/OtowoOrg/Stellar-K8s/blob/main/CONTRIBUTING.md)

!!! success "Ready to Contribute!"
    Your development environment is set up. Start writing documentation and submit a pull request!
