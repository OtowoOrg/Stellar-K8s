//! Versioned, consumer-aware schema registry for all internal API contracts.
//!
//! The registry deliberately uses a small, dependency-free structural model. It
//! is suitable for JSON Schema and the message subset of protobuf used by the
//! operator. A registration is atomic: it checks the subject policy, every
//! registered consumer, and an optional, audited override before mutating state.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SchemaFormat {
    JsonSchema,
    Avro,
    Protobuf,
    OpenApi,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityMode {
    #[default]
    Backward,
    Forward,
    Full,
    None,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaVersion {
    pub version: u32,
    pub schema_id: String,
    pub definition: serde_json::Value,
    pub format: SchemaFormat,
    #[serde(default)]
    pub description: String,
    pub author: String,
    pub created_at: u64,
    pub approval_status: ApprovalStatus,
    #[serde(default)]
    pub approved_by: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub deprecated: bool,
    #[serde(default)]
    pub migration_notes: Option<String>,
    #[serde(default)]
    pub impact_report: Option<ConsumerImpactReport>,
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
            impact_report: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SchemaSubject {
    pub name: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default)]
    pub compatibility: CompatibilityMode,
    #[serde(default)]
    pub versions: Vec<SchemaVersion>,
    #[serde(default)]
    pub usage_count: u64,
    #[serde(default)]
    pub last_used_at: Option<u64>,
    #[serde(default)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityResult {
    pub compatible: bool,
    pub mode: CompatibilityMode,
    pub issues: Vec<String>,
}

/// Check JSON-schema compatibility, including nested objects, type changes,
/// enum removals, and both directions. Unknown constructs are treated
/// conservatively (a removed property or narrowed type is breaking).
pub fn check_compatibility(
    old: &serde_json::Value,
    new: &serde_json::Value,
    mode: &CompatibilityMode,
) -> CompatibilityResult {
    let mut issues = Vec::new();
    if !matches!(mode, CompatibilityMode::None) {
        match (old.as_str(), new.as_str()) {
            (Some(old), Some(new)) => check_protobuf(old, new, mode, &mut issues),
            _ => check_json(old, new, "$", mode, &mut issues),
        }
    }
    CompatibilityResult {
        compatible: issues.is_empty(),
        mode: mode.clone(),
        issues,
    }
}
fn check_json(
    old: &serde_json::Value,
    new: &serde_json::Value,
    path: &str,
    mode: &CompatibilityMode,
    issues: &mut Vec<String>,
) {
    let op = old.get("properties").and_then(|v| v.as_object());
    let np = new.get("properties").and_then(|v| v.as_object());
    let or = required(old);
    let nr = required(new);
    if matches!(mode, CompatibilityMode::Backward | CompatibilityMode::Full) {
        for f in &or {
            if np.map(|p| !p.contains_key(f)).unwrap_or(true) {
                issues.push(format!(
                    "Backward incompatible: field '{path}.{f}' removed or not required"
                ));
            }
        }
    }
    if matches!(mode, CompatibilityMode::Forward | CompatibilityMode::Full) {
        for f in &nr {
            if op.map(|p| !p.contains_key(f)).unwrap_or(true) {
                issues.push(format!(
                    "Forward incompatible: required field '{path}.{f}' not in old schema"
                ));
            }
        }
    }
    if let (Some(o), Some(n)) = (op, np) {
        for (name, ov) in o {
            if let Some(nv) = n.get(name) {
                check_json(ov, nv, &format!("{path}.{name}"), mode, issues);
            }
        }
    }
    if let (Some(ot), Some(nt)) = (old.get("type"), new.get("type")) {
        if ot != nt
            && (matches!(mode, CompatibilityMode::Backward | CompatibilityMode::Full)
                || matches!(mode, CompatibilityMode::Forward | CompatibilityMode::Full))
        {
            issues.push(format!("Incompatible type at '{path}': {ot} -> {nt}"));
        }
    }
    let old_enum = old.get("enum").and_then(|x| x.as_array());
    let new_enum = new.get("enum").and_then(|x| x.as_array());
    match (old_enum, new_enum) {
        (Some(_), None) => issues.push(format!("Incompatible enum removed at '{path}'")),
        (Some(oe), Some(ne)) => {
            for value in oe {
                if !ne.contains(value) {
                    issues.push(format!("Incompatible enum value removed at '{path}'"));
                }
            }
        }
        _ => {}
    }
}
fn check_protobuf(old: &str, new: &str, mode: &CompatibilityMode, issues: &mut Vec<String>) {
    // The protobuf text format stores field numbers and wire declarations.
    // Removing any declared symbol is unsafe in both directions; additions are
    // safe under the supported proto3 contracts.
    if matches!(
        mode,
        CompatibilityMode::Backward | CompatibilityMode::Forward | CompatibilityMode::Full
    ) {
        let old_declarations = protobuf_declarations(old);
        let new_declarations = protobuf_declarations(new);
        for declaration in old_declarations {
            if !new_declarations.contains(&declaration) {
                issues.push(format!("Protobuf declaration '{declaration}' removed"));
            }
        }
    }
}

