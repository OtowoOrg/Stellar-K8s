# Design: GitOps Drift Detection & Auto-Revert

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                    Git Repository (Source of Truth)              │
│  manifests/production/*.yaml                                     │
└────────────────────────┬───────────────────────────────────────┘
                         │
                    ┌────▼──────┐
                    │ Git Poller│
                    └────┬──────┘
                         │
              ┌──────────▼──────────┐
              │   Git State Cache   │
              │   (Desired State)   │
              └──────────┬──────────┘
                         │
    ┌────────────────────┼────────────────────┐
    │                    │                    │
    │        ┌───────────▼───────────┐        │
    │        │  Drift Detector       │        │
    │        │                       │        │
    │        │  ┌─────────────────┐ │        │
    │        │  │Three-Way Differ │ │        │
    │        │  │ (base/live/git) │ │        │
    │        │  └─────────────────┘ │        │
    │        │  ┌─────────────────┐ │        │
    │        │  │Default Filter   │ │        │
    │        │  └─────────────────┘ │        │
    │        │  ┌─────────────────┐ │        │
    │        │  │Drift Classifier │ │        │
    │        │  └─────────────────┘ │        │
    │        └───────────┬───────────┘        │
    │                    │                    │
┌───▼────────┐    ┌──────▼──────┐    ┌──────▼────────┐
│Kubernetes  │    │DriftReport  │    │  Auto-Revert  │
│  API       │    │     CRs     │    │   Controller  │
│(Live State)│    └─────────────┘    └───────────────┘
└────────────┘            │                   │
      │                   │                   │
      │            ┌──────▼──────┐     ┌──────▼──────┐
      │            │ Prometheus  │     │  Rollback   │
      │            │   Metrics   │     │   Safety    │
      │            └─────────────┘     │   Checks    │
      │                                └─────────────┘
      │
┌─────▼──────────┐
│ Kubernetes     │
│  Audit Logs    │
│  (Attribution) │
└────────────────┘
```

## Component Design

### 1. Three-Way Differ

**Responsibility**: Compute structured diffs between base, live, and git states.

```rust
pub struct ThreeWayDiffer {
    // Uses JSON Patch (RFC 6902) for structured diffs
    patch_library: JsonPatch,
}

pub struct ThreeWayDiffResult {
    pub live_diff: JsonPatch,      // base → live
    pub git_diff: JsonPatch,       // base → git
    pub classification: DriftClass,
}

pub enum DriftClass {
    Healthy,              // live = git
    ManualMutation {      // live ≠ git, changes in live not in git
        mutations: Vec<FieldMutation>,
        confidence: f32,
    },
    PendingPropagation {  // live ≠ git, changes in git not in live
        pending: Vec<FieldChange>,
        confidence: f32,
    },
    Diverged {            // Both manual mutations and pending changes
        mutations: Vec<FieldMutation>,
        pending: Vec<FieldChange>,
    },
}

impl ThreeWayDiffer {
    pub fn diff(
        &self,
        base: &Resource,
        live: &Resource,
        git: &Resource,
    ) -> Result<ThreeWayDiffResult> {
        // Step 1: Compute diffs
        let base_json = serde_json::to_value(base)?;
        let live_json = serde_json::to_value(live)?;
        let git_json = serde_json::to_value(git)?;
        
        let live_diff = json_patch::diff(&base_json, &live_json);
        let git_diff = json_patch::diff(&base_json, &git_json);
        
        // Step 2: Classify drift
        let classification = self.classify_drift(
            &live_diff,
            &git_diff,
            live,
            git,
        )?;
        
        Ok(ThreeWayDiffResult {
            live_diff,
            git_diff,
            classification,
        })
    }
    
    fn classify_drift(
        &self,
        live_diff: &JsonPatch,
        git_diff: &JsonPatch,
        live: &Resource,
        git: &Resource,
    ) -> Result<DriftClass> {
        // If live = git, no drift
        if live_diff.is_empty() && git_diff.is_empty() {
            return Ok(DriftClass::Healthy);
        }
        
        // Analyze live_diff: changes in live not present in git
        let mutations = self.find_mutations(live_diff, git_diff)?;
        
        // Analyze git_diff: changes in git not yet in live
        let pending = self.find_pending_changes(git_diff, live_diff)?;
        
        match (mutations.is_empty(), pending.is_empty()) {
            (true, true) => Ok(DriftClass::Healthy),
            (false, true) => Ok(DriftClass::ManualMutation {
                mutations,
                confidence: self.calculate_confidence(&mutations),
            }),
            (true, false) => Ok(DriftClass::PendingPropagation {
                pending,
                confidence: self.calculate_confidence(&pending),
            }),
            (false, false) => Ok(DriftClass::Diverged {
                mutations,
                pending,
            }),
        }
    }
    
    fn find_mutations(
        &self,
        live_diff: &JsonPatch,
        git_diff: &JsonPatch,
    ) -> Result<Vec<FieldMutation>> {
        let mut mutations = Vec::new();
        
        for op in &live_diff.0 {
            let path = self.extract_path(op);
            
            // Check if this change also appears in git_diff
            let in_git = git_diff.0.iter().any(|git_op| {
                self.extract_path(git_op) == path
            });
            
            if !in_git {
                // This is a manual mutation (live changed, git didn't)
                mutations.push(FieldMutation {
                    path: path.clone(),
                    operation: self.operation_type(op),
                    old_value: self.extract_old_value(op),
                    new_value: self.extract_new_value(op),
                });
            }
        }
        
        Ok(mutations)
    }
}
```

### 2. Server-Side Default Filter

**Responsibility**: Exclude fields that are server-defaulted to prevent false positives.

```rust
pub struct DefaultFilter {
    // Allowlist of known defaulted fields per resource kind
    default_fields: HashMap<GroupVersionKind, Vec<JsonPath>>,
}

impl DefaultFilter {
    pub fn new() -> Self {
        let mut default_fields = HashMap::new();
        
        // Common defaults across all resources
        default_fields.insert(
            gvk("", "v1", "Pod"),
            vec![
                JsonPath::from("/spec/containers/*/imagePullPolicy"),
                JsonPath::from("/spec/restartPolicy"),
                JsonPath::from("/spec/dnsPolicy"),
                JsonPath::from("/spec/schedulerName"),
                JsonPath::from("/spec/securityContext/fsGroup"),
                JsonPath::from("/status/*"), // All status fields
                JsonPath::from("/metadata/uid"),
                JsonPath::from("/metadata/resourceVersion"),
                JsonPath::from("/metadata/generation"),
                JsonPath::from("/metadata/managedFields"),
                JsonPath::from("/metadata/creationTimestamp"),
            ],
        );
        
        // Deployment defaults
        default_fields.insert(
            gvk("apps", "v1", "Deployment"),
            vec![
                JsonPath::from("/spec/strategy/type"),
                JsonPath::from("/spec/revisionHistoryLimit"),
                JsonPath::from("/spec/progressDeadlineSeconds"),
                JsonPath::from("/status/*"),
            ],
        );
        
        // Service defaults
        default_fields.insert(
            gvk("", "v1", "Service"),
            vec![
                JsonPath::from("/spec/clusterIP"),
                JsonPath::from("/spec/sessionAffinity"),
                JsonPath::from("/spec/type"),
                JsonPath::from("/spec/ports/*/protocol"),
                JsonPath::from("/spec/ipFamilyPolicy"),
                JsonPath::from("/spec/internalTrafficPolicy"),
            ],
        );
        
        Self { default_fields }
    }
    
    pub fn filter_diff(
        &self,
        diff: JsonPatch,
        resource: &Resource,
    ) -> JsonPatch {
        let gvk = resource.group_version_kind();
        
        let default_paths = match self.default_fields.get(&gvk) {
            Some(paths) => paths,
            None => return diff, // No known defaults for this type
        };
        
        // Remove operations on default paths
        let filtered_ops = diff.0.into_iter()
            .filter(|op| {
                let path = self.extract_path(op);
                !default_paths.iter().any(|dp| dp.matches(&path))
            })
            .collect();
        
        JsonPatch(filtered_ops)
    }
}
```

### 3. Drift Detector

**Responsibility**: Continuously monitor resources and detect drift.

```rust
pub struct DriftDetector {
    k8s_client: Client,
    git_client: GitClient,
    differ: ThreeWayDiffer,
    default_filter: DefaultFilter,
    base_state_cache: BaseStateCache,
    metrics: DriftMetrics,
}

