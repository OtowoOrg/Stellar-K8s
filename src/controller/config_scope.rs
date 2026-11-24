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
//! Structural-safety checks for the generated `stellar-core.cfg` document.
//!
//! # Why this exists
//!
//! `stellar-core.cfg` mixes *root* keys (`CATCHUP_COMPLETE`, `TLS_CERT_FILE`, ...)
//! with *table* sections such as `[QUORUM_SET]`, `[[VALIDATORS]]` and
//! `[[HOME_DOMAINS]]`. In TOML every key written after a table header belongs to
//! that table, so if the operator appends its own keys *after* a user-supplied
//! section, the file still parses but every operator key is silently captured by
//! the last table. The resulting validator then starts with mTLS disabled, a
//! stale catch-up mode and no `KNOWN_PEERS` — with no error anywhere.
//!
//! This module provides the two halves of the fix used by
//! [`crate::controller::resources::build_config_map`]:
//!
//! 1. [`root_scope_header`] renders the operator-managed keys, and callers must
//!    place it *before* any user content so it always lands at the root.
//! 2. [`inspect_config_scope`] re-parses the assembled document and proves the
//!    claim: it reports any operator key that ended up table-scoped, and any
//!    user key that the author clearly intended to be at the root but wrote
//!    after a table header.

use tracing::warn;

/// Root-level `stellar-core.cfg` keys that the operator writes on the user's
/// behalf.
///
/// These must be emitted before any user-supplied table section; if any of them
/// shows up nested under a table in the assembled document, the generated
/// config is structurally wrong.
pub const OPERATOR_ROOT_KEYS: &[&str] = &[
    "CATCHUP_COMPLETE",
    "CATCHUP_RECENT",
    "HTTP_PORT_SECURE",
    "KNOWN_PEERS",
    "TLS_CERT_FILE",
    "TLS_KEY_FILE",
];

/// Well-known `stellar-core.cfg` root keys that this operator never writes
/// itself but that a user may supply inside `spec.validatorConfig`.
///
/// This list is what makes the orphan check actionable rather than noisy: keys
/// that legitimately live inside a table (`THRESHOLD_PERCENT`, `ADDRESS`,
/// `TOML`, ...) are never reported, while a root key written after a table
/// header almost always is a mistake.
pub const KNOWN_STELLAR_CORE_ROOT_KEYS: &[&str] = &[
    "ARTIFICIALLY_CATCHING_UP",
    "AUTO_AUTH",
    "BUCKET_LIST_SIZE_LIMIT",
    "CATCHUP_COMPLETE",
    "CATCHUP_RECENT",
    "CATCHUP_SKIP",
    "CONFIGURE_DNS",
    "DATABASE",
    "DISABLE_AUTO_UPDATE",
    "DISABLE_HISTORICAL_LEDGER_CHECKPOINT_ELISION",
    "DISABLE_SCP",
    "DISABLE_XDR_DEBUG",
    "ENABLE_DEBUG_HTTP_ENDPOINTS",
    "ENABLE_OTEL",
    "HTTP_PORT",
    "HTTP_PORT_SECURE",
    "KNOWN_PEERS",
    "LEDGER_CLOSE_LATENCY_MS",
    "LEDGER_STATE_UPPER_BOUND",
    "LEDGER_VALIDITY_LEDGERS",
    "LOG_FILE_PATH",
    "LOG_LEVEL",
    "LOG_LINE_LIMIT",
    "MAX_BACK_HISTORY_OBJECTS",
    "MEMORY_LIMIT_MODE",
    "METADATA_SERVER_PORT",
    "METADATA_SERVER_URL",
    "METADATA_STREAM_CACHE_SIZE",
    "METADATA_STREAM_CACHE_UPDATE_PERIOD_MS",
    "MINIMUM_STATE_LEDGER",
    "MIN_TEMP_PEERS",
    "MODE",
    "NODE_NAMES",
    "NODE_SEED",
    "OBSERVING_PORT",
    "PEER_PORT",
    "PEER_PORT_SECURE",
    "PORT",
    "PREVENT_CRAPFALL",
    "PUBLIC_HTTPS_PORT",
    "PUBLIC_HTTP_PORT",
    "RESPONSE_OVERHEAD_MS",
    "SECRET_SEED",
    "SNAPSHOT_FILE",
    "SOURCE_DIR",
    "STORAGE_TYPE",
    "TLS_CERT_FILE",
    "TLS_KEY_FILE",
    "USE_CONFIG_TOML",
    "USE_HISTORICAL_CACHE",
    "USE_TOML_CFGS",
    "WORKER_THREADS",
];