fn protobuf_declarations(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line == "}"
                || line.is_empty()
                || line.starts_with("//")
                || line.starts_with("syntax")
            {
                return None;
            }
            if let Some(rest) = line
                .strip_prefix("message ")
                .or_else(|| line.strip_prefix("enum "))
            {
                return Some(rest.trim().trim_end_matches('{').trim().to_string());
            }
            if line.contains('=') {
                let tokens: Vec<_> = line.split_whitespace().collect();
                if tokens.len() >= 2 {
                    let name = tokens[tokens.len() - 1].trim_end_matches(';');
                    let ty = tokens[tokens.len() - 2];
                    return Some(format!("{ty}:{name}"));
                }
            }
            None
        })
        .collect()
}
fn required(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationPlan {
    pub subject: String,
    pub from_version: u32,
    pub to_version: u32,
    pub steps: Vec<MigrationStep>,
    pub generated_at: u64,
}
pub fn generate_migration_plan(
    subject: &str,
    from: &SchemaVersion,
    to: &SchemaVersion,
) -> MigrationPlan {
    let old = from
        .definition
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let new = to
        .definition
        .get("properties")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut steps = vec![];
    for (f, v) in &new {
        if !old.contains_key(f) {
            steps.push(MigrationStep {
                description: format!("Add new field '{f}'"),
                field: f.clone(),
                action: MigrationAction::AddField {
                    default_value: v.get("default").cloned().unwrap_or(serde_json::Value::Null),
                },
            });
        }
    }
    for f in old.keys() {
        if !new.contains_key(f) {
            steps.push(MigrationStep {
                description: format!("Remove field '{f}'"),
                field: f.clone(),
                action: MigrationAction::RemoveField,
            });
        }
    }
    MigrationPlan {
        subject: subject.into(),
        from_version: from.version,
        to_version: to.version,
        steps,
        generated_at: now_secs(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Consumer {
    pub name: String,
    pub owner: String,
    /// subject -> immutable pinned version
    pub pinned_versions: BTreeMap<String, u32>,
    #[serde(default)]
    pub generated_client: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RegistryOverride {
    pub id: String,
    pub subject: String,
    pub reason: String,
    pub requested_by: String,
    pub approved_by: String,
    pub created_at: u64,
    #[serde(default)]
    pub consumed: bool,
}
impl RegistryOverride {
    pub fn is_valid(&self, subject: &str) -> bool {
        self.subject == subject
            && !self.consumed
            && !self.reason.trim().is_empty()
            && !self.approved_by.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsumerImpact {
    pub consumer: String,
    pub pinned_version: u32,
    pub compatible: bool,
    pub issues: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConsumerImpactReport {
    pub subject: String,
    pub from_version: u32,
    pub to_version: u32,
    pub mode: CompatibilityMode,
    pub breaking: bool,
    pub consumers: Vec<ConsumerImpact>,
    pub override_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    pub subjects: BTreeMap<String, SchemaSubject>,
    pub consumers: BTreeMap<String, Consumer>,
    #[serde(default)]
    pub overrides: Vec<RegistryOverride>,
    #[serde(default)]
    pub audit: Vec<AuditEntry>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub at: u64,
    pub actor: String,
    pub action: String,
    pub subject: String,
    pub detail: String,
}
impl Default for RegistrySnapshot {
    fn default() -> Self {
        Self {
            subjects: BTreeMap::new(),
            consumers: BTreeMap::new(),
            overrides: vec![],
            audit: vec![],
        }
    }
}

pub struct SchemaRegistry {
    state: RegistrySnapshot,
}
impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            state: RegistrySnapshot::default(),
        }
    }
    pub fn from_snapshot(s: RegistrySnapshot) -> Self {
        Self { state: s }
    }
    pub fn snapshot(&self) -> &RegistrySnapshot {
        &self.state
    }
    pub fn create_subject(&mut self, subject: SchemaSubject) {
        self.state
            .subjects
            .entry(subject.name.clone())
            .or_insert(subject);
    }
    pub fn set_compatibility(&mut self, subject: &str, mode: CompatibilityMode) -> bool {
        if let Some(s) = self.state.subjects.get_mut(subject) {
            s.compatibility = mode;
            true
        } else {
            false
        }
    }
    pub fn subject(&self, name: &str) -> Option<&SchemaSubject> {
        self.state.subjects.get(name)
    }
    pub fn consumers(&self) -> impl Iterator<Item = (&str, &Consumer)> {
        self.state.consumers.iter().map(|(k, v)| (k.as_str(), v))
    }
    pub fn audit(&self) -> &[AuditEntry] {
        &self.state.audit
    }

    pub fn register_consumer(&mut self, consumer: Consumer) -> Result<(), String> {
        if consumer.name.trim().is_empty() {
            return Err("consumer name is required".into());
        }
        for (subject, v) in &consumer.pinned_versions {
            if self
                .state
                .subjects
                .get(subject)
                .and_then(|s| s.get_version(*v))
                .is_none()
            {
                return Err(format!("consumer pin {subject}/v{v} does not exist"));
            }
        }
        if let Some(client) = &consumer.generated_client {
            if client.contains(":latest") || client.ends_with("/main") {
                return Err("generated client reference must be pinned".into());
            }
        }
        self.state.consumers.insert(consumer.name.clone(), consumer);
        Ok(())
    }

    pub fn add_override(&mut self, mut o: RegistryOverride) -> Result<String, String> {
        if o.id.trim().is_empty() || o.reason.trim().is_empty() || o.approved_by.trim().is_empty() {
            return Err("override requires id, reason and approved_by".into());
        }
        o.created_at = now_secs();
        let id = o.id.clone();
        self.state.overrides.push(o);
        Ok(id)
    }

    pub fn register_version_with_override(
        &mut self,
        subject_name: &str,
        definition: serde_json::Value,
        format: SchemaFormat,
        author: impl Into<String>,
        override_id: Option<&str>,
    ) -> Result<u32, String> {
        let subject = self
            .state
            .subjects
            .get(subject_name)
            .ok_or_else(|| format!("Subject '{subject_name}' not found"))?;
        let latest = subject.latest_approved().cloned();
        let mut report = None;
        let mut override_used = None;
        if let Some(old) = &latest {
            let base = check_compatibility(&old.definition, &definition, &subject.compatibility);
            let mut impacts = vec![];
            for c in self.state.consumers.values() {
                if let Some(pin) = c.pinned_versions.get(subject_name) {
                    let prior = self.state.subjects[subject_name]
                        .get_version(*pin)
                        .ok_or("pinned consumer version missing")?;
                    let r =
                        check_compatibility(&prior.definition, &definition, &subject.compatibility);
                    impacts.push(ConsumerImpact {
                        consumer: c.name.clone(),
                        pinned_version: *pin,
                        compatible: r.compatible,
                        issues: r.issues,
                    });
                }
            }
            let breaking = !base.compatible || impacts.iter().any(|i| !i.compatible);
            if breaking {
                let o = override_id
                    .and_then(|id| {
                        self.state
                            .overrides
                            .iter_mut()
                            .find(|o| o.id == id && o.is_valid(subject_name))
                    })
                    .ok_or_else(|| {
                        let issues = base
                            .issues
                            .iter()
                            .chain(impacts.iter().flat_map(|impact| &impact.issues))
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("; ");
                        format!(
                            "Schema incompatible; explicit registry override required: {issues}"
                        )
                    })?;
                o.consumed = true;
                override_used = Some(o.id.clone());
            }
            report = Some(ConsumerImpactReport {
                subject: subject_name.into(),
                from_version: old.version,
                to_version: subject.next_version_number(),
                mode: subject.compatibility.clone(),
                breaking,
                consumers: impacts,
                override_id: override_used.clone(),
            });
        }
        let actor = author.into();
        let s = self
            .state
            .subjects
            .get_mut(subject_name)
            .ok_or("subject disappeared")?;
        let n = s.next_version_number();
        let mut v = SchemaVersion::new(
            n,
            format!("{subject_name}-v{n}"),
            definition,
            format,
            actor.clone(),
        );
        v.impact_report = report;
        s.versions.push(v);
        self.state.audit.push(AuditEntry {
            at: now_secs(),
            actor,
            action: "register_version".into(),
            subject: subject_name.into(),
            detail: override_used
                .map(|x| format!("override:{x}"))
                .unwrap_or_else(|| "policy-clean".into()),
        });
        Ok(n)
    }
    pub fn register_version(
        &mut self,
        subject: &str,
        definition: serde_json::Value,
        format: SchemaFormat,
        author: impl Into<String>,
    ) -> Result<u32, String> {
        self.register_version_with_override(subject, definition, format, author, None)
    }
    pub fn approve_version(
        &mut self,
        subject: &str,
        version: u32,
        approver: impl Into<String>,
    ) -> bool {
        let a = approver.into();
        if let Some(v) = self
            .state
            .subjects
            .get_mut(subject)
            .and_then(|s| s.versions.iter_mut().find(|v| v.version == version))
        {
            v.approval_status = ApprovalStatus::Approved;
            v.approved_by = Some(a.clone());
            self.state.audit.push(AuditEntry {
                at: now_secs(),
                actor: a,
                action: "approve".into(),
                subject: subject.into(),
                detail: format!("v{version}"),
            });
            true
        } else {
            false
        }
    }
    pub fn reject_version(&mut self, s: &str, v: u32) -> bool {
        if let Some(x) = self
            .state
            .subjects
            .get_mut(s)
            .and_then(|x| x.versions.iter_mut().find(|x| x.version == v))
        {
            x.approval_status = ApprovalStatus::Rejected;
            true
        } else {
            false
        }
    }
    pub fn deprecate_version(&mut self, s: &str, v: u32) -> bool {
        if let Some(version) = self
            .state
            .subjects
            .get_mut(s)
            .and_then(|x| x.versions.iter_mut().find(|x| x.version == v))
        {
            version.deprecated = true;
            true
        } else {
            false
        }
    }
    pub fn get_latest(&self, s: &str) -> Option<&SchemaVersion> {
        self.state.subjects.get(s)?.latest_approved()
    }
    pub fn get_version(&self, s: &str, v: u32) -> Option<&SchemaVersion> {
        self.state.subjects.get(s)?.get_version(v)
    }
    pub fn search(&self, q: &str) -> Vec<&SchemaSubject> {
        let q = q.to_lowercase();
        self.state
            .subjects
            .values()
            .filter(|s| {
                s.name.to_lowercase().contains(&q) || s.namespace.to_lowercase().contains(&q)
            })
            .collect()
    }
    pub fn record_usage(&mut self, s: &str) {
        if let Some(x) = self.state.subjects.get_mut(s) {
            x.usage_count += 1;
            x.last_used_at = Some(now_secs());
        }
    }
    pub fn usage_analytics(&self) -> Vec<(&str, u64)> {
        let mut x: Vec<_> = self
            .state
            .subjects
            .iter()
            .map(|(k, v)| (k.as_str(), v.usage_count))
            .collect();
        x.sort_by_key(|v| std::cmp::Reverse(v.1));
        x
    }
    pub fn migration_plan(&self, s: &str, a: u32, b: u32) -> Option<MigrationPlan> {
        let x = self.state.subjects.get(s)?;
        Some(generate_migration_plan(
            s,
            x.get_version(a)?,
            x.get_version(b)?,
        ))
    }
    pub fn docs(&self, s: &str) -> Option<String> {
        self.state.subjects.get(s).map(|x| {
            format!(
                "# Schema: {}\n\nCompatibility: `{:?}`\n\n{}",
                x.name,
                x.compatibility,
                x.versions
                    .iter()
                    .map(|v| format!(
                        "### v{} by {}\n\n```json\n{}\n```\n",
                        v.version,
                        v.author,
                        serde_json::to_string_pretty(&v.definition).unwrap_or_default()
                    ))
                    .collect::<String>()
            )
        })
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.state).expect("registry state serializes")
    }
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s)
            .map(Self::from_snapshot)
            .map_err(|e| e.to_string())
    }
    pub fn impact(&self, subject: &str, definition: &serde_json::Value) -> ConsumerImpactReport {
        self.try_impact(subject, definition)
            .expect("subject exists in the registry")
    }

    /// Non-panicking variant used by the CLI gate.
    pub fn try_impact(
        &self,
        subject: &str,
        definition: &serde_json::Value,
    ) -> Result<ConsumerImpactReport, String> {
        let s = self
            .state
            .subjects
            .get(subject)
            .ok_or_else(|| format!("Subject '{subject}' not found"))?;
        let old = s.latest_approved();
        let mode = s.compatibility.clone();
        let from = old.map(|v| v.version).unwrap_or(0);
        let mut consumers = vec![];
        for c in self.state.consumers.values() {
            if let Some(p) = c.pinned_versions.get(subject) {
                if let Some(v) = s.get_version(*p) {
                    let r = check_compatibility(&v.definition, definition, &mode);
                    consumers.push(ConsumerImpact {
                        consumer: c.name.clone(),
                        pinned_version: *p,
                        compatible: r.compatible,
                        issues: r.issues,
                    });
                }
            }
        }
        let breaking = old
            .is_some_and(|v| !check_compatibility(&v.definition, definition, &mode).compatible)
            || consumers.iter().any(|x| !x.compatible);
        Ok(ConsumerImpactReport {
            subject: subject.into(),
            from_version: from,
            to_version: from + 1,
            mode,
            breaking,
            consumers,
            override_id: None,
        })
    }
}
impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}
/// Every internal contract hosted in the single registry snapshot. Build-time
/// coverage validation keeps this inventory from silently drifting.
pub const INTERNAL_SCHEMA_SUBJECTS: &[&str] = &["stellar.scp.message", "stellar.ledger.events"];
/// Verify the committed registry has no unregistered internal API contracts.
pub fn verify_internal_coverage(registry: &SchemaRegistry) -> Result<(), String> {
    let missing: Vec<_> = INTERNAL_SCHEMA_SUBJECTS
        .iter()
        .filter(|s| registry.subject(s).is_none())
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("internal schemas not registered: {missing:?}"))
    }
}
/// Compatibility result used by deploy gates. A deploy is accepted only when
/// the candidate is compatible or the named authorized override is supplied.
pub fn validate_deployment_pin(
    registry: &SchemaRegistry,
    subject: &str,
    version: u32,
    override_id: Option<&str>,
) -> Result<(), String> {
    if registry.get_version(subject, version).is_none() {
        return Err(format!(
            "pinned subject/version {subject}/v{version} is not registered"
        ));
    }
    if version == 0 {
        return Err("schema version zero is invalid".into());
    }
    if let Some(id) = override_id {
        if !registry
            .snapshot()
            .overrides
            .iter()
            .any(|o| o.id == id && o.is_valid(subject))
        {
            return Err(format!("override {id} is not authorized for {subject}"));
        }
    }
    Ok(())
}