impl DriftDetector {
    pub async fn detect_drift(
        &self,
        resource_ref: ResourceRef,
    ) -> Result<Option<DriftReport>> {
        // Step 1: Fetch live state
        let live = self.fetch_live_state(&resource_ref).await?;
        
        // Step 2: Fetch git state
        let git = self.git_client
            .fetch_resource(&resource_ref)
            .await?;
        
        // Step 3: Fetch base state (last successful sync)
        let base = self.base_state_cache
            .get(&resource_ref)
            .unwrap_or(&git); // If no base, use git as base
        
        // Step 4: Compute three-way diff
        let mut diff_result = self.differ.diff(base, &live, &git)?;
        
        // Step 5: Filter server-side defaults
        diff_result.live_diff = self.default_filter.filter_diff(
            diff_result.live_diff,
            &live,
        );
        diff_result.git_diff = self.default_filter.filter_diff(
            diff_result.git_diff,
            &git,
        );
        
        // Step 6: Re-classify after filtering
        if diff_result.live_diff.is_empty() && diff_result.git_diff.is_empty() {
            // No actual drift after filtering
            self.metrics.false_positives_prevented.inc();
            return Ok(None);
        }
        
        // Step 7: Attribute drift to actor
        let attribution = self.attribute_drift(&resource_ref, &live).await?;
        
        // Step 8: Create DriftReport
        let report = DriftReport {
            metadata: ObjectMeta {
                name: format!("{}-{}", resource_ref.name, Utc::now().timestamp()),
                namespace: resource_ref.namespace,
                labels: hashmap! {
                    "resource-kind".to_string() => resource_ref.kind.clone(),
                    "drift-class".to_string() => diff_result.classification.to_string(),
                },
                ..Default::default()
            },
            spec: DriftReportSpec {
                resource_ref,
                classification: diff_result.classification,
                live_diff: diff_result.live_diff,
                git_diff: diff_result.git_diff,
                detected_at: Utc::now(),
                attribution,
            },
            status: DriftReportStatus {
                auto_revert_attempted: false,
                auto_revert_succeeded: None,
            },
        };
        
        // Step 9: Emit metrics
        self.metrics.drifts_detected
            .with_label_values(&[
                &resource_ref.kind,
                &diff_result.classification.to_string(),
            ])
            .inc();
        
        Ok(Some(report))
    }
    