/// True when `key` names a setting that `stellar-core` reads from the document
/// root.
pub fn is_known_root_key(key: &str) -> bool {
    OPERATOR_ROOT_KEYS.contains(&key) || KNOWN_STELLAR_CORE_ROOT_KEYS.contains(&key)
}

/// Marker comment inserted between the operator header and user content.
const USER_SECTION_MARKER: &str = "# ---- user-supplied configuration (below) ----";

/// Marker comment inserted between the user content and the generated network
/// defaults.
const GENERATED_SECTION_MARKER: &str = "# ---- generated network defaults (below) ----";

/// An operator key that ended up nested inside a table instead of the root.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MisplacedKey {
    /// The key that was captured by a table.
    pub key: String,
    /// Dotted path of the table that captured it, e.g. `VALIDATORS[0]`.
    pub scoped_under: String,
}

/// A key the user wrote after a table header, where TOML silently scopes it
/// into that table even though top-level placement was almost certainly meant.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrphanedRootKey {
    /// The key as written.
    pub key: String,
    /// 1-based line number in the document.
    pub line: usize,
    /// The table header that captured it, e.g. `VALIDATORS`.
    pub table: String,
}

/// Outcome of [`inspect_config_scope`].
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ConfigScopeReport {
    /// Every key that really lives at the document root, sorted.
    pub root_keys: Vec<String>,
    /// Operator keys that were captured by a table instead of the root.
    pub misplaced_operator_keys: Vec<MisplacedKey>,
    /// User keys written after a table header.
    pub orphaned_root_keys: Vec<OrphanedRootKey>,
    /// Table headers encountered while scanning, in document order.
    pub table_headers: Vec<String>,
}

impl ConfigScopeReport {
    /// True when the document has no structural problems.
    pub fn is_clean(&self) -> bool {
        self.misplaced_operator_keys.is_empty() && self.orphaned_root_keys.is_empty()
    }

    /// True when at least one of [`OPERATOR_ROOT_KEYS`] is at the document root.
    pub fn has_operator_keys(&self) -> bool {
        self.root_keys
            .iter()
            .any(|k| OPERATOR_ROOT_KEYS.contains(&k.as_str()))
    }

    /// One-line human summary suitable for logs and events.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!("root_keys={}", self.root_keys.len()));
        parts.push(format!("tables={}", self.table_headers.len()));
        parts.push(format!(
            "misplaced_operator_keys={}",
            self.misplaced_operator_keys.len()
        ));
        parts.push(format!(
            "orphaned_root_keys={}",
            self.orphaned_root_keys.len()
        ));
        parts.join(" ")
    }
}

/// Parts of the operator-managed root header for a validator.
#[derive(Clone, Debug, Default)]
pub struct OperatorHeader {
    /// Lines to emit above the user section, already newline-terminated.
    pub lines: Vec<String>,
}

impl OperatorHeader {
    /// Add a comment line to the header.
    pub fn comment(&mut self, text: impl AsRef<str>) -> &mut Self {
        self.lines.push(format!("# {}", text.as_ref()));
        self
    }

    /// Add a `KEY=VALUE` line to the header.
    pub fn key_value(&mut self, key: &str, value: &str) -> &mut Self {
        self.lines.push(format!("{key}={value}"));
        self
    }

    /// Add an already rendered assignment line to the header.
    ///
    /// Used for the generated network defaults, which are rendered by
    /// [`crate::controller::network_defaults`] and must land at the document
    /// root.
    pub fn assignment(&mut self, line: &str) -> &mut Self {
        self.lines.push(line.to_string());
        self
    }

    /// True when nothing would be emitted.
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Render the header, always newline-terminated when non-empty.
    pub fn render(&self) -> String {
        if self.lines.is_empty() {
            return String::new();
        }
        let mut out = self.lines.join("\n");
        out.push('\n');
        out
    }
}