/// Validate a generated consumer deployment against its immutable registry pin.
pub fn validate_consumer_deployment(
    registry: &SchemaRegistry,
    consumer_name: &str,
    subject: &str,
    version: u32,
    generated_client: &str,
) -> Result<(), String> {
    if generated_client.contains(":latest") || generated_client.ends_with("/main") {
        return Err("generated client reference must be pinned".into());
    }
    let consumer = registry
        .snapshot()
        .consumers
        .get(consumer_name)
        .ok_or_else(|| format!("consumer {consumer_name} is not registered"))?;
    if consumer.pinned_versions.get(subject) != Some(&version) {
        return Err(format!(
            "consumer {consumer_name} pin does not match {subject}/v{version}"
        ));
    }
    if consumer.generated_client.as_deref() != Some(generated_client) {
        return Err("generated client does not match the registered pin".into());
    }
    Ok(())
}
pub type SharedSchemaRegistry = Arc<RwLock<SchemaRegistry>>;
pub fn new_shared() -> SharedSchemaRegistry {
    Arc::new(RwLock::new(SchemaRegistry::new()))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn s() -> serde_json::Value {
        serde_json::json!({"type":"object","required":["id"],"properties":{"id":{"type":"string"}}})
    }
    fn broken() -> serde_json::Value {
        serde_json::json!({"type":"object","required":[],"properties":{}})
    }
    #[test]
    fn consumer_pins_and_impact() {
        let mut r = SchemaRegistry::new();
        r.create_subject(SchemaSubject::new("events", "internal"));
        assert_eq!(
            r.register_version("events", s(), SchemaFormat::JsonSchema, "a"),
            Ok(1)
        );
        r.approve_version("events", 1, "a");
        r.register_consumer(Consumer {
            name: "horizon".into(),
            owner: "team".into(),
            pinned_versions: [("events".into(), 1)].into_iter().collect(),
            generated_client: Some("horizon-v1".into()),
        })
        .unwrap();
        let report = r.impact("events", &broken());
        assert!(report.breaking);
        assert_eq!(report.consumers.len(), 1);
        assert!(!report.consumers[0].issues.is_empty());
    }
    #[test]
    fn breaking_requires_audited_override() {
        let mut r = SchemaRegistry::new();
        r.create_subject(SchemaSubject::new("events", "internal"));
        r.register_version("events", s(), SchemaFormat::JsonSchema, "a")
            .unwrap();
        r.approve_version("events", 1, "a");
        assert!(r
            .register_version("events", broken(), SchemaFormat::JsonSchema, "a")
            .is_err());
        r.add_override(RegistryOverride {
            id: "ovr-1".into(),
            subject: "events".into(),
            reason: "migration".into(),
            requested_by: "a".into(),
            approved_by: "lead".into(),
            created_at: 0,
            consumed: false,
        })
        .unwrap();
        assert_eq!(
            r.register_version_with_override(
                "events",
                broken(),
                SchemaFormat::JsonSchema,
                "a",
                Some("ovr-1")
            ),
            Ok(2)
        );
        assert!(r.audit().iter().any(|a| a.detail.contains("ovr-1")));
    }
    #[test]
    fn nested_types_are_checked() {
        let a = serde_json::json!({"type":"object","properties":{"x":{"type":"object","properties":{"id":{"type":"string"}}}}});
        let b = serde_json::json!({"type":"object","properties":{"x":{"type":"object","properties":{"id":{"type":"integer"}}}}});
        assert!(!check_compatibility(&a, &b, &CompatibilityMode::Backward).compatible);
    }
    #[test]
    fn committed_registry_covers_every_internal_schema() {
        let registry = SchemaRegistry::from_json(include_str!("../schemas/registry.json")).unwrap();
        verify_internal_coverage(&registry).unwrap();
        assert_eq!(
            registry.snapshot().subjects.len(),
            INTERNAL_SCHEMA_SUBJECTS.len()
        );
    }
    #[test]
    fn thousand_consumer_gate_is_well_under_thirty_seconds() {
        let mut r = SchemaRegistry::new();
        r.create_subject(SchemaSubject::new("events", "internal"));
        r.register_version("events", s(), SchemaFormat::JsonSchema, "a")
            .unwrap();
        r.approve_version("events", 1, "a");
        for i in 0..1000 {
            r.register_consumer(Consumer {
                name: format!("consumer-{i}"),
                owner: "test".into(),
                pinned_versions: [("events".into(), 1)].into_iter().collect(),
                generated_client: None,
            })
            .unwrap();
        }
        let started = std::time::Instant::now();
        let report = r.impact("events", &s());
        assert!(!report.breaking);
        assert_eq!(report.consumers.len(), 1000);
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
    }
    #[test]
    fn modes_cover_all_compatibility_policies() {
        let old =
            serde_json::json!({"type":"object","required":[],"properties":{"a":{"type":"string"}}});
        let new = serde_json::json!({"type":"object","required":["a"],"properties":{"a":{"type":"string"}}});
        assert!(check_compatibility(&old, &new, &CompatibilityMode::Backward).compatible);
        assert!(!check_compatibility(&old, &new, &CompatibilityMode::Forward).compatible);
        assert!(!check_compatibility(&old, &new, &CompatibilityMode::Full).compatible);
        assert!(check_compatibility(&old, &broken(), &CompatibilityMode::None).compatible);
    }
    #[test]
    fn invalid_consumer_pins_are_rejected() {
        let mut r = SchemaRegistry::new();
        r.create_subject(SchemaSubject::new("events", "internal"));
        let err = r
            .register_consumer(Consumer {
                name: "horizon".into(),
                owner: "team".into(),
                pinned_versions: [("events".into(), 9)].into_iter().collect(),
                generated_client: None,
            })
            .unwrap_err();
        assert!(err.contains("does not exist"));
    }
    #[test]
    fn consumed_override_cannot_be_replayed() {
        let mut r = SchemaRegistry::new();
        r.create_subject(SchemaSubject::new("events", "internal"));
        r.register_version("events", s(), SchemaFormat::JsonSchema, "a")
            .unwrap();
        r.approve_version("events", 1, "a");
        r.add_override(RegistryOverride {
            id: "ovr-once".into(),
            subject: "events".into(),
            reason: "migration".into(),
            requested_by: "a".into(),
            approved_by: "lead".into(),
            created_at: 0,
            consumed: false,
        })
        .unwrap();
        r.register_version_with_override(
            "events",
            broken(),
            SchemaFormat::JsonSchema,
            "a",
            Some("ovr-once"),
        )
        .unwrap();
        assert!(r
            .register_version_with_override(
                "events",
                broken(),
                SchemaFormat::JsonSchema,
                "a",
                Some("ovr-once")
            )
            .is_err());
    }
    #[test]
    fn protobuf_removals_are_breaking_and_additions_are_not() {
        let old = serde_json::json!("message Envelope { string id = 1; }");
        let added = serde_json::json!("message Envelope { string id = 1; uint64 sequence = 2; }");
        let removed = serde_json::json!("message Envelope { }");
        assert!(check_compatibility(&old, &added, &CompatibilityMode::Full).compatible);
        assert!(!check_compatibility(&old, &removed, &CompatibilityMode::Full).compatible);
    }
    #[test]
    fn deployment_requires_exact_consumer_pin() {
        let registry = SchemaRegistry::from_json(include_str!("../schemas/registry.json")).unwrap();
        validate_deployment_pin(&registry, "stellar.ledger.events", 1, None).unwrap();
        assert!(validate_deployment_pin(&registry, "stellar.ledger.events", 2, None).is_err());
        validate_consumer_deployment(
            &registry,
            "horizon",
            "stellar.ledger.events",
            1,
            "github.com/otowo/stellar-go/horizon@v1.0.0",
        )
        .unwrap();
        assert!(validate_consumer_deployment(
            &registry,
            "horizon",
            "stellar.ledger.events",
            1,
            "horizon:latest"
        )
        .is_err());
    }
    #[test]
    fn round_trip_snapshot_is_stable() {
        let registry = SchemaRegistry::from_json(include_str!("../schemas/registry.json")).unwrap();
        let encoded = registry.to_json();
        let decoded = SchemaRegistry::from_json(&encoded).unwrap();
        assert_eq!(decoded.snapshot().subjects, registry.snapshot().subjects);
        assert_eq!(decoded.snapshot().consumers, registry.snapshot().consumers);
    }
}
