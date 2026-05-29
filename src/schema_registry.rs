/// Advanced schema registry with versioning and compatibility (Issue #796)
///
/// Manages data schemas with version control, backward/forward compatibility
/// checking, schema evolution, documentation generation, discovery/search,
/// usage analytics, migration tools, governance/approval workflow, and a
/// client-library-style API.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use tracing::{info, warn};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ── Schema types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaFormat {
    JsonSchema,
    Avro,
    Protobuf,
    OpenApi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityMode {
    /// New schema must be readable by old consumers.
    Backward,
    /// Old schema must be readable by new consumers.
    Forward,
    /// Both backward and forward compatible.
    Full,
    /// No compatibility checks enforced.
    None,
}

impl Default for CompatibilityMode {
    fn default() -> Self {
        Self::Backward
    }
}

// ── Schema version ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub version: u32,
    pub schema_id: String,
    pub definition: serde_json::Value,
    pub format: SchemaFormat,
    pub description: String,
    pub author: String,
    pub created_at: u64,
    pub approval_status: ApprovalStatus,
    pub approved_by: Option<String>,
    pub tags: Vec<String>,
    pub deprecated: bool,
    pub migration_notes: Option<String>,
}

impl SchemaVersion {
    pub fn new(
        version: u32,
        schema_id: impl Into<String>,
        definition: serde_json::Value,
        format: SchemaFormat,
        author: impl Into<String>,
    ) -> Self {
        Self {
            version,
            schema_id: schema_id.into(),
            definition,
            format,
            description: String::new(),
            author: author.into(),
            created_at: now_secs(),
            approval_status: ApprovalStatus::Pending,
            approved_by: None,
            tags: vec![],
            deprecated: false,
            migration_notes: None,
        }
    }
}

// ── Schema subject ────────────────────────────────────────────────────────────

/// A named schema subject that holds all versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaSubject {
    pub name: String,
    pub namespace: String,
    pub compatibility: CompatibilityMode,
    pub versions: Vec<SchemaVersion>,
    pub usage_count: u64,
    pub last_used_at: Option<u64>,
    pub owners: Vec<String>,
}

impl SchemaSubject {
    pub fn new(name: impl Into<String>, namespace: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            namespace: namespace.into(),
            compatibility: CompatibilityMode::Backward,
            versions: vec![],
            usage_count: 0,
            last_used_at: None,
            owners: vec![],
        }
    }

    pub fn latest_approved(&self) -> Option<&SchemaVersion> {
        self.versions
            .iter()
            .rev()
            .find(|v| v.approval_status == ApprovalStatus::Approved && !v.deprecated)
    }

    pub fn get_version(&self, v: u32) -> Option<&SchemaVersion> {
        self.versions.iter().find(|sv| sv.version == v)
    }

    pub fn next_version_number(&self) -> u32 {
        self.versions.iter().map(|v| v.version).max().unwrap_or(0) + 1
    }
}

// ── Compatibility checker ─────────────────────────────────────────────────────

/// Result of a compatibility check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityResult {
    pub compatible: bool,
    pub mode: CompatibilityMode,
    pub issues: Vec<String>,
}

/// Perform a structural compatibility check between two JSON Schema definitions.
/// This is a simplified heuristic: checks that required fields in the old schema
/// are still present in the new schema (backward), and vice-versa (forward).
pub fn check_compatibility(
    old: &serde_json::Value,
    new: &serde_json::Value,
    mode: &CompatibilityMode,
) -> CompatibilityResult {
    let mut issues = Vec::new();

    let old_required = required_fields(old);
    let new_required = required_fields(new);
    let old_props = property_keys(old);
    let new_props = property_keys(new);

    match mode {
        CompatibilityMode::Backward => {
            // New schema must be able to read data written with old schema.
            // Old required fields must still exist in new schema.
            for field in &old_required {
                if !new_props.contains(field) {
                    issues.push(format!("Backward incompatible: field '{}' removed", field));
                }
            }
        }
        CompatibilityMode::Forward => {
            // Old schema must be able to read data written with new schema.
            // New required fields must exist in old schema.
            for field in &new_required {
                if !old_props.contains(field) {
                    issues.push(format!("Forward incompatible: new required field '{}' not in old schema", field));
                }
            }
        }
        CompatibilityMode::Full => {
            for field in &old_required {
                if !new_props.contains(field) {
                    issues.push(format!("Full incompatible: field '{}' removed", field));
                }
            }
            for field in &new_required {
                if !old_props.contains(field) {
                    issues.push(format!("Full incompatible: new required field '{}' not in old schema", field));
                }
            }
        }
        CompatibilityMode::None => {}
    }

    CompatibilityResult {
        compatible: issues.is_empty(),
        mode: mode.clone(),
        issues,
    }
}