/// True when `user_cfg` declares at least one TOML table header.
///
/// Headers of the form `[TABLE]`, `[[TABLE]]` and dotted `[a.b]` all capture
/// every subsequent bare key, so all of them are detected.
pub fn declares_table_sections(user_cfg: &str) -> bool {
    logical_lines(user_cfg)
        .iter()
        .any(|(_, line)| is_table_header(line.trim()))
}

/// Assemble a `stellar-core.cfg` from an operator header and user content.
///
/// The header is emitted first so its keys stay at the document root no matter
/// what the user content contains. `user_cfg` is emitted verbatim after a
/// marker comment so operators can still see where their settings stop and the
/// user's own settings begin.
pub fn assemble_config(header: &OperatorHeader, user_cfg: &str) -> String {
    assemble_config_with_generated(header, user_cfg, "")
}

/// Assemble a `stellar-core.cfg` from an operator header, user content and
/// generated network defaults.
///
/// The generated content is split in two:
///
/// * root keys belong to the header, so they are emitted before the user
///   content and stay at the document root;
/// * table sections are emitted after the user content, so the generated
///   `[[VALIDATORS]]` blocks cannot capture the user's root keys.
pub fn assemble_config_with_generated(
    header: &OperatorHeader,
    user_cfg: &str,
    generated: &str,
) -> String {
    let mut out = header.render();
    let user = user_cfg.trim_end_matches(['\n', '\r']);
    let generated = generated.trim_end_matches(['\n', '\r']);

    if !user.trim().is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(USER_SECTION_MARKER);
        out.push('\n');
        out.push_str(user);
        out.push('\n');
    }

    if !generated.trim().is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(GENERATED_SECTION_MARKER);
        out.push('\n');
        out.push_str(generated);
        out.push('\n');
    }

    out
}

/// Parse `cfg` and report how its keys are actually scoped.
///
/// Returns `Err` with a human-readable message when the document is not valid
/// TOML, which is itself a reportable operator error.
pub fn inspect_config_scope(cfg: &str) -> Result<ConfigScopeReport, String> {
    let value: toml::Value = cfg
        .parse()
        .map_err(|e| format!("generated stellar-core.cfg is not valid TOML: {e}"))?;

    let root = match &value {
        toml::Value::Table(t) => t,
        _ => return Err("generated stellar-core.cfg did not parse to a TOML table".to_string()),
    };

    let mut root_keys: Vec<String> = root.keys().cloned().collect();
    root_keys.sort();

    let mut misplaced_operator_keys = Vec::new();
    for key in root.keys() {
        if let Some(child) = root.get(key) {
            collect_misplaced_operator_keys(child, key, &mut misplaced_operator_keys);
        }
    }
    misplaced_operator_keys.sort_by(|a, b| {
        a.key
            .cmp(&b.key)
            .then_with(|| a.scoped_under.cmp(&b.scoped_under))
    });

    let (table_headers, orphaned_root_keys) = scan_table_scoped_assignments(cfg);

    Ok(ConfigScopeReport {
        root_keys,
        misplaced_operator_keys,
        orphaned_root_keys,
        table_headers,
    })
}

/// Log every structural problem found in a generated config.
///
/// Safe to call on every reconcile: a clean report logs at debug level only.
pub fn log_config_scope_findings(node: &str, cfg: &str) {
    match inspect_config_scope(cfg) {
        Ok(report) if report.is_clean() => {
            tracing::debug!(
                "config scope check ok for StellarNode {}: {}",
                node,
                report.summary()
            );
        }
        Ok(report) => {
            for misplaced in &report.misplaced_operator_keys {
                warn!(
                    "StellarNode {}: operator key {} was scoped into table [{}] instead of the \
                     stellar-core.cfg root; it will be ignored by stellar-core",
                    node, misplaced.key, misplaced.scoped_under
                );
            }
            for orphaned in &report.orphaned_root_keys {
                warn!(
                    "StellarNode {}: key {} on line {} is scoped into table [{}]; move it above \
                     the first [[TABLE]] header if it was meant to be a stellar-core.cfg root key",
                    node, orphaned.key, orphaned.line, orphaned.table
                );
            }
        }
        Err(message) => {
            warn!("StellarNode {}: {}", node, message);
        }
    }
}