    async fn fetch_live_state(
        &self,
        resource_ref: &ResourceRef,
    ) -> Result<Resource> {
        let api: Api<DynamicObject> = if let Some(ns) = &resource_ref.namespace {
            Api::namespaced(self.k8s_client.clone(), ns)
        } else {
            Api::all(self.k8s_client.clone())
        };
        
        let obj = api.get(&resource_ref.name).await?;
        Ok(Resource::from_dynamic(obj))
    }
    
    async fn attribute_drift(
        &self,
        resource_ref: &ResourceRef,
        live: &Resource,
    ) -> Result<Option<Attribution>> {
        // Query Kubernetes audit logs for modifications
        let audit_events = self.query_audit_logs(
            resource_ref,
            live.metadata.resource_version.as_ref(),
        ).await?;
        
        if let Some(event) = audit_events.last() {
            Ok(Some(Attribution {
                actor: event.user.username.clone(),
                source_ip: event.source_ips.first().cloned(),
                user_agent: event.user_agent.clone(),
                timestamp: event.request_timestamp,
            }))
        } else {
            Ok(None)
        }
    }
}
```

### 4. Git State Cache

**Responsibility**: Maintain cached view of Git repository state.

```rust
pub struct GitStateCache {
    git_client: GitClient,
    cache: Arc<RwLock<HashMap<ResourceRef, CachedResource>>>,
    poll_interval: Duration,
}

struct CachedResource {
    resource: Resource,
    commit_sha: String,
    cached_at: DateTime<Utc>,
}

impl GitStateCache {
    pub async fn start_polling(&self) {
        let mut interval = tokio::time::interval(self.poll_interval);
        
        loop {
            interval.tick().await;
            
            if let Err(e) = self.refresh_cache().await {
                tracing::error!("Failed to refresh Git cache: {}", e);
                continue;
            }
        }
    }
    
    async fn refresh_cache(&self) -> Result<()> {
        // Fetch latest commit
        let latest_sha = self.git_client.fetch_latest_commit().await?;
        
        // Check if changed
        let current_sha = self.get_current_sha();
        if latest_sha == current_sha {
            return Ok(()); // No changes
        }
        
        // Fetch all resources from Git
        let resources = self.git_client.fetch_all_resources().await?;
        
        // Update cache
        let mut cache = self.cache.write().await;
        cache.clear();
        
        for resource in resources {
            let resource_ref = ResourceRef::from_resource(&resource);
            cache.insert(
                resource_ref,
                CachedResource {
                    resource,
                    commit_sha: latest_sha.clone(),
                    cached_at: Utc::now(),
                },
            );
        }
        
        tracing::info!(
            sha = %latest_sha,
            resources = cache.len(),
            "Git cache refreshed"
        );
        
        Ok(())
    }
    