fn required_fields(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn property_keys(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

// ── Migration tool ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub subject: String,
    pub from_version: u32,
    pub to_version: u32,
    pub steps: Vec<MigrationStep>,
    pub generated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationStep {
    pub description: String,
    pub field: String,
    pub action: MigrationAction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationAction {
    AddField { default_value: serde_json::Value },
    RemoveField,
    RenameField { new_name: String },
    ChangeType { new_type: String },
}

/// Generate a migration plan by diffing two schema versions.
pub fn generate_migration_plan(
    subject: &str,
    from: &SchemaVersion,
    to: &SchemaVersion,
) -> MigrationPlan {
    let old_props = property_keys(&from.definition);
    let new_props = property_keys(&to.definition);
    let mut steps = Vec::new();

    for field in &new_props {
        if !old_props.contains(field) {
            steps.push(MigrationStep {
                description: format!("Add new field '{}'", field),
                field: field.clone(),
                action: MigrationAction::AddField {
                    default_value: serde_json::Value::Null,
                },
            });
        }
    }
    for field in &old_props {
        if !new_props.contains(field) {
            steps.push(MigrationStep {
                description: format!("Remove field '{}'", field),
                field: field.clone(),
                action: MigrationAction::RemoveField,
            });
        }
    }

    MigrationPlan {
        subject: subject.to_string(),
        from_version: from.version,
        to_version: to.version,
        steps,
        generated_at: now_secs(),
    }
}

// ── Documentation generator ───────────────────────────────────────────────────

pub fn generate_docs(subject: &SchemaSubject) -> String {
    let mut doc = format!("# Schema: {}\n\nNamespace: `{}`\n\n", subject.name, subject.namespace);
    doc.push_str(&format!("Compatibility: `{:?}`\n\n", subject.compatibility));
    doc.push_str("## Versions\n\n");
    for v in &subject.versions {
        doc.push_str(&format!(
            "### v{} — {} ({})\n\n```json\n{}\n```\n\n",
            v.version,
            v.author,
            if v.deprecated { "deprecated" } else { "active" },
            serde_json::to_string_pretty(&v.definition).unwrap_or_default()
        ));
        if let Some(notes) = &v.migration_notes {
            doc.push_str(&format!("**Migration notes:** {}\n\n", notes));
        }
    }
    doc
}

// ── Registry ──────────────────────────────────────────────────────────────────

pub struct SchemaRegistry {
    subjects: HashMap<String, SchemaSubject>,
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self { subjects: HashMap::new() }
    }

    // ── Subject management ────────────────────────────────────────────────────

    pub fn create_subject(&mut self, subject: SchemaSubject) {
        info!("Creating schema subject '{}'", subject.name);
        self.subjects.entry(subject.name.clone()).or_insert(subject);
    }

    pub fn set_compatibility(&mut self, subject: &str, mode: CompatibilityMode) -> bool {
        if let Some(s) = self.subjects.get_mut(subject) {
            s.compatibility = mode;
            true
        } else {
            false
        }
    }

    // ── Version registration ──────────────────────────────────────────────────

    pub fn register_version(
        &mut self,
        subject_name: &str,
        definition: serde_json::Value,
        format: SchemaFormat,
        author: impl Into<String>,
    ) -> Result<u32, String> {
        let subject = self
            .subjects
            .get_mut(subject_name)
            .ok_or_else(|| format!("Subject '{}' not found", subject_name))?;

        // Compatibility check against latest approved version
        if let Some(latest) = subject.latest_approved() {
            let result = check_compatibility(&latest.definition, &definition, &subject.compatibility);
            if !result.compatible {
                return Err(format!(
                    "Schema incompatible: {}",
                    result.issues.join("; ")
                ));
            }
        }

        let version_num = subject.next_version_number();
        let schema_id = format!("{}-v{}", subject_name, version_num);
        let version = SchemaVersion::new(version_num, schema_id, definition, format, author);
        subject.versions.push(version);
        info!("Registered schema version {} for subject '{}'", version_num, subject_name);
        Ok(version_num)
    }

    // ── Approval workflow ─────────────────────────────────────────────────────

    pub fn approve_version(&mut self, subject: &str, version: u32, approver: impl Into<String>) -> bool {
        let approver = approver.into();
        if let Some(s) = self.subjects.get_mut(subject) {
            if let Some(v) = s.versions.iter_mut().find(|v| v.version == version) {
                v.approval_status = ApprovalStatus::Approved;
                v.approved_by = Some(approver.clone());
                info!("Schema {}/v{} approved by {}", subject, version, approver);
                return true;
            }
        }
        false
    }

    pub fn reject_version(&mut self, subject: &str, version: u32) -> bool {
        if let Some(s) = self.subjects.get_mut(subject) {
            if let Some(v) = s.versions.iter_mut().find(|v| v.version == version) {
                v.approval_status = ApprovalStatus::Rejected;
                warn!("Schema {}/v{} rejected", subject, version);
                return true;
            }
        }
        false
    }

    pub fn deprecate_version(&mut self, subject: &str, version: u32) -> bool {
        if let Some(s) = self.subjects.get_mut(subject) {
            if let Some(v) = s.versions.iter_mut().find(|v| v.version == version) {
                v.deprecated = true;
                return true;
            }
        }
        false
    }

    // ── Lookup ────────────────────────────────────────────────────────────────

    pub fn get_latest(&self, subject: &str) -> Option<&SchemaVersion> {
        self.subjects.get(subject)?.latest_approved()
    }

    pub fn get_version(&self, subject: &str, version: u32) -> Option<&SchemaVersion> {
        self.subjects.get(subject)?.get_version(version)
    }

    // ── Search / discovery ────────────────────────────────────────────────────

    pub fn search(&self, query: &str) -> Vec<&SchemaSubject> {
        let q = query.to_lowercase();
        self.subjects
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&q)
                    || s.namespace.to_lowercase().contains(&q)
                    || s.versions.iter().any(|v| {
                        v.tags.iter().any(|t| t.to_lowercase().contains(&q))
                            || v.description.to_lowercase().contains(&q)
                    })
            })
            .collect()
    }

    // ── Usage tracking ────────────────────────────────────────────────────────

    pub fn record_usage(&mut self, subject: &str) {
        if let Some(s) = self.subjects.get_mut(subject) {
            s.usage_count += 1;
            s.last_used_at = Some(now_secs());
        }
    }

    pub fn usage_analytics(&self) -> Vec<(&str, u64)> {
        let mut stats: Vec<(&str, u64)> = self
            .subjects
            .values()
            .map(|s| (s.name.as_str(), s.usage_count))
            .collect();
        stats.sort_by(|a, b| b.1.cmp(&a.1));
        stats
    }

    // ── Migration ─────────────────────────────────────────────────────────────

    pub fn migration_plan(&self, subject: &str, from: u32, to: u32) -> Option<MigrationPlan> {
        let s = self.subjects.get(subject)?;
        let from_v = s.get_version(from)?;
        let to_v = s.get_version(to)?;
        Some(generate_migration_plan(subject, from_v, to_v))
    }

    // ── Documentation ─────────────────────────────────────────────────────────

    pub fn docs(&self, subject: &str) -> Option<String> {
        self.subjects.get(subject).map(generate_docs)
    }
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedSchemaRegistry = Arc<RwLock<SchemaRegistry>>;