/// Assert that every operator key present in `cfg` really sits at the root.
///
/// Returns the offending entries, so callers can turn them into a status
/// condition or a test failure.
pub fn misplaced_operator_keys(cfg: &str) -> Vec<MisplacedKey> {
    match inspect_config_scope(cfg) {
        Ok(report) => report.misplaced_operator_keys,
        Err(_) => Vec::new(),
    }
}

/// Recursively look for [`OPERATOR_ROOT_KEYS`] entries below the current table.
fn collect_misplaced_operator_keys(value: &toml::Value, path: &str, out: &mut Vec<MisplacedKey>) {
    match value {
        toml::Value::Table(table) => {
            for (key, child) in table {
                let child_path = format!("{path}.{key}");
                if OPERATOR_ROOT_KEYS.contains(&key.as_str()) {
                    out.push(MisplacedKey {
                        key: key.clone(),
                        scoped_under: path.to_string(),
                    });
                }
                collect_misplaced_operator_keys(child, &child_path, out);
            }
        }
        toml::Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                let indexed = format!("{path}[{index}]");
                if let toml::Value::Table(table) = item {
                    for (key, child) in table {
                        if OPERATOR_ROOT_KEYS.contains(&key.as_str()) {
                            out.push(MisplacedKey {
                                key: key.clone(),
                                scoped_under: indexed.clone(),
                            });
                        }
                        collect_misplaced_operator_keys(child, &indexed, out);
                    }
                } else {
                    collect_misplaced_operator_keys(item, &indexed, out);
                }
            }
        }
        _ => {}
    }
}

/// Scan the raw text for root keys that follow a table header.
///
/// TOML has no way to say "reset back to the root table", so a bare `KEY=value`
/// written after `[FOO]` is really `FOO.KEY`. Only keys that `stellar-core`
/// reads from the document root are reported, so legitimate table members such
/// as `THRESHOLD_PERCENT` or `ADDRESS` never trip the check.
fn scan_table_scoped_assignments(cfg: &str) -> (Vec<String>, Vec<OrphanedRootKey>) {
    let mut table_headers: Vec<String> = Vec::new();
    let mut orphaned: Vec<OrphanedRootKey> = Vec::new();
    let mut current_table: Option<String> = None;

    for (line_no, line) in logical_lines(cfg) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if is_table_header(trimmed) {
            let header = normalize_table_header(trimmed);
            table_headers.push(header.clone());
            current_table = Some(header);
            continue;
        }

        if let (Some(table), Some(key)) = (current_table.as_deref(), top_level_key(trimmed)) {
            if is_known_root_key(&key) {
                orphaned.push(OrphanedRootKey {
                    key,
                    line: line_no,
                    table: table.to_string(),
                });
            }
        }
    }

    orphaned.dedup();
    (table_headers, orphaned)
}

/// True for `[TABLE]`, `[[TABLE]]` and `[dotted.path]` header lines.
fn is_table_header(line: &str) -> bool {
    line.starts_with('[') && line.ends_with(']')
}

/// Strip the brackets from a table header so the name can be reported.
fn normalize_table_header(line: &str) -> String {
    line.trim_matches(|c| c == '[' || c == ']')
        .trim()
        .to_string()
}

/// Extract the bare key from a logical `KEY=value` line.
///
/// Returns `None` for continuation lines that carry no assignment, which keeps
/// multi-line arrays and inline tables from producing phantom findings.
fn top_level_key(line: &str) -> Option<String> {
    let (lhs, _rhs) = line.split_once('=')?;
    let key = lhs.trim();
    if key.is_empty() {
        return None;
    }
    Some(key.trim_matches(['"', '\'']).to_string())
}

