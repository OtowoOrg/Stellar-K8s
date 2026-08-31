# License Header Compliance

Stellar-K8s requires all source files (Rust, Shell, and YAML) to contain the canonical Apache-2.0 license header. This document details the expected formats, automated checks, and exceptions.

## Canonical Header Formats

### Rust (`.rs`)
```rust
// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
```

### Shell (`.sh`)
```bash
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
```

### YAML (`.yaml` / `.yml`)
```yaml
# Copyright 2024 Stellar-K8s Contributors
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.
```

## Validation & Local Enforcement

### Pre-commit hook
The license header check runs automatically on every commit. To run it manually:
```bash
make pre-commit
```
Or specifically for license headers:
```bash
pre-commit run license-headers --all-files
```

### Auto-fixing missing headers
You can use the helper script to automatically add headers to all applicable files:
```bash
python3 scripts/check-license-headers.py --fix
```

## Exceptions

The following paths and files are excluded from license header enforcement (configured in `scripts/check-license-headers.py`):

- **Build & Dependency Artifacts**: `target/`, `vendor/`, `Cargo.lock`, `package-lock.json`
- **Configuration & Templates**: `charts/stellar-operator/templates/`, `charts/stellar-operator/tests/`, `config/samples/`, `examples/`
- **Documentation**: `docs/`, `*.md`
- **Metadata & Git Config**: `.git/`, `.github/`, `.kiro/`, `.pre-commit-config.yaml`, `.gitignore`, `.dockerignore`
- **Dashboard & Schemas**: `monitoring/*.json`, `schemas/`, `config/grafana/`