pub fn new_shared() -> SharedSchemaRegistry {
    Arc::new(RwLock::new(SchemaRegistry::new()))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn base_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["id", "name"],
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"}
            }
        })
    }

    fn extended_schema() -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "required": ["id", "name"],
            "properties": {
                "id": {"type": "string"},
                "name": {"type": "string"},
                "email": {"type": "string"}
            }
        })
    }

    fn breaking_schema() -> serde_json::Value {
        // Removes "name" which was required
        serde_json::json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "id": {"type": "string"}
            }
        })
    }

    fn make_registry() -> SchemaRegistry {
        let mut reg = SchemaRegistry::new();
        reg.create_subject(SchemaSubject::new("user-events", "stellar"));
        reg
    }

    #[test]
    fn test_register_and_approve() {
        let mut reg = make_registry();
        let v = reg
            .register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice")
            .unwrap();
        assert_eq!(v, 1);
        assert!(reg.approve_version("user-events", 1, "bob"));
        assert_eq!(
            reg.get_latest("user-events").unwrap().approval_status,
            ApprovalStatus::Approved
        );
    }

    #[test]
    fn test_backward_compatible_evolution() {
        let mut reg = make_registry();
        reg.register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        reg.approve_version("user-events", 1, "bob");
        // Adding optional field is backward compatible
        let v2 = reg.register_version("user-events", extended_schema(), SchemaFormat::JsonSchema, "alice");
        assert!(v2.is_ok());
    }

    #[test]
    fn test_backward_incompatible_rejected() {
        let mut reg = make_registry();
        reg.register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        reg.approve_version("user-events", 1, "bob");
        let result = reg.register_version("user-events", breaking_schema(), SchemaFormat::JsonSchema, "alice");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("incompatible"));
    }

    #[test]
    fn test_search() {
        let mut reg = make_registry();
        reg.create_subject(SchemaSubject::new("tx-events", "stellar"));
        let results = reg.search("user");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "user-events");
    }

    #[test]
    fn test_usage_analytics() {
        let mut reg = make_registry();
        reg.record_usage("user-events");
        reg.record_usage("user-events");
        let analytics = reg.usage_analytics();
        assert_eq!(analytics[0].0, "user-events");
        assert_eq!(analytics[0].1, 2);
    }

    #[test]
    fn test_migration_plan() {
        let mut reg = make_registry();
        reg.register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        reg.approve_version("user-events", 1, "bob");
        reg.register_version("user-events", extended_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        reg.approve_version("user-events", 2, "bob");

        let plan = reg.migration_plan("user-events", 1, 2).unwrap();
        assert_eq!(plan.from_version, 1);
        assert_eq!(plan.to_version, 2);
        assert!(!plan.steps.is_empty());
    }

    #[test]
    fn test_docs_generation() {
        let mut reg = make_registry();
        reg.register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        let docs = reg.docs("user-events").unwrap();
        assert!(docs.contains("# Schema: user-events"));
        assert!(docs.contains("v1"));
    }

    #[test]
    fn test_deprecate_version() {
        let mut reg = make_registry();
        reg.register_version("user-events", base_schema(), SchemaFormat::JsonSchema, "alice").unwrap();
        reg.approve_version("user-events", 1, "bob");
        assert!(reg.deprecate_version("user-events", 1));
        // latest_approved should now be None since only version is deprecated
        assert!(reg.get_latest("user-events").is_none());
    }

    #[test]
    fn test_compatibility_full_mode() {
        let result = check_compatibility(&base_schema(), &extended_schema(), &CompatibilityMode::Full);
        // extended adds optional field, doesn't remove required ones -> compatible
        assert!(result.compatible);
    }
}