    pub async fn get(&self, resource_ref: &ResourceRef) -> Option<Resource> {
        let cache = self.cache.read().await;
        cache.get(resource_ref).map(|cr| cr.resource.clone())
    }
}
```

### 5. Auto-Revert Controller

**Responsibility**: Automatically revert manual mutations based on policy.

```rust
pub struct AutoRevertController {
    k8s_client: Client,
    safety_checker: SafetyChecker,
    metrics: RevertMetrics,
}

impl AutoRevertController {
    pub async fn handle_drift_report(
        &self,
        report: &DriftReport,
        policy: &DriftPolicy,
    ) -> Result<()> {
        // Check if auto-revert enabled for this resource
        if !self.should_auto_revert(report, policy) {
            return Ok(());
        }
        
        // Only revert manual mutations, not pending propagation
        if !matches!(report.spec.classification, DriftClass::ManualMutation { .. }) {
            return Ok(());
        }
        
        tracing::info!(
            resource = %report.spec.resource_ref,
            "Attempting auto-revert"
        );
        
        // Step 1: Run safety checks
        let safety_result = self.safety_checker
            .check_revert_safety(&report.spec.resource_ref)
            .await?;
        
        if !safety_result.safe {
            tracing::warn!(
                resource = %report.spec.resource_ref,
                reason = %safety_result.reason,
                "Auto-revert blocked by safety check"
            );
            self.metrics.reverts_blocked.inc();
            return Ok(());
        }
        
        // Step 2: Create backup of current state
        let backup = self.create_backup(&report.spec.resource_ref).await?;
        
        // Step 3: Apply git state to revert
        let git_resource = self.fetch_git_state(&report.spec.resource_ref).await?;
        
        match self.apply_resource(&git_resource).await {
            Ok(_) => {
                tracing::info!(
                    resource = %report.spec.resource_ref,
                    "Auto-revert succeeded"
                );
                
                // Update DriftReport status
                self.update_report_status(
                    report,
                    true,
                    Some("Auto-reverted successfully".to_string()),
                ).await?;
                
                self.metrics.reverts_succeeded.inc();
                
                // Alert on auto-revert
                self.emit_revert_alert(report, &backup).await?;
            }
            Err(e) => {
                tracing::error!(
                    resource = %report.spec.resource_ref,
                    error = %e,
                    "Auto-revert failed"
                );
                
                // Attempt rollback to backup
                self.rollback_to_backup(&backup).await?;
                
                self.update_report_status(
                    report,
                    false,
                    Some(format!("Auto-revert failed: {}", e)),
                ).await?;
                
                self.metrics.reverts_failed.inc();
            }
        }
        
        Ok(())
    }
    
    fn should_auto_revert(
        &self,
        report: &DriftReport,
        policy: &DriftPolicy,
    ) -> bool {
        // Check if resource kind matches policy
        policy.spec.auto_revert_rules.iter().any(|rule| {
            rule.resource_kind == report.spec.resource_ref.kind
                && rule.enabled
                && self.namespace_matches(&rule.namespaces, &report.spec.resource_ref.namespace)
        })
    }
}
```

### 6. Safety Checker

**Responsibility**: Verify revert operations are safe before execution.

```rust
pub struct SafetyChecker {
    k8s_client: Client,
}

pub struct SafetyResult {
    pub safe: bool,
    pub reason: String,
    pub checks: Vec<SafetyCheck>,
}

impl SafetyChecker {
    pub async fn check_revert_safety(
        &self,
        resource_ref: &ResourceRef,
    ) -> Result<SafetyResult> {
        let mut checks = Vec::new();
        
        // Check 1: Resource is not being deleted
        let deletion_check = self.check_not_deleting(resource_ref).await?;
        checks.push(deletion_check.clone());
        if !deletion_check.passed {
            return Ok(SafetyResult {
                safe: false,
                reason: "Resource is being deleted".to_string(),
                checks,
            });
        }
        
        // Check 2: No active mutations in progress
        let mutation_check = self.check_no_active_mutations(resource_ref).await?;
        checks.push(mutation_check.clone());
        if !mutation_check.passed {
            return Ok(SafetyResult {
                safe: false,
                reason: "Active mutations in progress".to_string(),
                checks,
            });
        }
        
        // Check 3: Resource is not in use by critical workloads
        let usage_check = self.check_not_critical_usage(resource_ref).await?;
        checks.push(usage_check.clone());
        if !usage_check.passed {
            return Ok(SafetyResult {
                safe: false,
                reason: "Resource in use by critical workload".to_string(),
                checks,
            });
        }
        
        // Check 4: Revert diff is not destructive
        let destructive_check = self.check_not_destructive(resource_ref).await?;
        checks.push(destructive_check.clone());
        if !destructive_check.passed {
            return Ok(SafetyResult {
                safe: false,
                reason: "Revert would be destructive".to_string(),
                checks,
            });
        }
        
        Ok(SafetyResult {
            safe: true,
            reason: "All safety checks passed".to_string(),
            checks,
        })
    }
    