/// Join physical lines into logical TOML statements.
///
/// Multi-line arrays, multi-line inline tables and multi-line basic strings are
/// folded into a single entry so that their continuation lines are never
/// mistaken for table headers or assignments. Returns `(1-based line number,
/// statement text)` pairs with comments dropped.
fn logical_lines(cfg: &str) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    let mut current = String::new();
    let mut start_line = 1usize;
    let mut line = 1usize;
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escaped = false;
    let mut started = false;

    for ch in cfg.chars() {
        if ch == '\n' {
            if depth == 0 && !in_string {
                if started {
                    out.push((start_line, current.trim_end().to_string()));
                }
                current.clear();
                start_line = line + 1;
                started = false;
            } else {
                current.push(ch);
            }
            line += 1;
            continue;
        }

        if started {
            current.push(ch);
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '#' if !started => {}
            '"' => {
                in_string = true;
                started = true;
            }
            '[' => {
                depth += 1;
                started = true;
            }
            ']' => {
                depth = depth.saturating_sub(1);
                started = true;
            }
            _ => {
                if !ch.is_whitespace() {
                    started = true;
                }
            }
        }
    }

    if started {
        out.push((start_line, current.trim_end().to_string()));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_WITH_TABLES: &str = r#"
[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["GCEZW7", "GCB2F5"]

[[VALIDATORS]]
HOME_DOMAINS=["validator1.example.com"]
NAME="validator1"
PUBLIC_KEY="GCEZW7"
ADDRESS="10.0.0.11"

[[HOME_DOMAINS]]
HOME_DOMAINS=["history.example.com"]
TOML=["./stellar-core_history.toml"]
""#;

    fn header_with_catchup() -> OperatorHeader {
        let mut header = OperatorHeader::default();
        header.comment("Full History Mode");
        header.key_value("CATCHUP_COMPLETE", "true");
        header
    }

    #[test]
    fn operator_keys_survive_table_bearing_user_config() {
        let assembled = assemble_config(&header_with_catchup(), USER_WITH_TABLES);
        let report = inspect_config_scope(&assembled).expect("assembled config must parse");

        assert!(report.misplaced_operator_keys.is_empty());
        assert!(report.root_keys.contains(&"CATCHUP_COMPLETE".to_string()));
        assert!(report.has_operator_keys());
        assert!(report.orphaned_root_keys.is_empty());
    }

    #[test]
    fn table_bearing_user_config_never_traps_operator_keys() {
        // The regression this module exists for: operator keys appended *after*
        // a user table header get captured by that table, and both detectors
        // have to say so.
        let broken = format!("{USER_WITH_TABLES}\nCATCHUP_COMPLETE=true\n");

        let report = inspect_config_scope(&broken).expect("config must still parse");

        // The key no longer sits at the document root...
        assert!(!report.root_keys.contains(&"CATCHUP_COMPLETE".to_string()));
        // ...it is captured by the last table, which is why both checks fire.
        assert_eq!(
            report.misplaced_operator_keys,
            vec![MisplacedKey {
                key: "CATCHUP_COMPLETE".to_string(),
                scoped_under: "HOME_DOMAINS[0]".to_string(),
            }]
        );
        assert!(report
            .orphaned_root_keys
            .iter()
            .any(|o| o.key == "CATCHUP_COMPLETE" && o.table == "HOME_DOMAINS"));
    }

    #[test]
    fn misplaced_operator_key_is_reported_with_its_table() {
        let broken = "CATCHUP_COMPLETE=true\n\n[QUORUM_SET]\nCATCHUP_RECENT=10\n";

        let misplaced = misplaced_operator_keys(broken);
        assert_eq!(
            misplaced,
            vec![MisplacedKey {
                key: "CATCHUP_RECENT".to_string(),
                scoped_under: "QUORUM_SET".to_string(),
            }]
        );
    }

    #[test]
    fn config_without_table_sections_is_clean() {
        let assembled = assemble_config(&header_with_catchup(), "NODE_SEED=\"SBRPTHQ\"");

        let report = inspect_config_scope(&assembled).expect("config must parse");
        assert!(report.is_clean());
        assert!(report.table_headers.is_empty());
        assert!(report.root_keys.contains(&"NODE_SEED".to_string()));
        assert!(report.root_keys.contains(&"CATCHUP_COMPLETE".to_string()));
    }

    #[test]
    fn detects_table_sections_in_user_config() {
        assert!(declares_table_sections(USER_WITH_TABLES));
        assert!(declares_table_sections(
            "[QUORUM_SET]\nTHRESHOLD_PERCENT=67\n"
        ));
        assert!(!declares_table_sections("KNOWN_PEERS=[\"10.0.0.1:11625\"]"));
        assert!(!declares_table_sections("# only a comment\n"));
        assert!(!declares_table_sections(""));
    }

    #[test]
    fn multi_line_arrays_do_not_produce_phantom_findings() {
        let user = "[QUORUM_SET]\nVALIDATORS=[\n  \"GCEZW7\",\n  \"GCB2F5\"\n]\n";

        let (headers, orphaned) = scan_table_scoped_assignments(user);
        assert_eq!(headers, vec!["QUORUM_SET".to_string()]);
        assert!(
            orphaned.is_empty(),
            "table members must not be reported as orphaned root keys: {orphaned:?}"
        );
    }

    #[test]
    fn multi_line_root_key_after_a_table_is_reported_once() {
        let user =
            "[[HOME_DOMAINS]]\nTOML=[\"./h.toml\"]\nKNOWN_PEERS=[\n  \"10.0.0.1:11625\",\n]\n";

        let (_, orphaned) = scan_table_scoped_assignments(user);
        assert_eq!(
            orphaned,
            vec![OrphanedRootKey {
                key: "KNOWN_PEERS".to_string(),
                line: 3,
                table: "HOME_DOMAINS".to_string(),
            }]
        );
    }

    #[test]
    fn nested_table_paths_are_reported_for_operator_keys() {
        let broken = "[[VALIDATORS]]\nHOME_DOMAINS=[\"a\"]\nTLS_CERT_FILE=\"/tmp/tls.crt\"\n";

        let misplaced = misplaced_operator_keys(broken);
        assert_eq!(misplaced.len(), 1);
        assert_eq!(misplaced[0].key, "TLS_CERT_FILE");
        assert_eq!(misplaced[0].scoped_under, "VALIDATORS[0]");
    }

    #[test]
    fn empty_header_and_user_config_yields_empty_document() {
        let assembled = assemble_config(&OperatorHeader::default(), "");
        assert!(assembled.is_empty());
        assert!(header_with_catchup().render().ends_with('\n'));
    }

    #[test]
    fn invalid_toml_is_reported_as_an_error() {
        let err = inspect_config_scope("this is = = not toml").unwrap_err();
        assert!(err.contains("not valid TOML"), "unexpected error: {err}");
    }

    #[test]
    fn summary_reports_counts() {
        let report = ConfigScopeReport {
            root_keys: vec!["CATCHUP_COMPLETE".to_string()],
            misplaced_operator_keys: vec![MisplacedKey {
                key: "TLS_CERT_FILE".to_string(),
                scoped_under: "QUORUM_SET".to_string(),
            }],
            orphaned_root_keys: vec![OrphanedRootKey {
                key: "KNOWN_PEERS".to_string(),
                line: 9,
                table: "VALIDATORS".to_string(),
            }],
            table_headers: vec!["VALIDATORS".to_string()],
        };

        assert!(!report.is_clean());
        let summary = report.summary();
        assert!(summary.contains("root_keys=1"));
        assert!(summary.contains("misplaced_operator_keys=1"));
        assert!(summary.contains("orphaned_root_keys=1"));
    }
}

/// Golden-file coverage for the generated `stellar-core.cfg` (#1560).
///
/// These pin the exact bytes the operator emits, not just "it parses". The
/// failure this issue is about is silent: a structurally valid document with
/// the operator keys swallowed by a user table. A byte-exact golden makes any
/// reordering of the header, or any change to the section marker, a test
/// failure instead of a quiet behaviour change.
mod golden {
    use super::*;
    use crate::crd::{HistoryMode, NodeType, StellarNode, StellarNodeSpec, ValidatorConfig};

    /// Quorum set with all three table shapes the issue calls out.
    const QUORUM_WITH_TABLES: &str = r#"[QUORUM_SET]
THRESHOLD_PERCENT=67
VALIDATORS=["GCEZW7", "GCB2F5"]

[[VALIDATORS]]
HOME_DOMAINS=["validator1.example.com"]
NAME="validator1"
PUBLIC_KEY="GCEZW7"
ADDRESS="validator1.stellar.example.com:11625"

[[HOME_DOMAINS]]
HOME_DOMAINS=["history.stellar.example.com"]
TOML=["./stellar-core_history.toml"]"#;

    /// User content that is root keys only, i.e. no table header to trap them.
    const ROOT_KEYS_ONLY: &str = r#"NODE_NAMES="validator1"
KNOWN_PEERS=["10.0.0.11:11625", "10.0.0.12:11625"]"#;

    fn validator(quorum_set: Option<&str>, history_mode: HistoryMode) -> StellarNode {
        let mut node = StellarNode::new(
            "golden-validator",
            StellarNodeSpec {
                node_type: NodeType::Validator,
                history_mode,
                ..Default::default()
            },
        );
        node.spec.validator_config = Some(ValidatorConfig {
            quorum_set: quorum_set.map(str::to_string),
            ..Default::default()
        });
        node
    }

    /// Render the config for `node` and compare it to `golden`, then assert the
    /// structural guarantees hold on the real output rather than on a fixture.
    fn assert_golden(node: &StellarNode, enable_mtls: bool, golden: &str) {
        let config_map = crate::controller::resources::build_config_map(node, None, enable_mtls);
        let generated = config_map
            .data
            .as_ref()
            .expect("config map data")
            .get("stellar-core.cfg")
            .expect("stellar-core.cfg must be generated for a validator")
            .clone();

        assert_eq!(
            generated.as_str(),
            golden,
            "generated stellar-core.cfg no longer matches its golden file"
        );

        let report = inspect_config_scope(&generated).expect("generated config must parse");
        assert!(
            report.is_clean(),
            "generated config has structural problems: {report:?}"
        );
        assert!(
            report.has_operator_keys(),
            "operator keys must survive into the document root: {report:?}"
        );
        assert!(
            report.misplaced_operator_keys.is_empty(),
            "no operator key may be captured by a table: {report:?}"
        );
    }

    /// Normalise line endings so a CRLF checkout cannot fail the comparison.
    fn lf(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    #[test]
    fn full_history_keeps_operator_keys_above_user_tables() {
        assert_golden(
            &validator(Some(QUORUM_WITH_TABLES), HistoryMode::Full),
            false,
            &lf(include_str!(
                "../../tests/fixtures/stellar_core_cfg/full_history_with_quorum_tables.cfg"
            )),
        );
    }

    #[test]
    fn recent_history_with_mtls_precedes_user_root_keys() {
        assert_golden(
            &validator(Some(ROOT_KEYS_ONLY), HistoryMode::Recent),
            true,
            &lf(include_str!(
                "../../tests/fixtures/stellar_core_cfg/recent_history_mtls_root_keys.cfg"
            )),
        );
    }

    #[test]
    fn operator_header_is_emitted_without_a_user_section_marker() {
        assert_golden(
            &validator(None, HistoryMode::Full),
            false,
            &lf(include_str!(
                "../../tests/fixtures/stellar_core_cfg/operator_header_only.cfg"
            )),
        );
    }

    #[test]
    fn every_operator_key_is_present_at_the_root_for_each_history_mode() {
        for (history_mode, expected) in [
            (HistoryMode::Full, "CATCHUP_COMPLETE=true"),
            (HistoryMode::Recent, "CATCHUP_RECENT=60480"),
        ] {
            let config_map = crate::controller::resources::build_config_map(
                &validator(Some(QUORUM_WITH_TABLES), history_mode),
                None,
                true,
            );
            let generated = config_map
                .data
                .as_ref()
                .expect("config map data")
                .get("stellar-core.cfg")
                .expect("stellar-core.cfg must be generated")
                .clone();

            // The first non-comment line must be the operator header, which is
            // what keeps every later operator key out of a user table.
            assert!(
                generated.lines().take_while(|l| l.starts_with('#')).count() >= 1,
                "operator header must lead the document: {generated}"
            );
            assert!(
                generated.contains(expected),
                "missing {expected} in generated config: {generated}"
            );
            assert!(
                generated.find(expected) < generated.find("[QUORUM_SET]").unwrap_or(usize::MAX)
            );

            let report = inspect_config_scope(&generated).expect("generated config must parse");
            assert!(report.is_clean(), "{report:?}");
            for key in OPERATOR_ROOT_KEYS {
                if generated.contains(&format!("{key}=")) {
                    assert!(
                        report.root_keys.contains(&key.to_string()),
                        "{key} must live at the document root: {report:?}"
                    );
                }
            }
        }
    }
}
