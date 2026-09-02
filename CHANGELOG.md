# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
## Chart v1.3.2 (2026-09-02) [patch]

🐛 fix(ci): clean up redundant workflows, fix build, and resolve dependency issues
• - Delete 5 redundant workflows (wave-security-compliance, yaml-schema-validation,
•   k8s-manifest-validation, helm-drift-detection, db-migration-testing) as they
•   were duplicating functionality already covered by existing jobs
• - Fix Dockerfile stage numbering and comments for clarity
• - Fix bundle.Dockerfile metadata (Go -> Rust project layout)
• - Remove deprecated 'version' field from all 4 docker-compose files
• - Simplify ci.yml: remove duplicate clippy run, consolidate image security scanning
•   into container-image-security.yml, streamline test/coverage job dependencies
• - Fix ci-reliability-test.yml dead code (duplicate find call)
• - Fix dr-drill.yml broken Prometheus query job (prometheus unreachable at
•   http://prometheus:9090)
• - Fix README.md Rust version (1.95 -> 1.98 to match toolchain)
• - Fix .dockerignore blocking docs/api/openapi.yaml needed by include_bytes!
• - Downgrade k8s-openapi from 0.26 to 0.22 to match kube 0.94 dependency
• - Fix rcgen API changes: Ia5String moved to rcgen::string::Ia5String,
•   signed_by() now takes (public_key, &Issuer) instead of (key_pair, ca_cert, ca_key_pair)
• Build verified on AWS EC2 VM (t3.xlarge, Ubuntu 22.04):
• - cargo build passes (dev profile)
• - Docker image builds successfully (74.6MB runtime image)
• - Helm chart lints clean, templates render correctly (1571 lines)
• Note: 30 pre-existing test compilation errors remain where test structs
• (ControllerState, AuditRecorder, AuditLog, AnomalyDetector) are out of
• sync with the actual code. These were never caught because the project
• could not build before the k8s-openapi fix.


## Chart v1.3.1 (2026-09-01) [patch]

• Merge pull request #1472 from OtowoOrg/dependabot/github_actions/github-actions-813fcdc74f
📝 ci(deps): bump the github-actions group with 15 updates
• Merge pull request #1468 from OtowoOrg/dependabot/docker/lukemathwalker/cargo-chef-latest-rust-1.98-slim-bookworm
📝 build(deps): bump lukemathwalker/cargo-chef from latest-rust-1.95-slim-bookworm to latest-rust-1.98-slim-bookworm
• Merge pull request #1469 from OtowoOrg/dependabot/cargo/production-dependencies-ad20fc3b21
• deps(deps): bump the production-dependencies group with 20 updates
• Merge pull request #1470 from OtowoOrg/dependabot/cargo/kubernetes-client-4125ce749a
• deps(deps): bump k8s-openapi from 0.22.0 to 0.26.1 in the kubernetes-client group
• Merge pull request #1471 from OtowoOrg/dependabot/cargo/security-105db6feec
• deps(deps): bump rcgen from 0.13.2 to 0.14.10 in the security group
📝 ci(deps): bump the github-actions group with 15 updates
• Bumps the github-actions group with 15 updates:
• | Package | From | To |
• | --- | --- | --- |
• | [actions/checkout](https://github.com/actions/checkout) | `4` | `7` |
• | [actions/setup-python](https://github.com/actions/setup-python) | `5` | `7` |
• | [actions/upload-artifact](https://github.com/actions/upload-artifact) | `4` | `7` |
• | [actions/download-artifact](https://github.com/actions/download-artifact) | `4` | `8` |
• | [azure/setup-helm](https://github.com/azure/setup-helm) | `4` | `5` |
• | [helm/kind-action](https://github.com/helm/kind-action) | `1.10.0` | `1.14.0` |
• | [docker/setup-buildx-action](https://github.com/docker/setup-buildx-action) | `3` | `4` |
• | [docker/build-push-action](https://github.com/docker/build-push-action) | `6` | `7` |
• | [docker/metadata-action](https://github.com/docker/metadata-action) | `5` | `6` |
• | [docker/login-action](https://github.com/docker/login-action) | `3` | `4` |
• | [github/codeql-action](https://github.com/github/codeql-action) | `3` | `4` |
• | [actions/github-script](https://github.com/actions/github-script) | `7` | `9` |
• | [dependabot/fetch-metadata](https://github.com/dependabot/fetch-metadata) | `2` | `3` |
• | [google-github-actions/setup-gcloud](https://github.com/google-github-actions/setup-gcloud) | `1` | `3` |
• | [ossf/scorecard-action](https://github.com/ossf/scorecard-action) | `2.4.0` | `2.4.4` |
• Updates `actions/checkout` from 4 to 7
• - [Release notes](https://github.com/actions/checkout/releases)
• - [Changelog](https://github.com/actions/checkout/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/actions/checkout/compare/v4...v7)
• Updates `actions/setup-python` from 5 to 7
• - [Release notes](https://github.com/actions/setup-python/releases)
• - [Commits](https://github.com/actions/setup-python/compare/v5...v7)
• Updates `actions/upload-artifact` from 4 to 7
• - [Release notes](https://github.com/actions/upload-artifact/releases)
• - [Commits](https://github.com/actions/upload-artifact/compare/v4...v7)
• Updates `actions/download-artifact` from 4 to 8
• - [Release notes](https://github.com/actions/download-artifact/releases)
• - [Commits](https://github.com/actions/download-artifact/compare/v4...v8)
• Updates `azure/setup-helm` from 4 to 5
• - [Release notes](https://github.com/azure/setup-helm/releases)
• - [Changelog](https://github.com/Azure/setup-helm/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/azure/setup-helm/compare/v4...v5)
• Updates `helm/kind-action` from 1.10.0 to 1.14.0
• - [Release notes](https://github.com/helm/kind-action/releases)
• - [Commits](https://github.com/helm/kind-action/compare/v1.10.0...v1.14.0)
• Updates `docker/setup-buildx-action` from 3 to 4
• - [Release notes](https://github.com/docker/setup-buildx-action/releases)
• - [Commits](https://github.com/docker/setup-buildx-action/compare/v3...v4)
• Updates `docker/build-push-action` from 6 to 7
• - [Release notes](https://github.com/docker/build-push-action/releases)
• - [Commits](https://github.com/docker/build-push-action/compare/v6...v7)
• Updates `docker/metadata-action` from 5 to 6
• - [Release notes](https://github.com/docker/metadata-action/releases)
• - [Commits](https://github.com/docker/metadata-action/compare/v5...v6)
• Updates `docker/login-action` from 3 to 4
• - [Release notes](https://github.com/docker/login-action/releases)
• - [Commits](https://github.com/docker/login-action/compare/v3...v4)
• Updates `github/codeql-action` from 3 to 4
• - [Release notes](https://github.com/github/codeql-action/releases)
• - [Changelog](https://github.com/github/codeql-action/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/github/codeql-action/compare/v3...v4)
• Updates `actions/github-script` from 7 to 9
• - [Release notes](https://github.com/actions/github-script/releases)
• - [Commits](https://github.com/actions/github-script/compare/v7...v9)
• Updates `dependabot/fetch-metadata` from 2 to 3
• - [Release notes](https://github.com/dependabot/fetch-metadata/releases)
• - [Commits](https://github.com/dependabot/fetch-metadata/compare/v2...v3)
• Updates `google-github-actions/setup-gcloud` from 1 to 3
• - [Release notes](https://github.com/google-github-actions/setup-gcloud/releases)
• - [Changelog](https://github.com/google-github-actions/setup-gcloud/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/google-github-actions/setup-gcloud/compare/v1...v3)
• Updates `ossf/scorecard-action` from 2.4.0 to 2.4.4
• - [Release notes](https://github.com/ossf/scorecard-action/releases)
• - [Changelog](https://github.com/ossf/scorecard-action/blob/main/RELEASE.md)
• - [Commits](https://github.com/ossf/scorecard-action/compare/v2.4.0...v2.4.4)
• ---
• updated-dependencies:
• - dependency-name: actions/checkout
•   dependency-version: '7'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: actions/setup-python
•   dependency-version: '7'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: actions/upload-artifact
•   dependency-version: '7'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: actions/download-artifact
•   dependency-version: '8'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: azure/setup-helm
•   dependency-version: '5'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: helm/kind-action
•   dependency-version: 1.14.0
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: github-actions
• - dependency-name: docker/setup-buildx-action
•   dependency-version: '4'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: docker/build-push-action
•   dependency-version: '7'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: docker/metadata-action
•   dependency-version: '6'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: docker/login-action
•   dependency-version: '4'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: github/codeql-action
•   dependency-version: '4'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: actions/github-script
•   dependency-version: '9'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: dependabot/fetch-metadata
•   dependency-version: '3'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: google-github-actions/setup-gcloud
•   dependency-version: '3'
•   dependency-type: direct:production
•   update-type: version-update:semver-major
•   dependency-group: github-actions
• - dependency-name: ossf/scorecard-action
•   dependency-version: 2.4.4
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: github-actions
• ...
• Signed-off-by: dependabot[bot] <support@github.com>
• deps(deps): bump rcgen from 0.13.2 to 0.14.10 in the security group
• Bumps the security group with 1 update: [rcgen](https://github.com/rustls/rcgen).
• Updates `rcgen` from 0.13.2 to 0.14.10
• - [Release notes](https://github.com/rustls/rcgen/releases)
• - [Commits](https://github.com/rustls/rcgen/compare/v0.13.2...v0.14.10)
• ---
• updated-dependencies:
• - dependency-name: rcgen
•   dependency-version: 0.14.10
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: security
• ...
• Signed-off-by: dependabot[bot] <support@github.com>
• deps(deps): bump k8s-openapi in the kubernetes-client group
• Bumps the kubernetes-client group with 1 update: [k8s-openapi](https://github.com/Arnavion/k8s-openapi).
• Updates `k8s-openapi` from 0.22.0 to 0.26.1
• - [Release notes](https://github.com/Arnavion/k8s-openapi/releases)
• - [Changelog](https://github.com/Arnavion/k8s-openapi/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/Arnavion/k8s-openapi/compare/v0.22.0...v0.26.1)
• ---
• updated-dependencies:
• - dependency-name: k8s-openapi
•   dependency-version: 0.26.1
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: kubernetes-client
• ...
• Signed-off-by: dependabot[bot] <support@github.com>
• deps(deps): bump the production-dependencies group with 20 updates
• Bumps the production-dependencies group with 20 updates:
• | Package | From | To |
• | --- | --- | --- |
• | [glob](https://github.com/rust-lang/glob) | `0.3.3` | `0.3.4` |
• | [tokio](https://github.com/tokio-rs/tokio) | `1.52.3` | `1.53.1` |
• | [tokio-util](https://github.com/tokio-rs/tokio) | `0.7.18` | `0.7.19` |
• | [futures](https://github.com/rust-lang/futures-rs) | `0.3.32` | `0.3.34` |
• | [serde](https://github.com/serde-rs/serde) | `1.0.228` | `1.0.229` |
• | [serde_json](https://github.com/serde-rs/json) | `1.0.150` | `1.0.151` |
• | [regex](https://github.com/rust-lang/regex) | `1.12.3` | `1.13.1` |
• | [http](https://github.com/hyperium/http) | `1.4.0` | `1.5.0` |
• | [anyhow](https://github.com/dtolnay/anyhow) | `1.0.103` | `1.0.104` |
• | [clap](https://github.com/clap-rs/clap) | `4.6.1` | `4.6.6` |
• | [clap_complete](https://github.com/clap-rs/clap) | `4.6.5` | `4.6.9` |
• | [chrono](https://github.com/chronotope/chrono) | `0.4.44` | `0.4.45` |
• | [bytes](https://github.com/tokio-rs/bytes) | `1.11.1` | `1.12.1` |
• | [rustls](https://github.com/rustls/rustls) | `0.23.40` | `0.23.43` |
• | [rustls-pki-types](https://github.com/rustls/pki-types) | `1.14.1` | `1.15.1` |
• | [flate2](https://github.com/rust-lang/flate2-rs) | `1.1.9` | `1.1.10` |
• | [async-trait](https://github.com/dtolnay/async-trait) | `0.1.89` | `0.1.92` |
• | [aws-sdk-s3](https://github.com/awslabs/aws-sdk-rust) | `1.132.0` | `1.134.0` |
• | [md5](https://github.com/stainless-steel/md5) | `0.8.0` | `0.8.1` |
• | [wat](https://github.com/bytecodealliance/wasm-tools) | `1.251.0` | `1.258.0` |
• Updates `glob` from 0.3.3 to 0.3.4
• - [Release notes](https://github.com/rust-lang/glob/releases)
• - [Changelog](https://github.com/rust-lang/glob/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/rust-lang/glob/compare/v0.3.3...v0.3.4)
• Updates `tokio` from 1.52.3 to 1.53.1
• - [Release notes](https://github.com/tokio-rs/tokio/releases)
• - [Commits](https://github.com/tokio-rs/tokio/compare/tokio-1.52.3...tokio-1.53.1)
• Updates `tokio-util` from 0.7.18 to 0.7.19
• - [Release notes](https://github.com/tokio-rs/tokio/releases)
• - [Commits](https://github.com/tokio-rs/tokio/compare/tokio-util-0.7.18...tokio-util-0.7.19)
• Updates `futures` from 0.3.32 to 0.3.34
• - [Release notes](https://github.com/rust-lang/futures-rs/releases)
• - [Changelog](https://github.com/rust-lang/futures-rs/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/rust-lang/futures-rs/compare/0.3.32...0.3.34)
• Updates `serde` from 1.0.228 to 1.0.229
• - [Release notes](https://github.com/serde-rs/serde/releases)
• - [Commits](https://github.com/serde-rs/serde/compare/v1.0.228...v1.0.229)
• Updates `serde_json` from 1.0.150 to 1.0.151
• - [Release notes](https://github.com/serde-rs/json/releases)
• - [Commits](https://github.com/serde-rs/json/compare/v1.0.150...v1.0.151)
• Updates `regex` from 1.12.3 to 1.13.1
• - [Release notes](https://github.com/rust-lang/regex/releases)
• - [Changelog](https://github.com/rust-lang/regex/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/rust-lang/regex/compare/1.12.3...1.13.1)
• Updates `http` from 1.4.0 to 1.5.0
• - [Release notes](https://github.com/hyperium/http/releases)
• - [Changelog](https://github.com/hyperium/http/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/hyperium/http/compare/v1.4.0...v1.5.0)
• Updates `anyhow` from 1.0.103 to 1.0.104
• - [Release notes](https://github.com/dtolnay/anyhow/releases)
• - [Commits](https://github.com/dtolnay/anyhow/compare/1.0.103...1.0.104)
• Updates `clap` from 4.6.1 to 4.6.6
• - [Release notes](https://github.com/clap-rs/clap/releases)
• - [Changelog](https://github.com/clap-rs/clap/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/clap-rs/clap/compare/clap_complete-v4.6.1...clap_complete-v4.6.6)
• Updates `clap_complete` from 4.6.5 to 4.6.9
• - [Release notes](https://github.com/clap-rs/clap/releases)
• - [Changelog](https://github.com/clap-rs/clap/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/clap-rs/clap/compare/clap_complete-v4.6.5...clap_complete-v4.6.9)
• Updates `chrono` from 0.4.44 to 0.4.45
• - [Release notes](https://github.com/chronotope/chrono/releases)
• - [Changelog](https://github.com/chronotope/chrono/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/chronotope/chrono/compare/v0.4.44...v0.4.45)
• Updates `bytes` from 1.11.1 to 1.12.1
• - [Release notes](https://github.com/tokio-rs/bytes/releases)
• - [Changelog](https://github.com/tokio-rs/bytes/blob/master/CHANGELOG.md)
• - [Commits](https://github.com/tokio-rs/bytes/compare/v1.11.1...v1.12.1)
• Updates `rustls` from 0.23.40 to 0.23.43
• - [Release notes](https://github.com/rustls/rustls/releases)
• - [Changelog](https://github.com/rustls/rustls/blob/main/CHANGELOG.md)
• - [Commits](https://github.com/rustls/rustls/compare/v/0.23.40...v/0.23.43)
• Updates `rustls-pki-types` from 1.14.1 to 1.15.1
• - [Release notes](https://github.com/rustls/pki-types/releases)
• - [Commits](https://github.com/rustls/pki-types/compare/v/1.14.1...v/1.15.1)
• Updates `flate2` from 1.1.9 to 1.1.10
• - [Release notes](https://github.com/rust-lang/flate2-rs/releases)
• - [Commits](https://github.com/rust-lang/flate2-rs/compare/1.1.9...1.1.10)
• Updates `async-trait` from 0.1.89 to 0.1.92
• - [Release notes](https://github.com/dtolnay/async-trait/releases)
• - [Commits](https://github.com/dtolnay/async-trait/compare/0.1.89...0.1.92)
• Updates `aws-sdk-s3` from 1.132.0 to 1.134.0
• - [Release notes](https://github.com/awslabs/aws-sdk-rust/releases)
• - [Commits](https://github.com/awslabs/aws-sdk-rust/commits)
• Updates `md5` from 0.8.0 to 0.8.1
• - [Commits](https://github.com/stainless-steel/md5/commits)
• Updates `wat` from 1.251.0 to 1.258.0
• - [Release notes](https://github.com/bytecodealliance/wasm-tools/releases)
• - [Commits](https://github.com/bytecodealliance/wasm-tools/compare/v1.251.0...v1.258.0)
• ---
• updated-dependencies:
• - dependency-name: glob
•   dependency-version: 0.3.4
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: tokio
•   dependency-version: 1.53.1
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: tokio-util
•   dependency-version: 0.7.19
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: futures
•   dependency-version: 0.3.34
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: serde
•   dependency-version: 1.0.229
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: serde_json
•   dependency-version: 1.0.151
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: regex
•   dependency-version: 1.13.1
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: http
•   dependency-version: 1.5.0
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: anyhow
•   dependency-version: 1.0.104
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: clap
•   dependency-version: 4.6.6
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: clap_complete
•   dependency-version: 4.6.9
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: chrono
•   dependency-version: 0.4.45
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: bytes
•   dependency-version: 1.12.1
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: rustls
•   dependency-version: 0.23.43
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: rustls-pki-types
•   dependency-version: 1.15.1
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: flate2
•   dependency-version: 1.1.10
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: async-trait
•   dependency-version: 0.1.92
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: aws-sdk-s3
•   dependency-version: 1.134.0
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• - dependency-name: md5
•   dependency-version: 0.8.1
•   dependency-type: direct:production
•   update-type: version-update:semver-patch
•   dependency-group: production-dependencies
• - dependency-name: wat
•   dependency-version: 1.258.0
•   dependency-type: direct:production
•   update-type: version-update:semver-minor
•   dependency-group: production-dependencies
• ...
• Signed-off-by: dependabot[bot] <support@github.com>
📝 build(deps): bump lukemathwalker/cargo-chef
• Bumps lukemathwalker/cargo-chef from latest-rust-1.95-slim-bookworm to latest-rust-1.98-slim-bookworm.
• ---
• updated-dependencies:
• - dependency-name: lukemathwalker/cargo-chef
•   dependency-version: latest-rust-1.98-slim-bookworm
•   dependency-type: direct:production
• ...
• Signed-off-by: dependabot[bot] <support@github.com>


## Chart v1.3.0 (2026-08-31) [minor]

• Merge pull request #1447 from otsimaofficial/feat/issue-1393-structured-error-handling
✨ feat: implement structured error handling across all services
• Merge remote-tracking branch 'upstream/main' into feat/issue-1393-structured-error-handling
• # Conflicts:
• #	docs/errors.md
• #	src/commands/backup.rs
• #	src/controller/tenant_reconciler.rs
• #	src/rest_api/dto.rs
• #	src/rest_api/server.rs
• #	src/security/cert_manager.rs
• Merge pull request #1465 from rudeus112266/test/1259-chaos-make-target
• Wire chaos engineering suite into make chaos-test
• Merge pull request #1467 from rudeus112266/docs/1359-dashboard-access
• Document metric naming conventions and Grafana dashboard access
• Merge pull request #1464 from rudeus112266/chore/1256-dev-setup-script
• Add unified developer environment setup script
• Merge pull request #1466 from rudeus112266/test/1358-chaos-quorum-loss
• Register Stellar Core crash-recovery chaos experiments in local runner
• Merge pull request #1462 from TheCreatorNode/feat/helm-chart-release-versioning
✨ feat(helm): harden automated chart release versioning (#1319)
• Merge pull request #1463 from TheCreatorNode/feat/network-policy-enforcement
✨ feat(helm): add pod-to-pod network policy enforcement (#1320)
• Merge branch 'main' into feat/helm-chart-release-versioning
• Merge pull request #1461 from TheCreatorNode/feat/helm-chart-release-tests
📝 test(helm): add bump-chart-version tests and fix first-commit analysis
• Document metric naming conventions and Grafana dashboard access
• Register Stellar Core crash-recovery chaos experiments in local runner
• Wire chaos engineering suite into make chaos-test
• Add unified developer environment setup script
✨ feat(helm): add pod-to-pod network policy enforcement (#1320)
• Enforce zero-trust pod-to-pod segmentation with default-deny and explicit
• allow rules for required service communication.
• - Add explicit egress allow rules to the operator default-deny for the
•   operator's required intra-cluster links (Redis rate limiting, Vault PKI,
•   OTel collector, Kafka SCP analytics), each gated on the matching feature
•   so the default render is unchanged.
• - Add templates/network-pod-policy.yaml implementing a per-namespace
•   default-deny (ingress+egress) baseline for any namespace listed in
•   security.networkPolicy.defaultDenyNamespaces.
• - Add helm-unittest coverage (network_policy_test.yaml, 11 tests).
• - Document the network topology and policy rationale in
•   docs/network-pod-to-pod.md and update related docs.
✨ feat(helm): harden automated chart release versioning (#1319)
• Implement the versioning.min-bump annotation as a minimum bump floor in
• bump-chart-version.sh, fix the root-commit exclusion that dropped the very
• first commit from analysis, and add bats coverage for the bump rules, the
• floor, and the --output-env mode.
• Also validate charts with helm lint --strict and helm unittest before
• publishing to the OCI registry, and register the new tests in CI and the
• Makefile.
📝 test(helm): add bump-chart-version tests and fix first-commit analysis
• Add bats coverage for scripts/bump-chart-version.sh (#1319) covering the
• SemVer bump rules (major/minor/patch/none), changelog generation, the
• --bump-override flag, --output-env GitHub Actions mode, and real Chart.yaml
• writes.
• Fix a bug where, before any chart-v* tag exists, the script used the root
• commit SHA as the analysis baseline which excluded the very first commit from
• the git log range. Leaving the baseline empty now analyzes all history.
✨ feat: implement structured error handling across all services
• Closes #1393.
• - Move ApiErrorCode/ErrorResponse into error.rs (unconditional) so both
•   rest_api and api_gateway share one definition instead of duplicating
•   it; rest_api::dto re-exports for compatibility. Add Error::status_code()
•   and Error::to_error_response() for consistent HTTP-code + JSON-envelope
•   mapping, plus ErrRateLimited/ErrGone codes.
• - Add correlation IDs: telemetry::resolve_correlation_id() reuses an
•   inbound X-Correlation-Id header or mints a UUID, http_trace_middleware
•   records it on the tracing span and echoes it back as a response header.
•   REST API handlers (list_nodes, get_node, set_log_level,
•   compliance_report) now populate ErrorResponse.correlation_id from it
•   instead of hardcoding None.
• - api_gateway::server: replace ad hoc (StatusCode, &str) responses with
•   the shared ErrorResponse envelope. Add graceful degradation: a
•   transform-response failure (we have upstream data, just couldn't
•   reshape it) returns ErrorResponse::degraded() with the raw upstream
•   body attached; an upstream-connection failure (no data, no cache)
•   returns a structured ERR_SERVICE_UNAVAILABLE instead.
• - docs/errors.md: document the Error -> StatusCode/ApiErrorCode mapping,
•   gateway-specific codes, degradation semantics, and the correlation-ID
•   mechanism end to end.
• Validated with cargo check --locked --bin stellar-operator (clean).
• Full clippy/lint-strict and test suite were not run locally due to this
• host's disk constraints; deferred to CI.
• Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
• Signed-off-by: otsimaofficial <iemmanuelogbu@gmail.com>
🐛 fix: repair broken kube-rs APIs in tenant_reconciler and syntax error in backup
• tenant_reconciler.rs referenced APIs that don't exist in kube 0.94
• (kube::utils::json_patch::*, kube::api::ReplaceParams,
• kube::api::apiextensions_apiserver::...::CustomResourceDefinition) and
• tried to build k8s_openapi Quantity via a nonexistent From<String> impl,
• so the crate failed to compile on every branch. backup.rs had a stray
• closing brace and referenced an undefined variable. Neither bug is
• specific to any single wave issue; fixing both here since they block
• building this branch at all.
• Also sweeps in cargo fmt output for a few pre-existing formatting-drifted
• files (backup-verify.rs, changelog-gen.rs, conventional-commit-check.rs,
• controller/mod.rs) picked up while validating the build.
• Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>
• Signed-off-by: otsimaofficial <iemmanuelogbu@gmail.com>


## Chart v1.2.0 (2026-08-31) [minor]

• Merge pull request #1433 from Shindailulu/fix-license-and-security-1397-1400
• Implement wave issues 1397-1400
• Merge branch 'main' into fix-license-and-security-1397-1400
• Merge pull request #1459 from Sulamoney222/8-reentrancy-guard-middleware
✨ feat(security): Soroban reentrancy guard middleware
✨ feat(security): add Soroban reentrancy guard middleware
• Implements a native reentrancy guard sub-contract middleware under
• wasm-plugins/security/reentrancy/, enforced through the Stellar-K8s custom
• validation (Wasm) layer (issue #8).
• - Storage-agnostic write-lock stack core that reverts nested, mutating
•   cross-contract re-entries of the same state variable while producing zero
•   false positives on non-mutating read callbacks.
• - ConfigMap-driven per-namespace / per-contract-ID scoping with a safe
•   "enabled everywhere" default and explicit opt-outs.
• - Optional 'soroban' feature binds the core to Soroban host instance storage
•   and compiles to a no_std (alloc) wasm32-unknown-unknown guest that ships a
•   minimal global allocator; overhead stays < 500 instructions (MAX_DEPTH=8).
• - Deliberately vulnerable mock vault plus a 19-unit/7-integration security
•   suite proving the exploit and its prevention.
• - ADR 0005 documenting the locking mechanism, plus deployable ConfigMap
•   example.
🐛 fix: add missing license headers to new upstream files
• Merge upstream/main into fix-license-and-security-1397-1400
🐛 fix: update api openapi spec, add missing license headers, and ignore new rust security advisories
• Merge upstream/main into fix-license-and-security-1397-1400
📝 ci: resolve all CI/CD failures and enforce license header compliance
📝 docs: add license header enforcement guide


## Chart v1.1.1 (2026-08-31) [patch]

• Merge pull request #1460 from olalois/fix-issue-1198-delete-obsolete-CI-cache-keys-and-normalize-cache-usage
🐛 fix: issue-1198-delete-obsolete-CI-cache-keys-and-normalize-cache-usage
🐛 fix: relove issues 1197 & 1198
🐛 fix: issue-1198-delete-obsolete-CI-cache-keys-and-normalize-cache-usage


## Chart v1.1.0 (2026-08-30) [minor]

• Merge pull request #1457 from Divine-designs/feat/stellar-wave-dr-ha
✨ feat: DR/HA wave — chaos drills, log aggregation, compliance scanning, federation (#1412 #1411 #1410 #1409)
• Merge pull request #1458 from euniceotowo/feat/1258-metrics-monitoring-dashboards
✨ feat(monitoring): implement comprehensive metrics and monitoring dashboards
✨ feat: add multi-cluster federation sample, secret sync, and failover runbook (#1409)
✨ feat: add organisational compliance policies and standard CSV compliance reports (#1410)
🐛 fix: define and mount the CRI parser so the Fluent Bit log shipper starts (#1411)
✨ feat: honour scheduled CronJob env vars in chaos drills and add results tracking (#1412)
✨ feat(monitoring): implement comprehensive metrics and monitoring dashboards
• - Add monitoring setup guide with local dev and production deployment
• - Add operational runbook with health checks and troubleshooting
• - Implement monitoring status endpoint with health indicators
• - Add docker-compose monitoring stack overlay
• - Create Prometheus, Grafana, AlertManager configurations
• - Add monitoring status DTOs and handlers
• - Add comprehensive dashboard integration tests
• - Update REST API with monitoring health check route
• Closes #1258


## Chart v1.0.0 (2026-08-30) [major]




## [unreleased]

### Added

- Automated API documentation generation from code annotations and CRD schema with versioned docs-as-code and CI link checking (#1424)
- Feature flag system for gradual rollouts with percentage bucketing, user/segment targeting, allow/deny lists, and ConfigMap hot-reloading (#1423)
- Automated load testing pipeline in CI with k6, performance budgets, SLO targets, and trend tracking (#1422)
- Distributed rate limiting across API gateway with Redis-backed counters, atomic Lua scripts, fail-open resilience, and Prometheus alerting (#1421)

## [0.1.0] - 2026-07-27

### Add

- Comprehensive testing for the traffic shaping/rate-limiting controller and implements a Kubernetes Custom Metrics API server to enable HPA-based autoscaling on Stellar-specific metrics.

### Added

- Implement Stellar Kubernetes Operator with custom resources, controller, REST API, and Helm chart.
- Add contributor welcome template, project logo, and update gitignore to exclude Stellar Wave artifacts.
- Add support for external postgres database
- ReadyReplicas
- ServiceMonitor
- Ingress
- *(metrics)* Add stellar_node_ledger_sequence gauge and expose /metrics
- Implement automated history archive health check with retry logic
- Implement automated history archive health check with retry #26
- Implement OpenTelemetry tracing support #37
- Implement Maintenance Mode flag
- Implement auto-sync health checks for Horizon and Soroban RPC nodes (#19)
- *(metrics)* Add stellar_node_ledger_sequence gauge and expose /metrics
- Implement auto-remediation for stale/desynced nodes (#35)
- Add support for suspended validators in StellarNode
- *(operator)* Add NodePort support and StellarNode CRD
- Grafana dashboard
- Integrate MetalLB/BGP Anycast for Global Node Discovery
- Add automated performance benchmarking suite
- *(webhook)* Implement Wasm-based admission webhook for custom validation
- Add support for topologySpreadConstraints in StellarNodeSpec
- Decentralized Storage Backup Implementation
- Proper Organisation
- Proper Organisation
- *(horizon)* Add automatic database migration support for Horizon nodes
- Implement cross-region multi-cluster disaster recovery
- *(controller)* Implement automated PodDisruptionBudget management
- Implement custom schedular
- Add support for canary rollouts with traffic weighting and automated rollback
- Add cross-cluster communication and synchronization support
- Introduce Hardware Security Module (HSM) configuration for validator nodes and add service port settings to the CRD.
- Add `hsm_config` field to `StellarCoreConfig` defaults and examples.
- Implemtn better error handling
- Add dry-run mode to reconciler
- Add version and info subcommands to operator binary
- Fix CI/CD failures
- History-node
- Fix ci
- Add implementation of core config generator
- Implement E2E Integration Test Suite with KinD
- Implemtn better error handling
- Add dry-run mode to reconciler
- Add version and info subcommands to operator binary
- Fix CI/CD failures
- Add version and info subcommands to operator binary
- Fix CI/CD failures
- Enhance StellarNode spec validation with type-specific rules for Validator, Horizon, and SorobanRpc nodes, and add general feature validations.
- Implement leader election, dry-run test, and CVE test coverage
- Build both binaries in single cargo build step with cargo-chef caching
- Verify helm chart lints and renders valid manifests (#148)
- Add integration tests for backup scheduler and remediation module
- Add wiremock integration tests for archive health checks
- State machine fuzzer
- Add comprehensive test coverage for reconciler module
- Add dummy client helper function for testing without kubeconfig
- Add read replica configuration to StellarNode and related tests
- *(operator)* Implement auto-scaling read-only replica pools
- Add end-to-end test for Horizon node lifecycle with health checks
- Add OLM bundle packaging support
- Integrate Chaos Engineering
- Read Pool Optimization
- Implement Network Topology
- Add CRD generation utility and remove static StellarNode CRD definition
- Helm: Integration with External Secrets Operator (ESO)
- Implement carbon-aware scheduling for Stellar nodes
- Implement carbon-aware scheduling for Stellar nodes
- Implement Automated Upgrade Strategy
- Add debug subcommand to kubectl-stellar plugin
- Implement automated Horizon DB maintenance (#252)
- Self-Healing State: Automated DB Vacuum and Reindexing
- Certificate rotation
- Unit tests for the wasm admission
- *(spec)* Add SCP Quorum Analysis Dashboard specification
- Add analyzer details
- Add analyzer files
- Add quorum analysis module
- *(cli)* Add explain command to kubectl-stellar to decode error codes
- Implement LocalStorage nodeAffinity and volume capabilities for CRD
- Add rust-toolchain
- Add rust-toolchain.
- Add operator metrics to grafana dashboard and update README
- *(dr)* Add DR drill schedule types to CRD
- *(dr)* Implement DR drill orchestrator module
- *(dr)* Integrate DR drill orchestrator into reconciliation loop
- *(dr)* Add DR drill metrics for monitoring
- *(dr)* Integrate metrics recording into DR drill execution
- *(dashboard)* Add web-based operator dashboard with REST API
- *(dashboard)* Add operator performance dashboard with web UI
- *(cve)* Add auto-patch safety gate with annotation control
- *(benchmarks)* Add performance regression testing framework
- Vault secrets, forensic snapshots, simulator, Chaos Mesh
- Implement dry-run mode and Architecture Decision Records
- Add preflight self-test, audit trail annotations
- Auto-balancing validator weights based, Distributed ML model training for network attack detection, Hardware Security Module support for validator seed protection
- *(scheduling)* Default pod anti-affinity and AZ-aware topology spread (#259)
- Add Changelog Generation with conventional-changelog
- Add Docker Compose development environment (#315)
- Implement retry backoff configuration for reconciler (#314)
- Add image digest pinning support and mutable tag warnings (#323)
- *(controller)* Emit Stellar audit events via kube-rs Recorder
- Standardize Error Messages with Error Codes and Documentation
- Implement CONTRIBUTING.md with DCO and PR Guidelines
- Add Makefile with Standard Development Targets
- Implement namespace-scoped operator mode (#322)
- Add standard labels and ownerReferences to all managed child resources
- Add quickstart guide and make quickstart target for Kind cluster setup
- Add ConfigMap-based runtime feature flags with live watcher
- Add operator version, leader status, and uptime Prometheus metrics
- Implement 'stellar logs' command in CLI
- Add Shell Completions and Enhanced Info Command
- Add version command, shell completion, condition tests, and scalability docs
- Four issues
- Four issues
- Four issues
- Implement 'stellar-operator' Crash Loop Analysis sidecar
- Cache VSL fetches
- Update_check_in_interval Function
- Expose node hardware generation
- Four issues
- Four issues
- Implement 'Stellar-K8s' Documentation Search Engine
- Add Support for Node Anti-Affinity based on SCP slices
- Implement 'stellar-operator' Dynamic Log Level Control
- Error mapping
- PDB supports
- Stellar prune command for history archives
- Stellar diff command to compare CRD
- [253] STUN/TURN Integration for Managed Nodes
- Add sidecar container support to StellarNodeSpec (#16)
- Implement Automatic Checkpoint Integrity' check for Archives
- Implement 'Stellar-K8s' Post-Mortem Template and Tooling
- Implement deep readiness probe and operator readiness metric (updated to latest main)
- Add OpenAPI v3 validation for StellarNetwork names #366
- Add OpenAPI v3 validation for StellarNetwork names #366
- Implement reconciler property tests and workload hardening
- Implement 'Service Mesh' mTLS enforcement guide
- Add Support for OPA/Gatekeeper Policies for StellarNode
- Implement 'stellar-operator' Self-Upgrade Simulation
- Implement 'stellar-operator' Self-Upgrade Simulation
- Add pre-commit hooks for code quality enforcement
- Add sample stellarnode manifests and ci smoke test
- Introduce CRD schema utilities, refactor Stellar network custom passphrase handling, and update rollout strategy definition.
- Implement comprehensive security testing including penetration testing vulnerability assessments compliance monitoring (closes AC)
- *(kubectl)* Verify kubectl-stellar builds and works as plugin
- Issue
- *(metrics)* Add stellar_node_sync_status gauge for tracking node phases
- *(metrics)* Add stellar_node_up gauge metric for node health
- Implement log scrubbing layer for sensitive data redaction
- Improve version subcommand to fetch operator version from deployment label
- Add memory soak test CI workflow
- Add DR failover e2e test
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- Resolving issues
- *(scripts)* Standardize retry/backoff helper and add DRY_RUN mode to all batch scripts
- Implement 4 high-difficulty issues for Stellar-K8s
- Add k8s version feature flags for k8s-openapi
- Add Helm values schema for stellar-operator chart
- [255] add background job monitoring dashboard
- [252] add webhook delivery system for transaction events
- [253] add audit log endpoint for admin activity
- Add end-of-run summary report for issue batches
- Implement #510 #511 #512 #514 — probes, validation DX, dry-run, branding
- Add gh auth and label readiness preflight checks
- Add StellarBenchmark CRD and built-in performance test controller
- *(security)* Enforce Mainnet/Testnet network isolation (SK8S-021)
- Snapshot bootstrap for near-instant Stellar Core node sync
- All features completed
- Eslint fix
- *(workflow)* Standardize issue templates, parameterize soak tests, and centralize labels
- *(security,reliability,performance)* Implement OIDC auth, hitless upgrade, jurisdiction compliance, and predictive scaling
- [254] add Prisma connection pooling and query timeout config
- *(scripts)* Add run_batches.sh launcher for batch generators (#480)
- Hpa autoscaling based on WASM execution metrics (Issue #493)
- *(scripts)* Add EXPECTED_ISSUE_COUNT self-check to all batch issue scripts
- *(scripts)* Add -h/--help usage output to all batch issue scripts
- Durable log-to-S3 sidecar with CLI fetch tool
- Dynamic sync-state resource scaling for Stellar Core pods
- Implement multi-region ledger replication and failover CLI
- Add PVC pruning tests for Delete and Retain retention policies
- *(#507)* Add sidecar injection tests and documentation
- *(#508)* Integrate cert-manager for mTLS certificate rotation
- Add CLI version check and upgrade notification system
- Implement automated DB vacuuming orchestrator for Postgres
- Implement canary analysis engine using Kayenta integration
- Implement pod-to-pod mTLS enforcement using Linkerd
- Build stellar-native autoscaler for Horizon (rate-limit based)
- Implement automated DB vacumming orchestrator
- Built a  History Archive Pruning Worker with Lifecycle Integration
- Integrate OpenTelemetry SDK with OTLP export and trace-ID logging
- *(dashboard)* Add real-time SCP topology visualization
- *(archive)* Implement ZK verification for encrypted history backups
- Add summary command to kubectl-stellar plugin
- Implement Stellar Fork Detection sidecar
- Implement Automated Certificate Authority (CA) Management
- Implement stubs for #581 #582 #583 #584 to resolve issue acceptance criteria
- Add macOS development environment setup script
- Add code coverage reporting to CI pipeline
- *(metrics)* Implement advanced metrics pipeline with Prometheus federation
- *(policy)* Implement self-healing cluster policy engine with remediation
- *(certificates)* Implement comprehensive mTLS certificate management with rotation
- *(telemetry)* Implement distributed tracing with OpenTelemetry and Jaeger
- *(scripts)* Finalize batch launcher script
- Add support for extraAnnotations in deployment and service templates
- Add 'doctor' command for local environment verification
- *(cli)* Add --json flag to audit command for automated scanning #592
- Add --version and -v flags to stellar CLI
- Add  Response Toolkit / Improve Help Outpu/ Add Shell Completion
- Add release template for versioning and documentation
- Build Real-time SCP Analytics Dashboard using OpenSearch
- Implement multi-region federation, ML-based anomaly detection, and unified audit recording
- Implement issues #624, #625, #626, #627
- Build a custom Kubernetes metrics server for Stellar-specific scaling
- Build a custom Kubernetes metrics server for Stellar-specific scaling
- Implement zero-downtime database migrations for Horizon
- Update README badges for CI, coverage, and versioning
- Implement WebSocket-based real-time operator status streaming API (#637)
- Implement zero-downtime operator upgrades with canary strategy (#638)
- Build Byzantine-tolerant consensus monitoring with adaptive alerting (#639)
- Implement predictive load modeling and dynamic resource autoscaling (#640)
- Consolidate and optimize core CI workflows with shared caching
- Resolve issues #712, #702, #719, #718
- All issues resolved
- *(#732)* Implement Horizon query optimization with intelligent caching
- *(#733)* Build automated compliance reporting for regulatory requirements
- *(#735)* Implement advanced secret management with external KMS integration
- *(#734)* Implement ML-based dynamic resource optimization
- Add adaptive traffic shaping with QoS and rate limiting
- *(horizon)* Enforce rollback and failure metrics in blue-green migrations
- *(controller)* Add gitops protocol upgrade orchestration
- *(scheduler)* Add latency monitor with auto-eviction for proximity scheduling
- *(webhook)* Implement generic policy delegation framework
- All issues resolved
- *(validator)* Introduce native rust manifest validation engine for cluster resources
- *(logging)* Add log aggregation guide, helm configurations, and dashboard templates
- Multi-cluster guide, performance tuning, upgrade workflow, PVC auto-expansion
- Implement load balancer, message queue, schema registry, and deployment strategies
- *(ingress)* Add configurable NGINX rate limiting to ingress controller
- *(security)* Automated secret rotation for network passphrases (#709)
- *(crd)* Add initContainers support to StellarNode deployments (#710)
- *(tools)* Introduce unified web and cli capacity quota calculator for miva stellar node deployments
- Comprehensive enhancements for monitoring, dashboards, kubectl plugin, and Helm chart
- Add resiliency e2e tests and secure network policies
- *(#668)* Implement leader election for operator high availability
- Resolve issues #839, #840, #680, #681 — probes, priority class, latency scheduling, GitOps upgrades
- Advanced probes, leader election HA, and auto PDB (#704, #705, #707)
- Implement 4 epic CRDs - federation, autoscaling, upgrades, observability
- Implement advanced data pipeline with stream processing and ETL
- Build advanced workflow orchestration with DAG-based task execution
- *(webhook)* Enforce minimum resource requests in production mode
- *(performance)* Add StellarPerformance CRD with budgets and regression detection
- *(topology)* Add StellarTopology CRD with partition detection and simulation
- Implement advanced cost optimization with multi-cloud pricing analysis
- Build advanced service discovery with dynamic topology mapping
- Implement StellarNode status, ServiceMonitor, scheduling and env overrides
- Add automatic HPA creation for Horizon and Soroban RPC nodes
- Add custom init containers support to StellarNode pods
- Implement ResourceQuota awareness and validation in operator
- Add PodSecurityStandard and SecurityContext configuration to StellarNode
- Add sophisticated event processing system
- Add comprehensive API gateway with advanced features
- Add comprehensive chaos engineering framework
- Add sophisticated database management system
- Add documentation site infrastructure with mkdocs
- Add comprehensive getting started guides and deployment documentation
- Add tutorials and troubleshooting documentation
- Add contributing guides and configuration reference sections
- Add github actions workflow for automated documentation deployment
- *(scheduler)* Implement intelligent resource scheduling with ML-based optimization
- *(epic)* Add initial Wave 5 epic implementations
- Implement data pipeline, API gateway, and Horizon dashboard (#788, #789, #708)
- Cleanup docs, tests, and feature flags
- Cleanup docs, tests, and feature flags

### Documentation

- *(contributing)* Enhance pre-push checks and update guidelines
- Add before/after build time documentation for Dockerfile optimization
- Add CHANGELOG.md and link from README
- *(dashboard)* Add RBAC configuration example for dashboard access
- *(cve)* Add CVE auto-patch documentation and examples
- Fix run_controller doc-test after controller state update
- Add comprehensive k3d local development guide #367
- *(#509)* Add networking troubleshooting guide and debug script
- Add Minikube getting-started guide
- Architecture for #581 #582 #583 #584
- Add comprehensive glossary of Stellar-K8s terms
- Regenerate API reference documentation
- Implement bug, feature, and support issue templates #595
- Add Windows WSL2 setup guide (issue #593)
- Add FAQ section to provide answers to common questions
- Audit TOML code fences for correct syntax highlighting
- Add network policy templates
- Add comprehensive implementation summary for issues #757, #754, #755, #756
- Add leader election implementation summary for issue #668
- Build core onboarding guide, API reference, ops runbook, and interactive C4 architecture schemas (closes #803, closes #804, closes #805, closes #806)

### Fixed

- Resolve merge conflicts and fix Resource import after upstream sync
- Update check_node_health calls to include None parameter for improved health check functionality
- Streamline error handling and enhance test data structure
- Correct binding of pod to node by passing node reference directly
- Add missing cluster and cross_cluster fields to doctests
- Address clippy single_match warning in remediation logic
- Integrate PDB management and fix test initializations
- Add missing error type conversions for rcgen and io errors
- Cli
- Add resource_meta to all StellarNodeSpec initializers and doctests
- Implement requested fixes
- Lint errors
- Address clippy single_match warning in remediation logic
- Integrate PDB management and fix test initializations
- Unclosed delimiter
- Address clippy single_match warning in remediation logic
- Integrate PDB management and fix test initializations
- Lint and format errors
- Cargo fmt --all --check
- Clippy Lint with -D warnings
- Clippy errors
- CICD failure
- Remove duplicate read_replica_config field in kubectl_plugin
- Mod file
- Fix lint errors
- Resolve schema validation errors in example manifests
- Fix pipeline
- Fix pipeline
- Custom Grafana Dashboard for SOROBAN Specific Metrics (#222)
- Fix pipeline
- Wasm-Powered Admission Controller Layer (#230)
- Fix clippy error
- Security
- Operator Webhook Performance: Load Testing & Latency Benchmarks (#221)
- Ci
- Clippy warnings
- Remove pqc_sidecar.rs binary with unresolved dependencies
- Use correct actions-rs/audit-check@v1 and remove deleted pqc-sidecar artifact
- *(ci)* Fix cargo fmt and clippy warnings
- Resolve CI failures for LocalStorage testing and formatting
- Resolve clippy warnings and regenerate Cargo.lock
- Resolve clippy warnings and test compilation errors
- Remove unused imports and prefix unused parameters
- Format
- Resolve formatting and webhook route issues
- Apply rustfmt formatting to fix CI lint check
- Collapse short resolver assignments to single line for rustfmt
- Lint
- *(ci)* Use robust grep for helm schema validation
- Resolve compilation errors after rebase
- Fix ci/cd
- Fix pipeline
- Fix failing pipeline
- Fix main.rs
- Fix ci/cd
- Fix lint error
- Remove unused imports from reconciler files
- Format livez function signature
- Merge conflicts - add missing ControllerState fields and methods
- Remove unused import and fix span lifetime issues
- Resolve merge conflicts in main.rs and json_logging_test.rs
- Sort imports alphabetically
- Remove unused log_format match in webhook function
- Resolve clippy uninlined_format_args and rustfmt issues in types.rs
- Resolve conflicts
- Satisfy clippy in build script
- Resolve ci lint and compile regressions
- Resolve rustfmt formatting and handlers.rs syntax error
- Add sidecar property to Helm values schema
- Add podDisruptionBudget property to Helm values schema
- Remove trailing whitespace from all source files
- Resolve compilation errors in runbook and blue_green modules
- Use debug format for StellarNetwork in runbook
- Include URL and status code in VSL fetch error message
- Correct rustfmt formatting across test and source files
- *(ci)* Stabilize lint and pre-commit hooks
- Make retry budget configurable via env
- *(ci)* Unblock lint and pre-commit on branch 466
- *(ci)* Unblock pre-commit and formatting on branch 477
- Resolve fmt, clippy, and Cargo.lock drift CI failures
- Skip gh preflight when repository is unset
- Align CI checks and example manifests
- Align examples and schema with ci checks
- *(ci)* Unblock helm lint and cargo locked builds
- *(helm)* Remove null pdb fields from default values
- *(helm)* Define default featureFlags values
- *(deps)* Align schemars and k8s-openapi with kube
- *(ci)* Resolve pre-push check failures
- *(ci)* Resolve make lint clippy errors and unused imports
- *(merge)* Resolve Cargo.lock conflicts and fix k8s-openapi CI builds
- *(helm)* Add missing security property to values schema
- *(ci)* Update rustls-webpki to 0.103.13 and align pre-commit clippy with make lint
- *(helm)* Guard pdb nil pointer and trim Cargo.toml trailing newline
- *(helm)* Add featureFlags defaults to values.yaml and schema
- *(helm)* Add featureFlags defaults to values.yaml and schema
- *(helm)* Add featureFlags defaults to values.yaml and schema
- *(helm)* Add featureFlags defaults to values.yaml and schema
- *(code)* Passing CI checks
- *(code)* Passing CI checks
- *(code)* Passing CI checks
- *(code)* Passing CI checks
- *(scripts)* Clean up dry-run passthrough in run_batches.sh
- Resolve E0063 missing fields and clippy lints across controller and tests
- Resolve rebase conflicts and clippy lints in new upstream files
- Resolve merge conflicts
- Fix lint error
- Fix lint errror
- Fix lint error
- Fix errors
- Fix helm lint
- Correct punctuation in README for CI/CD integration instructions
- Add system dependencies for Docker build and CI workflows
- Enable ARM64 architecture for cross-compilation dependencies
- Add libcurl headers and remove trailing whitespace
- Add pkg-config path and cross-compilation flags for ARM64
- Use export for conditional OPENSSL_DIR and PKG_CONFIG_PATH in RUN commands
- Correct YAML indentation and use clamp() instead of max().min()
- Resolve merge conflicts, keep standardized retry/dry-run helpers
- Resolve clippy errors required for CI lint gate
- *(logging)* Relocate raw manifests to docs folder and upgrade fluentd image tag to clear CI gates
- Resolve compile errors
- Log CRD validation rejection details
- Default diagnostic sidecar resources
- Close mod tests brace in latency_monitor.rs; fix Helm template delimiters in chart CRDs
- Add missing closing paren on .route() call in rest_api/server.rs
- Remove unused import in gateway mod.rs
- Add missing closing parenthesis for horizon cache status route
- Resolve issues #904 #905 #906 #907 — docs links, preflight checks, test isolation, build scripts
- Resolve issues #908 #909 #910 #911 — dead code audit, config defaults, cleanup workflow docs, naming conventions

### Miscellaneous

- Add github action for cargo audit
- Update dependencies in Cargo.lock and Cargo.toml
- *(deps)* Remove unused packages and update dependencies in Cargo.lock
- *(deps)* Update Cargo.lock with new and upgraded dependencies
- *(ci)* Update GitHub workflows and dependencies
- *(deps)* Bump axum and axum-server to latest versions
- *(deps)* Update wasmtime and related crates to v24.0.5
- *(ci)* Update GitHub Actions workflow YAML formatting and Cargo.lock dependencies
- *(deps)* Update dependencies and upgrade wasmtime to 24.0.5
- Fix CI issues, fix build and update readme details
- Add proper fixes
- Fix bugs and brnach details
- Adjust details and fix inconsistencies
- Fix issues
- Fmt
- Adjust details
- Fix lint issues
- Fix lint
- Adjust details so CI runs
- Adjust details
- Update Cargo.lock to resolve CI build failure
- Fix pipeline issues
- Rustfmt scheduling label selectors
- Fix clippy uninlined_format_args in feature_flags watcher
- Add featureFlags schema validation to Helm values
- Fix broken reconciler declaration and apply rustfmt
- Fix publish_stellar_event, duplicate pod_anti_affinity, and instrument skip list
- Fix lint issue
- Fix lint again
- Remove v1_30 feature flag from k8s-openapi dependency
- *(lockfile)* Sync Cargo.lock for CI dependency graph
- Normalize resources section quality across batch scripts
- Apply rustfmt for CI lint check
- Merge upstream main and keep CI preflight fixes
- Update K8s to v1.30, refactor CRDs, and general cleanup
- Start setup for issue
- *(fmt)* Apply rustfmt to satisfy CI lint
- *(fmt)* Apply rustfmt to satisfy CI lint

### Performance

- *(benchmark)* Add initial benchmark results and regression report

### Refactor

- Consolidate CRD imports by removing unused types and fix indentation.

### Refactored

- Enhance node listing functionality and output formatting
- Introduce helper function for node phase retrieval and streamline log command parameters
- *(controller)* Improve code clarity and deprecate old phase usage
- *(dr)* Remove unused imports and variables in DR controller
- Simplify client initialization in run function
- Clean up comments and improve code structure in CVE handling modules
- Improve code formatting and organization
- Update StellarNodeSpec and related modules to disable unimplemented fields
- Remove unused fields from StellarNodeSpec and related modules
- Remove `load_balancer`, `global_discovery`, `cross_cluster`, and `cluster` fields from `StellarNodeSpec` and perform minor code cleanups.

### Security
- Type-safe error handling to prevent runtime failures
- TLS certificate generation for webhook server using `rcgen`
- Rustls-based TLS implementation for secure communications
- SHA256-based integrity verification for WASM plugins
- Security policy documentation (SECURITY.md)

[unreleased]: https://github.com/OtowoOrg/Stellar-K8s/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/OtowoOrg/Stellar-K8s/releases/tag/v0.1.0

- *(deps)* Bump the github-actions group with 9 updates
- *(deps)* Bump the github-actions group across 1 directory with 15 updates

### Styling

- Apply cargo fmt formatting fixes
- Remove trailing whitespace in cloudhsm-client container definition.
- Apply cargo fmt to preflight and audit modules
- Fix cargo fmt issues
- Apply cargo fmt across the codebase
- Apply rustfmt for CI lint consistency
- Satisfy rustfmt on shared modules
- Apply rustfmt to satisfy CI fmt-check gate
- Apply rustfmt after clippy fixes
- Apply rustfmt to all files failing fmt-check

### Testing

- Add comprehensive tests for CaptiveCoreConfigBuilder functionality
- Make soak cleanup timeout configurable and explicit
- Make soak retry delay configurable with validation
- Add robust signal-aware soak cleanup traps
- *(cli)* Add comprehensive CLI argument parser tests (issue #594)
- *(cli)* Add comprehensive CLI argument parser tests (issue #594)

### Build

- *(deps)* Bump lukemathwalker/cargo-chef
- *(deps)* Bump rust from 1.93-bookworm to 1.94-bookworm
- *(deps)* Bump lukemathwalker/cargo-chef

### Ci

- Reduce Dependabot noise - monthly updates, better grouping
- Add GitHub Actions workflow for performance regression testing
- Fix cargo-audit compatibility with Rust 1.88
- Use official rustsec audit-check action for security scanning
- Simplify security audit with direct cargo-audit execution
- Make performance regression tests more lenient for initial runs
- Fix performance regression workflow - consolidate cluster setup
- Disable performance regression on PR, enable manual trigger only
- Make webhook performance checks non-blocking
- Fix GitHub Actions permissions for PR comments
- Add verify-operator-boot workflow for issue #146
- Scope heavy checks to changed files
- Fetch PR refs before scoped pre-commit
- Relax commitlint subject case rule
- Fix yamllint issues in workflow updates
- Scope heavy checks to changed files
- Fetch PR refs before scoped pre-commit
- Relax commitlint subject case rule
- Fix yamllint issues in workflow updates
- Add scripts-only shellcheck gate
- Scope heavy checks to changed files
- Fetch PR refs before scoped pre-commit
- Relax commitlint subject case rule
- Fix yamllint issues in workflow updates
- Scope heavy checks to changed files
- Fetch PR refs before scoped pre-commit
- Relax commitlint subject case rule
- Fix yamllint issues in workflow updates
- Scope precommit checks to PR diff
- Consolidate core workflows with shared caching and pre-commit
- Fix yamllint line-length in ci.yml change detection
- Fix tarpaulin flags for coverage job compatibility
- Restore optimized heavy validation workflows with shared actions
- Unblock lint and commit message gates
- Unify performance and benchmark pipelines into matrix workflow
- Make performance report job resilient on fork PRs
- Harden regression benchmark job against setup and compare failures

### Deps

- *(deps)* Bump schemars in the serialization group
- *(deps)* Bump the production-dependencies group across 1 directory with 3 updates
- *(deps)* Bump the production-dependencies group with 4 updates
- *(deps)* Bump schemars in the serialization group
- *(deps)* Bump the production-dependencies group with 3 updates
- *(deps)* Bump k8s-openapi in the kubernetes-client group
- *(deps)* Bump k8s-openapi in the kubernetes-client group
- *(deps)* Bump the production-dependencies group across 1 directory with 9 updates

### Fex

- Fix faiing test

### Refac

- Add retention policy support
- Clean up code formatting and improve comments in finalizer, reconciler, resources, and CRD files

### Security

- Fix rustls-webpki vulnerability RUSTSEC-2026-0049