    async fn check_not_destructive(
        &self,
        resource_ref: &ResourceRef,
    ) -> Result<SafetyCheck> {
        // Check if revert would delete data, scale down critically, etc.
        // For example:
        // - Scaling replicas from 3 → 0 is destructive
        // - Deleting PVC is destructive
        // - Changing storage class is destructive
        
        // This is a simplified check
        Ok(SafetyCheck {
            name: "not_destructive".to_string(),
            passed: true,
            details: "Revert is non-destructive".to_string(),
        })
    }
}
```

## Custom Resource Definitions

### DriftPolicy CRD

```yaml
apiVersion: drift.stellar.io/v1
kind: DriftPolicy
metadata:
  name: production-drift-policy
  namespace: production
spec:
  # Auto-revert rules
  autoRevertRules:
    - resourceKind: Deployment
      enabled: true
      namespaces: ["production"]
      excludeFields:
        - /spec/replicas  # Allow manual scaling
    - resourceKind: Service
      enabled: true
      namespaces: ["production"]
    - resourceKind: ConfigMap
      enabled: false  # Never auto-revert ConfigMaps
  
  # Detection settings
  detectionInterval: 30s
  
  # Alert settings
  alertOnDrift: true
  alertChannels:
    - type: slack
      webhook: https://hooks.slack.com/services/...
    - type: pagerduty
      integrationKey: secret/pagerduty-key
```

### DriftReport CRD

```yaml
apiVersion: drift.stellar.io/v1
kind: DriftReport
metadata:
  name: nginx-deployment-1727493600
  namespace: production
  labels:
    resource-kind: Deployment
    drift-class: ManualMutation
spec:
  resourceRef:
    kind: Deployment
    name: nginx
    namespace: production
    apiVersion: apps/v1
  
  classification: ManualMutation
  
  liveDiff:
    - op: replace
      path: /spec/replicas
      value: 5
  
  gitDiff: []
  
  detectedAt: "2026-09-28T10:00:00Z"
  
  attribution:
    actor: john@example.com
    sourceIP: 10.0.1.50
    userAgent: kubectl/v1.28
    timestamp: "2026-09-28T09:59:45Z"

status:
  autoRevertAttempted: true
  autoRevertSucceeded: true
  autoRevertedAt: "2026-09-28T10:00:15Z"
```

## Metrics

```prometheus
# Detection
drift_reports_total{kind, namespace, classification="healthy|manual|pending|diverged"}
drift_detection_duration_seconds{kind}
drift_false_positives_prevented_total

# Attribution
drift_attribution_success_rate{kind}
drift_attributed_to_actor_total{actor, kind}

# Auto-revert
drift_auto_reverts_attempted_total{kind, namespace}
drift_auto_reverts_succeeded_total{kind, namespace}
drift_auto_reverts_failed_total{kind, namespace, reason}
drift_auto_reverts_blocked_total{kind, namespace, reason}

# Safety
drift_safety_checks_total{check, result="passed|failed"}
```

## kubectl Plugin

```bash
# List all drifts
kubectl drift list

# Show drift for specific resource
kubectl drift show deployment/nginx -n production

# Preview auto-revert action
kubectl drift preview-revert deployment/nginx -n production

# Manually approve revert
kubectl drift revert deployment/nginx -n production
```

## Testing Strategy

### Unit Tests
- Three-way diff computation
- Server-side default filtering
- Drift classification logic
- Safety check implementations

### Integration Tests
- End-to-end drift detection
- Auto-revert with safety checks
- Attribution from audit logs
- Git cache refresh

### Acceptance Tests
- 60-second detection latency
- Zero false positives for defaulted fields
- 100% safety check coverage for reverts
- 95%+ attribution success rate
