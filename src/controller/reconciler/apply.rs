//! Apply reconciliation path.

use super::events::ActionType;
use super::events::{
    emit_spec_validation_event, format_spec_validation_errors, publish_object_event, recorder_for,
    workload_resource_exists,
};
use super::prelude::*;
use super::state::ControllerState;
use super::support::*;
use super::{ToControllerStateArc, ToStellarNodeArc};
use crate::{apply_or_emit, emit_event, publish_stellar_event};

pub(crate) fn apply_stellar_node(
    client: Client,
    node: Arc<StellarNode>,
    ctx: Arc<ControllerState>,
) -> BoxFuture<'static, Result<Action>> {
    async move {
        let name = node.name_any();
        let namespace = node.namespace().unwrap_or_else(|| "default".to_string());

        info!("Applying StellarNode: {}/{}", namespace, name);

        // Resolve effective resource requirements:
        // Precedence: spec.resources (non-empty) > Helm defaults > hardcoded fallback.
        let effective_resources = {
            let spec_resources = &node.spec.resources;
            if !spec_resources.requests.cpu.is_empty() {
                // Spec wins — use as-is
                spec_resources.clone()
            } else if let Some(helm_d) = ctx.operator_config.defaults_for(&node.spec.node_type) {
                crate::crd::ResourceRequirements {
                    requests: crate::crd::ResourceSpec {
                        cpu: helm_d.requests.cpu.clone(),
                        memory: helm_d.requests.memory.clone(),
                    },
                    limits: crate::crd::ResourceSpec {
                        cpu: helm_d.limits.cpu.clone(),
                        memory: helm_d.limits.memory.clone(),
                    },
                }
            } else {
                hardcoded_defaults(&node.spec.node_type)
            }
        };
        debug!(
            "Effective resources for {}/{}: requests={}/{} limits={}/{}",
            namespace,
            name,
            effective_resources.requests.cpu,
            effective_resources.requests.memory,
            effective_resources.limits.cpu,
            effective_resources.limits.memory,
        );

        // Validate the spec
        if let Err(errors) = node.spec.validate() {
            let message = format_spec_validation_errors(&errors);
            warn!("Validation failed for {}/{}: {}", namespace, name, message);
            emit_spec_validation_event(&client, &ctx.event_reporter, &node, &errors).await?;
            update_status(&client, &node, "Failed", Some(message.clone()), 0, true).await?;
            return Err(Error::ValidationError(message));
        }

        // Network safety check — must run before any resources are created.
        // Ensures no Mainnet node shares a namespace with a Testnet node (or vice versa).
        if let Err(e) = crate::controller::network_isolation::check_network_safety(&client, &node).await {
            let msg = e.to_string();
            warn!(
                "Network safety check failed for {}/{}: {}",
                namespace, name, msg
            );
            emit_event!(
                &client,
                &ctx.event_reporter,
                &node,
                kube::runtime::events::EventType::Warning,
                "NetworkSafetyViolation",
                "NetworkIsolation",
                &msg,
            )
            .await?;
            update_status(&client, &node, "Failed", Some(msg.clone()), 0, true).await?;
            return Err(e);
        }

        let propagated_labels = Arc::new(LabelPropagator::new(&node).compute());

        // ── Plugin SDK: pre_reconcile hooks ───────────────────────────────────
        let plugin_ctx = ReconcileContext::from_node(&node);
        match ctx.plugin_registry.run_pre_reconcile(&plugin_ctx).await {
            HookResult::Continue => {}
            HookResult::Abort(reason) => {
                warn!(
                    "Plugin aborted reconciliation for {}/{}: {}",
                    namespace, name, reason
                );
                return Err(Error::ConfigError(format!("plugin aborted: {reason}")));
            }
        }

        // Enforce PSS 'restricted' on the managed namespace (idempotent)
        if let Err(e) = pss::ensure_namespace_pss_labels(&client, &namespace).await {
            warn!(
                "Failed to apply PSS labels to namespace '{}': {}. Continuing reconciliation.",
                namespace, e
            );
        }

        // 1. Core infrastructure (PVC and ConfigMap) always managed by operator
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "PVC and ConfigMap", clones: [propagated_labels], move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                resources::ensure_pvc(&client, &node, &propagated_labels, ctx.dry_run).await?;
                resources::ensure_config_map(&client, &node, None, ctx.enable_mtls, ctx.dry_run)
                    .await?;
                Ok(())
            }
        )
        .await?;

        // 1a. Managed Database (CloudNativePG)
        apply_or_emit!(&ctx, &node, ActionType::Update, "Managed Database", move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            resources::ensure_cnpg_cluster(&client, &node, ctx.dry_run).await?;
            resources::ensure_cnpg_pooler(&client, &node, ctx.dry_run).await?;
            Ok(())
        })
        .await?;

        // 2. Handle suspension
        if node.spec.suspended {
            apply_or_emit!(&ctx, &node, ActionType::Update, "Suspended state resources", clones: [propagated_labels], move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                    resources::ensure_pvc(&client, &node, &propagated_labels, ctx.dry_run).await?;
                    resources::ensure_config_map(&client, &node, None, ctx.enable_mtls, ctx.dry_run)
                        .await?;

                    match node.spec.node_type {
                        NodeType::Validator => {
                            // Suspended validators don't need seed injection resolved
                            resources::ensure_statefulset(&client, &node, ctx.enable_mtls,
                                None,
                                &propagated_labels,
                                ctx.dry_run,
                            )
                            .await?;
                        }
                        NodeType::Horizon | NodeType::SorobanRpc => {
                            resources::ensure_deployment(&client, &node, ctx.enable_mtls,
                                &propagated_labels,
                                ctx.dry_run,
                            )
                            .await?;
                        }
                    }

                    resources::ensure_service(&client, &node, ctx.enable_mtls,
                        &propagated_labels,
                        ctx.dry_run,
                    )
                    .await?;
                    Ok(())
                }
            )
            .await?;

            apply_or_emit!(&ctx, &node, ActionType::Update, "Status (Maintenance)", clones: [], move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                    update_status(
                        &client,
                        &node,
                        "Maintenance",
                        Some("Manual maintenance mode active; workload management paused".to_string()),
                        0,
                        true,
                    )
                    .await?;
                    update_suspended_status(&client, &node).await?;
                    Ok(())
                }
            )
            .await?;

            return Ok(Action::requeue(Duration::from_secs(60)));
        }

        // 3. Normal Mode: Handle suspension
        // This only runs if NOT in maintenance mode.
        if node.spec.suspended {
            info!("Node {}/{} is suspended, scaling to 0", namespace, name);
            update_status(
                &client,
                &node,
                "Suspended",
                Some("Node is suspended".to_string()),
                0,
                true,
            )
            .await?;
            // Still create resources but with 0 replicas
        }

        // Handle Horizon database migrations
        if node.spec.node_type == NodeType::Horizon {
            if let Some(horizon_config) = &node.spec.horizon_config {
                if horizon_config.auto_migration {
                    let current_version = &node.spec.version;
                    let last_migrated = node
                        .status
                        .as_ref()
                        .and_then(|s| s.last_migrated_version.as_ref());

                    if last_migrated.map(|v| v != current_version).unwrap_or(true) {
                        info!(
                            "Database migration required for Horizon {}/{} (version: {})",
                            namespace, name, current_version
                        );

                        publish_stellar_event!(
                            &client,
                            &ctx.event_reporter,
                            &node,
                            EventType::Normal,
                            "DatabaseMigrationRequired",
                            "Migrate",
                            &format!(
                                "Database migration will be performed via InitContainer for version {current_version}"
                            ),
                        )
                        .await?;
                    }
                }
            }
        }

        // History Archive Health Check for Validators
        if node.spec.node_type == NodeType::Validator {
            if let Some(validator_config) = &node.spec.validator_config {
                if validator_config.enable_history_archive
                    && !validator_config.history_archive_urls.is_empty()
                {
                    let is_startup_or_update = node
                        .status
                        .as_ref()
                        .and_then(|s| s.observed_generation)
                        .map(|og| og < node.metadata.generation.unwrap_or(0))
                        .unwrap_or(true);

                    if is_startup_or_update {
                        info!(
                            "Running history archive health check for {}/{}",
                            namespace, name
                        );

                        let health_result = Arc::new(
                            check_history_archive_health(&validator_config.history_archive_urls, None)
                                .await?,
                        );

                        if !health_result.any_healthy {
                            warn!(
                                "Archive health check failed for {}/{}: {}",
                                namespace,
                                name,
                                health_result.summary()
                            );

                            // Emit Kubernetes Event
                            publish_stellar_event!(
                                &client,
                                &ctx.event_reporter,
                                &node,
                                EventType::Warning,
                                "ArchiveHealthCheckFailed",
                                "ArchiveHealth",
                                &format!(
                                    "None of the configured archives are reachable:\n{}",
                                    health_result.error_details()
                                ),
                            )
                            .await?;

                            // Update status with archive health condition (observed_generation NOT updated to trigger retry)
                            apply_or_emit!(
                                &ctx,
                                &node,
                                ActionType::Update,
                                "Status (Archive Health Failed)",
                                move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                                    update_archive_health_status(&client, &node, &health_result)
                                        .await?;
                                    Ok(())
                                }
                            )
                            .await?;

                            let delay = calculate_backoff(0, None, None);
                            info!(
                                "Archive health check failed for {}/{}, requeuing in {:?}",
                                namespace, name, delay
                            );

                            return Ok(Action::requeue(delay));
                        } else {
                            info!(
                                "Archive health check passed for {}/{}: {}",
                                namespace,
                                name,
                                health_result.summary()
                            );
                            apply_or_emit!(
                                &ctx,
                                &node,
                                ActionType::Update,
                                "Status (Archive Health Passed)",
                                move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                                    update_archive_health_status(&client, &node, &health_result)
                                        .await?;
                                    Ok(())
                                }
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        // Periodic archive integrity check (every 1 hour) for validators with archive enabled.
        // This compares stellar-history.json ledger sequences against the validator's current
        // ledger and sets/clears the ArchiveIntegrityDegraded condition + Prometheus alert metric.
        if node.spec.node_type == NodeType::Validator {
            if let Some(validator_config) = &node.spec.validator_config {
                if validator_config.enable_history_archive
                    && !validator_config.history_archive_urls.is_empty()
                {
                    const ARCHIVE_CHECK_INTERVAL_SECS: i64 = 3600;
                    let last_check_time = node
                        .status
                        .as_ref()
                        .and_then(|s| {
                            s.conditions
                                .iter()
                                .find(|c| c.type_ == "ArchiveIntegrityDegraded")
                                .map(|c| c.last_transition_time.clone())
                        })
                        .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                        .map(|dt| dt.with_timezone(&chrono::Utc));

                    let should_run = match last_check_time {
                        None => true, // never checked
                        Some(last) => {
                            let age_secs = (chrono::Utc::now() - last).num_seconds();
                            age_secs >= ARCHIVE_CHECK_INTERVAL_SECS
                        }
                    };

                    if should_run {
                        if let Err(e) = run_archive_integrity_check(
                            &client,
                            &ctx.event_reporter,
                            &node,
                            &validator_config.history_archive_urls,
                        )
                        .await
                        {
                            warn!(
                                "Archive integrity check error for {}/{}: {}",
                                namespace, name, e
                            );
                        }
                    }
                }

                // Automatic checkpoint integrity checks are configured under DR config.
                if let Some(archive_config) = node
                    .spec
                    .dr_config
                    .as_ref()
                    .and_then(|dr| dr.archive_integrity_config.as_ref())
                {
                    if archive_config.enabled && !validator_config.history_archive_urls.is_empty() {
                        let interval = match parse_duration(&archive_config.interval) {
                            Ok(d) => d,
                            Err(_) => Duration::from_secs(21600), // Default 6h
                        };

                        let last_check_time = node
                            .status
                            .as_ref()
                            .and_then(|s| {
                                s.conditions
                                    .iter()
                                    .find(|c| c.type_ == "ArchiveIntegrityCheck")
                                    .map(|c| c.last_transition_time.clone())
                            })
                            .and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok())
                            .map(|dt| dt.with_timezone(&chrono::Utc));

                        let should_run = match last_check_time {
                            None => true,
                            Some(last) => {
                                let age = chrono::Utc::now() - last;
                                age.to_std().unwrap_or(Duration::from_secs(0)) >= interval
                            }
                        };

                        if should_run {
                            if let Err(e) = run_archive_checkpoint_verification(
                                &client,
                                &ctx.event_reporter,
                                &node,
                                &validator_config.history_archive_urls,
                                archive_config,
                            )
                            .await
                            {
                                warn!(
                                    "Archive checkpoint verification error for {}/{}: {}",
                                    namespace, name, e
                                );
                            }
                        }
                    }
                }
            }
        }

        // Update status to Creating
        apply_or_emit!(&ctx, &node, ActionType::Update, "Status (DR)", move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            update_status(
                &client,
                &node,
                "DR_Active",
                Some("Disaster recovery mode active".to_string()),
                0,
                true,
            )
            .await?;
            Ok(())
        })
        .await?;

        // 1. Create/update the PersistentVolumeClaim
        apply_or_emit!(&ctx, &node, ActionType::Create, "PVC", clones: [propagated_labels], move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            resources::ensure_pvc(&client, &node, &propagated_labels, ctx.dry_run).await?;
            Ok(())
        })
        .await?;
        info!("PVC ensured for {}/{}", namespace, name);

        // 2. Handle VSL Fetching for Validators
        let mut quorum_override: Option<crate::controller::vsl::QuorumSet> = None;
        if node.spec.node_type == NodeType::Validator {
            if let Some(config) = &node.spec.validator_config {
                if let Some(vl_source) = &config.vl_source {
                    match vsl::fetch_vsl(vl_source).await {
                        Ok(quorum) => {
                            quorum_override = Some(quorum);
                        }
                        Err(e) => {
                            warn!("Failed to fetch VSL for {}/{}: {}", namespace, name, e);
                            publish_stellar_event!(
                                &client,
                                &ctx.event_reporter,
                                &node,
                                EventType::Warning,
                                "VSLFetchFailed",
                                "VSLFetch",
                                &format!("Failed to fetch VSL from {vl_source}: {e}"),
                            )
                            .await?;
                        }
                    }
                }
            }
        }

        let quorum_override = Arc::new(quorum_override);

        // 3. Create/update the ConfigMap for node configuration
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "ConfigMap",
            clones: [quorum_override],
            move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                resources::ensure_config_map(&client, &node, (*quorum_override).clone(),
                    ctx.enable_mtls,
                    ctx.dry_run,
                )
                .await?;
                Ok(())
            }
        )
        .await?;
        info!("ConfigMap ensured for {}/{}", namespace, name);

        // 3. Handle suspension or Maintenance
        if node.spec.maintenance_mode {
            update_status(
                &client,
                &node,
                "Maintenance",
                Some("Manual maintenance mode active; workload management paused".to_string()),
                0,
                true,
            )
            .await?;
            return Ok(Action::requeue(Duration::from_secs(60)));
        }

        if node.spec.suspended {
            info!("Node {}/{} is suspended, scaling to 0", namespace, name);
            apply_or_emit!(&ctx, &node, ActionType::Update, "Status (Suspended)", clones: [], move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                    update_suspended_status(&client, &node).await?;
                    Ok(())
                }
            )
            .await?;
            // Continue to ensure resources exist but with 0 replicas
        }

        // 4. Ensure mTLS certificates
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "mTLS certificates",
            clones: [namespace],
            move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                mtls::ensure_ca(&client, &namespace).await?;
                mtls::ensure_node_cert(&client, &node).await?;
                // If cert-manager is configured, also create the Certificate CR so
                // cert-manager takes over issuance and rotation going forward.
                if let Some(cm_cfg) = &node.spec.cert_manager {
                    mtls::ensure_cert_manager_certificate(&client, &node, cm_cfg).await?;
                }
                Ok(())
            }
        )
        .await?;

        let workload_existed_before = workload_resource_exists(&client, &node)
            .await
            .unwrap_or(false);

        // 5. Create/update the Deployment/StatefulSet based on node type
        let workload_result = apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "Workload (Deployment/StatefulSet)",
            clones: [propagated_labels, namespace, name],
            move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                match node.spec.node_type {
                    NodeType::Validator => {
                        // Resolve the KMS/ESO/CSI seed injection spec before building the StatefulSet.
                        // Creates any required ExternalSecret CR and returns a lightweight descriptor
                        // of how to wire the seed into the pod. No secret values are ever read.
                        let seed_injection = if let Some(validator_config) = &node.spec.validator_config {
                            if let Some(_source) = validator_config.resolve_seed_source() {
                                match kms_secret::reconcile_seed_secret(&client, &node).await {
                                    Ok(spec) => Some(spec),
                                    Err(e) => {
                                        warn!(
                                            "Seed secret reconciliation failed for {}/{}: {}. \
                                             Falling back to legacy seed_secret_ref behaviour.",
                                            namespace, name, e
                                        );
                                        None
                                    }
                                }
                            } else {
                                None
                            }
                        } else {
                            None
                        };

                        resources::ensure_statefulset(&client, &node, ctx.enable_mtls,
                            seed_injection.as_ref(),
                            &propagated_labels,
                            ctx.dry_run,
                        )
                        .await?;
                        kms_secret::reconcile_vault_secret_rotation(&client, &node, seed_injection.as_ref(),
                        )
                        .await?;
                        crate::controller::forensic_snapshot::reconcile_forensic_snapshot(&client, &node).await?;
                    }
                    NodeType::Horizon | NodeType::SorobanRpc => {
                        let current_version = get_current_deployment_version(&client, &node).await?;
                        let blue_green_migration = node.spec.node_type == NodeType::Horizon
                            && node.spec.strategy.strategy_type
                                == crate::crd::types::RolloutStrategyType::BlueGreen
                            && node
                                .spec
                                .horizon_config
                                .as_ref()
                                .map(|cfg| cfg.auto_migration)
                                .unwrap_or(false)
                            && current_version
                                .as_ref()
                                .map(|v| v != &node.spec.version)
                                .unwrap_or(false);

                        if !blue_green_migration {
                            resources::ensure_deployment(
                                &client,
                                &node,
                                ctx.enable_mtls,
                                &propagated_labels,
                                ctx.dry_run,
                            )
                            .await?;
                        } else {
                            info!(
                                "Starting blue/green Horizon migration for {}/{}",
                                namespace, name
                            );

                            let status_patch = serde_json::json!({
                                "status": {
                                    "phase": "Migrating",
                                    "message": format!(
                                        "Performing blue/green Horizon schema migration to {}",
                                        node.spec.version
                                    )
                                }
                            });
                            let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                            api.patch_status(
                                &name,
                                &PatchParams::apply("stellar-operator"),
                                &Patch::Merge(&status_patch),
                            )
                            .await?;

                            let config = crate::controller::blue_green::BlueGreenConfig::default();
                            let migration_success = crate::controller::blue_green::orchestrate_horizon_migration(
                                &client,
                                &node,
                                &config,
                            )
                            .await?;

                            if migration_success {
                                let patch = serde_json::json!({
                                    "status": {
                                        "lastMigratedVersion": node.spec.version,
                                        "phase": "Running",
                                        "message": format!(
                                            "Horizon migration to {} completed successfully",
                                            node.spec.version
                                        )
                                    }
                                });
                                api.patch_status(
                                    &name,
                                    &PatchParams::apply("stellar-operator"),
                                    &Patch::Merge(&patch),
                                )
                                .await?;
                            } else {
                                let patch = serde_json::json!({
                                    "status": {
                                        "phase": "Failed",
                                        "message": format!(
                                            "Blue/green Horizon migration to {} failed",
                                            node.spec.version
                                        )
                                    }
                                });
                                api.patch_status(
                                    &name,
                                    &PatchParams::apply("stellar-operator"),
                                    &Patch::Merge(&patch),
                                )
                                .await?;
                            }
                        }

                        // Handle Canary Deployment
                        if let Some(cfg) = node.spec.strategy.canary() {
                            // Determine if we are in a canary state
                            let current_version = get_current_deployment_version(&client, &node).await?;

                            // Check if we already have an active canary
                            let mut is_canary_active = node
                                .status
                                .as_ref()
                                .and_then(|status| status.canary_version.as_ref())
                                .is_some();

                            if !is_canary_active {
                                if let Some(cv) = &current_version {
                                    if cv != &node.spec.version {
                                        // 1. Start Canary: We have a version mismatch, start canary
                                        info!(
                                            "Canary version mismatch: spec={} current={}. Starting canary.",
                                            node.spec.version, cv
                                        );
                                        let now = chrono::Utc::now().to_rfc3339();

                                        // Update status to indicate canary has started
                                        let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                                        let patch = serde_json::json!({
                                            "status": {
                                                "canaryVersion": node.spec.version,
                                                "canaryStartTime": now,
                                                "phase": "Canary"
                                            }
                                        });
                                        api.patch_status(
                                            &name,
                                            &PatchParams::apply("stellar-operator"),
                                            &Patch::Merge(&patch),
                                        ).await?;

                                        is_canary_active = true;

                                        // We need to fetch the updated node with the new status
                                        // but we can proceed with creating canary resources for now
                                    }
                                }
                            }

                            if is_canary_active {
                                // 2. Monitor Canary: manage both deployments and sync ingress weights
                                resources::ensure_canary_deployment(&client, &node, ctx.enable_mtls, ctx.dry_run).await?;
                                resources::ensure_canary_service(&client, &node, ctx.enable_mtls, ctx.dry_run).await?;

                                let mut stable_node = node.as_ref().clone();
                                if let Some(cv) = &current_version {
                                    stable_node.spec.version = cv.clone();
                                }
                                resources::ensure_deployment(&client, &stable_node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;

                                // Sync ingress traffic weights (Nginx annotations + Istio VirtualService)
                                resources::ensure_ingress(&client, &node, ctx.dry_run).await?;

                                // Check if the canary interval has elapsed
                                if let Some(status) = &node.status {
                                    if let Some(start_time_str) = &status.canary_start_time {
                                        if let Ok(start_time) = chrono::DateTime::parse_from_rfc3339(start_time_str) {
                                            let now = chrono::Utc::now();
                                            let elapsed_secs = now.signed_duration_since(start_time).num_seconds();

                                            if elapsed_secs >= cfg.check_interval_seconds as i64 {
                                                // 3. Evaluate: check pod health + HTTP error rate
                                                info!(
                                                    "Canary check interval elapsed ({} >= {}s). Evaluating.",
                                                    elapsed_secs, cfg.check_interval_seconds
                                                );

                                                let canary_health = check_canary_health(&client, &node).await?;
                                                let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);

                                                if canary_health.healthy {
                                                    let consecutive = status.canary_consecutive_healthy + 1;
                                                    let current_weight = status.canary_weight.unwrap_or(cfg.weight);
                                                    let next_weight = if cfg.step_weight > 0 {
                                                        (current_weight + cfg.step_weight).min(cfg.max_weight)
                                                    } else {
                                                        current_weight
                                                    };

                                                    if consecutive >= cfg.success_threshold
                                                        && next_weight >= cfg.max_weight
                                                    {
                                                        // 4a. Promote — enough healthy checks at max weight
                                                        info!(
                                                            "Canary {}/{} healthy ({}/{} checks). Promoting.",
                                                            namespace, name, consecutive, cfg.success_threshold
                                                        );
                                                        resources::ensure_deployment(&client, &node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                                                        resources::delete_canary_resources(&client, &node, ctx.dry_run).await?;

                                                        let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                                                        let _ = publish_object_event(
                                                            &recorder,
                                                            EventType::Normal,
                                                            "CanaryPromoted",
                                                            "Canary",
                                                            &format!(
                                                                "Canary version {} promoted to stable after {} healthy checks",
                                                                node.spec.version, consecutive
                                                            ),
                                                        ).await;

                                                        let patch = serde_json::json!({
                                                            "status": {
                                                                "canaryVersion": null,
                                                                "canaryStartTime": null,
                                                                "canaryWeight": null,
                                                                "canaryErrorRate": null,
                                                                "canaryConsecutiveHealthy": 0,
                                                                "phase": "Running"
                                                            }
                                                        });
                                                        api.patch_status(&name, &PatchParams::apply("stellar-operator"), &Patch::Merge(&patch)).await?;
                                                    } else {
                                                        // Step up weight, reset interval timer
                                                        info!(
                                                            "Canary {}/{} healthy (check {}/{}). Weight {} -> {}.",
                                                            namespace, name, consecutive, cfg.success_threshold,
                                                            current_weight, next_weight
                                                        );
                                                        let patch = serde_json::json!({
                                                            "status": {
                                                                "canaryWeight": next_weight,
                                                                "canaryConsecutiveHealthy": consecutive,
                                                                "canaryStartTime": Utc::now().to_rfc3339()
                                                            }
                                                        });
                                                        api.patch_status(&name, &PatchParams::apply("stellar-operator"), &Patch::Merge(&patch)).await?;
                                                    }
                                                } else {
                                                    // 4b. Rollback — error rate spiked or pod unhealthy
                                                    warn!(
                                                        "Canary {}/{} unhealthy. Rolling back. Reason: {}",
                                                        namespace, name, canary_health.message
                                                    );
                                                    resources::delete_canary_resources(&client, &node, ctx.dry_run).await?;

                                                    let message = format!(
                                                        "Canary rollback triggered: {}",
                                                        canary_health.message
                                                    );

                                                    let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                                                    let _ = publish_object_event(
                                                        &recorder,
                                                        EventType::Warning,
                                                        "CanaryRolledBack",
                                                        "Canary",
                                                        &message,
                                                    ).await;

                                                    let patch = serde_json::json!({
                                                        "status": {
                                                            "canaryVersion": null,
                                                            "canaryStartTime": null,
                                                            "canaryWeight": null,
                                                            "canaryErrorRate": null,
                                                            "canaryConsecutiveHealthy": 0,
                                                            "phase": "Failed",
                                                            "message": message
                                                        }
                                                    });
                                                    api.patch_status(&name, &PatchParams::apply("stellar-operator"), &Patch::Merge(&patch)).await?;

                                                    let _ = remediation::emit_remediation_event(
                                                        &client,
                                                        &ctx.event_reporter,
                                                        &node,
                                                        remediation::RemediationLevel::Restart,
                                                        &message,
                                                    ).await;
                                                }
                                            } else {
                                                debug!(
                                                    "Canary interval not yet elapsed: {} < {}s",
                                                    elapsed_secs, cfg.check_interval_seconds
                                                );
                                            }
                                        }
                                    }
                                }
                            } else {
                                // No canary active, regular deployment ensure
                                resources::ensure_deployment(&client, &node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                                resources::delete_canary_resources(&client, &node, ctx.dry_run).await?;
                            }
                        } else {
                            // RPC nodes use Deployment
                            resources::ensure_deployment(&client, &node, ctx.enable_mtls, &propagated_labels, ctx.dry_run).await?;
                            info!("Deployment ensured for RPC node {}/{}", namespace, name);

                            // Clean up canary resources if they exist
                            resources::delete_canary_resources(&client, &node, ctx.dry_run).await?;
                        }
                    }
                }
                Ok(())
            },
        )
        .await;

        // 5.3: Handle workload failure — emit LabelPropagationFailed warning event and patch status
        match workload_result {
            Ok(()) => {}
            Err(workload_err) => {
                if let Err(e) = publish_stellar_event!(
                    &client,
                    &ctx.event_reporter,
                    &node,
                    EventType::Warning,
                    "LabelPropagationFailed",
                    "LabelPropagation",
                    &format!("Label propagation failed for workload: {workload_err}"),
                )
                .await
                {
                    warn!("Failed to publish LabelPropagationFailed event: {}", e);
                }
                {
                    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                    let patch = serde_json::json!({
                        "status": {
                            "labelPropagationStatus": "Failed"
                        }
                    });
                    if let Err(e) = api
                        .patch_status(
                            &name,
                            &PatchParams::apply("stellar-operator"),
                            &Patch::Merge(&patch),
                        )
                        .await
                    {
                        warn!("Failed to patch labelPropagationStatus to Failed: {}", e);
                    }
                }
                return Err(workload_err);
            }
        }

        // Workload block succeeded — continue with observability

        if !ctx.dry_run {
            let workload_exists_after = workload_resource_exists(&client, &node)
                .await
                .unwrap_or(true);
            if !workload_existed_before && workload_exists_after {
                let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                if let Err(e) = publish_object_event(
                    &recorder,
                    EventType::Normal,
                    "SuccessfulReconciliation",
                    "Created",
                    "Managed workload and related Kubernetes resources were created for this StellarNode.",
                )
                .await
                {
                    warn!("Failed to publish SuccessfulReconciliation event: {e}");
                }
            }
        }

        // 5.2 / 5.4: Emit LabelsPropagated event and patch labelPropagationStatus to Synced
        if let Err(e) = publish_stellar_event!(
            &client,
            &ctx.event_reporter,
            &node,
            EventType::Normal,
            "LabelsPropagated",
            "LabelPropagation",
            "Labels propagated to child resources",
        )
        .await
        {
            warn!("Failed to publish LabelsPropagated event: {}", e);
        }

        // 5.2: Emit LabelRemoved event when a generation change indicates labels may have been removed
        {
            let current_gen = node.metadata.generation.unwrap_or(0);
            let observed_gen = node
                .status
                .as_ref()
                .and_then(|s| s.observed_generation)
                .unwrap_or(0);
            if current_gen > observed_gen {
                if let Err(e) = publish_stellar_event!(
                    &client,
                    &ctx.event_reporter,
                    &node,
                    EventType::Normal,
                    "LabelRemoved",
                    "LabelPropagation",
                    "Removed orphan labels from child resources",
                )
                .await
                {
                    warn!("Failed to publish LabelRemoved event: {}", e);
                }
            }
        }

        {
            let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
            let patch = serde_json::json!({
                "status": {
                    "labelPropagationStatus": "Synced"
                }
            });
            if let Err(e) = api
                .patch_status(
                    &name,
                    &PatchParams::apply("stellar-operator"),
                    &Patch::Merge(&patch),
                )
                .await
            {
                warn!("Failed to patch labelPropagationStatus: {}", e);
            }
        }

        // 5a. MetalLB / LoadBalancer
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "MetalLB configuration",
            move |_client: Client, _ctx: Arc<ControllerState>, _node: Arc<StellarNode>| async move {
                // TODO: Load balancer and global discovery fields not yet implemented in StellarNodeSpec
                // resources::ensure_metallb_config(&client, &node).await?;
                // resources::ensure_load_balancer_service(&client, &node).await?;
                Ok(())
            }
        )
        .await?;

        // 5c. Secret rotation detection — passphrase and seed secrets
        //
        // Checks whether any referenced secrets have been rotated since the last
        // reconciliation. If so, triggers a graceful rolling restart via pod template
        // annotations so pods pick up the new secret values without downtime.
        {
            let dry_run = ctx.dry_run;
            if let Err(e) =
                secret_watcher::handle_passphrase_secret_rotation(&client, &node, dry_run).await
            {
                warn!(
                    "Passphrase secret rotation check failed for {}/{}: {}",
                    namespace, name, e
                );
            }
            if let Err(e) =
                secret_watcher::handle_seed_secret_rotation(&client, &node, dry_run).await
            {
                warn!(
                    "Seed secret rotation check failed for {}/{}: {}",
                    namespace, name, e
                );
            }
        }

        // 5b. Read-Only Replica Pools
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "Read-Only Replica Pool",
            move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                crate::controller::read_pool::ensure_read_pool(&client, &node, ctx.enable_mtls).await?;
                crate::controller::traffic::reconcile_traffic_routing(&client, &node).await?;
                Ok(())
            }
        )
        .await?;

        // 6. Autoscaling and Monitoring
        apply_or_emit!(
            &ctx,
            &node,
            ActionType::Update,
            "Monitoring and Scaling resources",
            move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                resources::ensure_service_monitor(&client, &node).await?;

                if node.spec.autoscaling.is_some() {
                    resources::ensure_hpa(&client, &node, ctx.dry_run).await?;
                }

                // VPA Integration
                match &node.spec.vpa_config {
                    Some(vpa_cfg) => {
                        vpa_controller::ensure_vpa(&client, &node, vpa_cfg).await?;
                    }
                    None => {
                        // Clean up VPA if vpaConfig was removed from the spec
                        vpa_controller::delete_vpa(&client, &node).await?;
                    }
                }

                resources::ensure_pdb(&client, &node, ctx.dry_run).await?;
                resources::ensure_alerting(&client, &node, ctx.dry_run).await?;
                resources::ensure_network_policy(&client, &node, ctx.dry_run).await?;
                Ok(())
            },
        )
        .await?;

        // 6.5. Gas Autoscaling (Soroban RPC only)
        if !ctx.dry_run && node.spec.node_type == NodeType::SorobanRpc {
            if let Some(autoscaling) = &node.spec.autoscaling {
                if let Some(gas_cfg) = &autoscaling.gas_autoscaling {
                    crate::controller::gas_autoscaling::ensure_gas_autoscaler_running(
                        client.clone(),
                        &node,
                        gas_cfg,
                    );
                }
            }
        }

        // 6a. CSI VolumeSnapshot schedule (Validator only)
        if node.spec.node_type == NodeType::Validator {
            if let Some(ref snapshot_config) = node.spec.snapshot_schedule {
                if let Err(e) =
                    crate::controller::snapshot::reconcile_snapshot(&client, &node, snapshot_config).await
                {
                    warn!(
                        "Snapshot reconciliation failed for {}/{}: {}",
                        namespace, name, e
                    );
                }
            }
        }

        // 7. Perform health check to determine if node is ready
        //
        // Measure reduction in API polling overhead: Reactive Status check
        // If the DB trigger updated the status very recently (e.g. < 15 seconds ago), we can skip the health check API poll
        let mut skipped_poll = false;
        let mut recent_health = None;
        if let Some(ref status) = node.status {
            if let Some(updated_at_str) = &status.ledger_updated_at {
                if let Ok(updated_at) = chrono::DateTime::parse_from_rfc3339(updated_at_str) {
                    let age = chrono::Utc::now()
                        .signed_duration_since(updated_at.with_timezone(&chrono::Utc))
                        .num_seconds();
                    if age < 15 {
                        info!("Skipping health polling for {}/{}, DB trigger recently updated status {}s ago", namespace, name, age);
                        #[cfg(feature = "metrics")]
                        crate::controller::metrics::inc_api_polls_avoided(&namespace, &name);
                        skipped_poll = true;
                        // Assume node is healthy, use the reactively set ledger sequence
                        recent_health = Some(health::HealthCheckResult::synced(status.ledger_sequence));
                    }
                }
            }
        }

        let health_result = if skipped_poll {
            recent_health.unwrap()
        } else {
            health::check_node_health(&client, &node, ctx.mtls_config.as_ref()).await?
        };

        debug!(
            "Health check result for {}/{}: healthy={}, synced={}, message={}",
            namespace, name, health_result.healthy, health_result.synced, health_result.message
        );

        // 7a. Sync-state-driven resource scaling (Validator only)
        //
        // Queries the stellar-core /info endpoint to determine whether the node is
        // "Catching up" or "Synced!" and applies the matching resource profile via
        // an in-place pod patch (no pod restart required).
        if let Some(scaling_config) = node.spec.sync_state_scaling.clone() {
            if scaling_config.enabled && node.spec.node_type == NodeType::Validator {
                let sync_state = sync_state_monitor::resolve_node_sync_state(&client, &node).await;

                info!("Sync state for {}/{}: {}", namespace, name, sync_state);

                // Persist the observed sync state to the CRD status.
                {
                    let api: Api<StellarNode> = Api::namespaced(client.clone(), &namespace);
                    let profile_label = sync_state.to_string();
                    let patch = serde_json::json!({
                        "status": {
                            "syncState": sync_state,
                            "syncScalingActiveProfile": profile_label,
                        }
                    });
                    if let Err(e) = api
                        .patch_status(
                            &name,
                            &PatchParams::apply("stellar-operator"),
                            &Patch::Merge(&patch),
                        )
                        .await
                    {
                        warn!(
                            "Failed to patch syncState status for {}/{}: {}",
                            namespace, name, e
                        );
                    }
                }

                let _scaling_config_clone = scaling_config.clone();
                apply_or_emit!(
                    &ctx,
                    &node,
                    ActionType::Update,
                    "Sync-state resource scaling",
                    move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                        sync_scale::reconcile_sync_scaling(&client, &node, &scaling_config, &sync_state)
                            .await?;
                        Ok(())
                    }
                )
                .await?;
            }
        }

        if let Some(cve_config) = node.spec.cve_handling.clone() {
            apply_or_emit!(&ctx, &node, ActionType::Update, "CVE Handling", move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                cve_reconciler::reconcile_cve_patches(&client, &node, &cve_config).await?;
                Ok(())
            })
            .await?;
        }

        // 7c. History archive pruning (for validators)
        if node.spec.node_type == NodeType::Validator {
            if let Some(pruning_policy) = &node.spec.pruning_policy {
                if pruning_policy.enabled {
                    apply_or_emit!(&ctx, &node, ActionType::Update, "Archive Pruning", clones: [namespace, name], move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                        match crate::controller::pruning_reconciler::reconcile_pruning(&client, &node).await {
                            Ok(Some(result)) => {
                                info!(
                                    "Archive pruning completed for {}/{}: {} deleted, {} retained",
                                    namespace, name, result.deleted_count, result.retained_count
                                );
                                crate::controller::pruning_reconciler::update_pruning_status(&client, &node, result)
                                    .await?;
                            }
                            Ok(None) => {
                                debug!("Pruning not scheduled to run for {}/{}", namespace, name);
                            }
                            Err(e) => {
                                warn!("Archive pruning failed for {}/{}: {}", namespace, name, e);
                            }
                        }
                        Ok(())
                    })
                    .await?;
                }
            }
        }

        // 6. Trigger peer configuration reload for validators if healthy
        if node.spec.node_type == NodeType::Validator && health_result.healthy {
            if let Err(e) = peer_discovery::trigger_peer_config_reload(&client, &node).await {
                warn!(
                    "Failed to trigger peer config reload for {}/{}: {}",
                    namespace, name, e
                );
            }
        }

        // 6.5. Quorum analysis for validators
        if node.spec.node_type == NodeType::Validator && health_result.healthy {
            if let Err(e) = perform_quorum_analysis(&client, &node, ctx.retry_budget_max_attempts).await
            {
                warn!("Quorum analysis failed for {}/{}: {}", namespace, name, e);
                // Don't fail reconciliation on quorum analysis errors
            }
        }

        // 7. Trigger config-reload if VSL was updated and pod is ready
        if let Some(_quorum) = &*quorum_override {
            if health_result.healthy {
                // Get pod IP to trigger reload
                let pod_api: Api<k8s_openapi::api::core::v1::Pod> =
                    Api::namespaced(client.clone(), &namespace);
                let lp = kube::api::ListParams::default()
                    .labels(&format!("app.kubernetes.io/instance={name}"));
                if let Ok(pods) = pod_api.list(&lp).await {
                    if let Some(pod) = pods.items.first() {
                        if let Some(status) = &pod.status {
                            if let Some(ip) = &status.pod_ip {
                                if let Err(e) = vsl::trigger_config_reload(ip).await {
                                    warn!(
                                        "Failed to trigger config-reload for {}/{}: {}",
                                        namespace, name, e
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }

        // 8. Disaster Recovery reconciliation
        let prev_dr_failover = node
            .status
            .as_ref()
            .and_then(|s| s.dr_status.as_ref())
            .map(|d| d.failover_active)
            .unwrap_or(false);
        if let Some(mut dr_status) = dr::reconcile_dr(&client, &node).await? {
            if dr_status.failover_active && !prev_dr_failover {
                let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                if let Err(e) = publish_object_event(
                    &recorder,
                    EventType::Normal,
                    "NodePromotedToPrimary",
                    "Failover",
                    "DR failover activated; this standby node is now primary.",
                )
                .await
                {
                    warn!("Failed to publish NodePromotedToPrimary event: {e}");
                }
            }
            // 8a. Check if DR drill should be executed
            if let Some(drill_config) = &node
                .spec
                .dr_config
                .as_ref()
                .and_then(|c| c.drill_schedule.clone())
            {
                if dr_drill::should_run_drill(&node, drill_config) {
                    match dr_drill::execute_dr_drill(&client, &node, drill_config, &dr_status).await {
                        Ok(drill_result) => {
                            dr_status.last_drill_time = Some(chrono::Utc::now().to_rfc3339());
                            dr_status.last_drill_result = Some(drill_result);
                            info!("DR drill completed for {}", node.name_any());
                        }
                        Err(e) => {
                            warn!("DR drill failed for {}: {}", node.name_any(), e);
                        }
                    }
                }
            }

            apply_or_emit!(
                &ctx,
                &node,
                ActionType::Update,
                "Status (DR)",
                move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                    update_dr_status(&client, &node, dr_status).await?;
                    Ok(())
                }
            )
            .await?;
        }

        // 8b. Cross-cloud failover for Horizon/SorobanRpc nodes
        if node
            .spec
            .cross_cloud_failover
            .as_ref()
            .map(|c| c.enabled)
            .unwrap_or(false)
        {
            let prev_cc_failover = node
                .status
                .as_ref()
                .and_then(|s| s.cross_cloud_failover_status.as_ref())
                .map(|s| s.failover_active)
                .unwrap_or(false);

            match cross_cloud_failover::reconcile_cross_cloud_failover(&client, &node).await {
                Ok(Some(cc_status)) => {
                    if cc_status.failover_active && !prev_cc_failover {
                        let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                        if let Err(e) = publish_object_event(
                            &recorder,
                            EventType::Normal,
                            "CrossCloudFailoverActivated",
                            "CrossCloudFailover",
                            &format!(
                                "Cross-cloud failover activated. Traffic routed to: {}",
                                cc_status.active_cloud.as_deref().unwrap_or("unknown")
                            ),
                        )
                        .await
                        {
                            warn!("Failed to publish CrossCloudFailoverActivated event: {e}");
                        }
                    } else if !cc_status.failover_active && prev_cc_failover {
                        let recorder = recorder_for(&client, &ctx.event_reporter, &node);
                        if let Err(e) = publish_object_event(
                            &recorder,
                            EventType::Normal,
                            "CrossCloudFailbackCompleted",
                            "CrossCloudFailover",
                            &format!(
                                "Cross-cloud failback completed. Traffic restored to: {}",
                                cc_status.active_cloud.as_deref().unwrap_or("primary")
                            ),
                        )
                        .await
                        {
                            warn!("Failed to publish CrossCloudFailbackCompleted event: {e}");
                        }
                    }

                    apply_or_emit!(
                        &ctx,
                        &node,
                        ActionType::Update,
                        "Status (Cross-Cloud Failover)",
                        move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                            update_cross_cloud_failover_status(&client, &node, cc_status).await?;
                            Ok(())
                        }
                    )
                    .await?;
                }
                Ok(None) => {} // Not configured or not applicable
                Err(e) => {
                    warn!(
                        "Cross-cloud failover reconciliation failed for {}/{}: {}",
                        namespace, name, e
                    );
                }
            }
        }

        // 9. Auto-remediation check
        if health_result.healthy && !node.spec.suspended {
            let stale_check = remediation::check_stale_node(&node, health_result.ledger_sequence);
            if stale_check.is_stale && remediation::can_remediate(&node) {
                if stale_check.recommended_action == remediation::RemediationLevel::Restart {
                    apply_or_emit!(
                        &ctx,
                        &node,
                        ActionType::Update,
                        "Remediation (Restart)",
                        move |client: Client, ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                            remediation::emit_remediation_event(
                                &client,
                                &ctx.event_reporter,
                                &node,
                                remediation::RemediationLevel::Restart,
                                "Stale ledger",
                            )
                            .await?;
                            remediation::restart_pod(&client, &node).await?;
                            remediation::update_remediation_state(
                                &client,
                                &node,
                                stale_check.current_ledger,
                                remediation::RemediationLevel::Restart,
                                true,
                            )
                            .await?;
                            Ok(())
                        }
                    )
                    .await?;
                    return Ok(Action::requeue(Duration::from_secs(30)));
                }
            } else {
                apply_or_emit!(
                    &ctx,
                    &node,
                    ActionType::Update,
                    "Remediation State",
                    move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                        remediation::update_remediation_state(
                            &client,
                            &node,
                            health_result.ledger_sequence,
                            remediation::RemediationLevel::None,
                            false,
                        )
                        .await?;
                        Ok(())
                    }
                )
                .await?;
            }
        }

        let prev_ready_reason = node.status.as_ref().and_then(|s| {
            conditions::find_condition(&s.conditions, conditions::CONDITION_TYPE_READY)
                .map(|c| c.reason.clone())
        });
        let sync_lag_begun = health_result.healthy
            && !health_result.synced
            && prev_ready_reason.as_deref() != Some("NodeSyncing");
        if sync_lag_begun {
            let recorder = recorder_for(&client, &ctx.event_reporter, &node);
            if let Err(e) = publish_object_event(
                &recorder,
                EventType::Warning,
                "SyncLagDetected",
                "Syncing",
                &health_result.message,
            )
            .await
            {
                warn!("Failed to publish SyncLagDetected event: {e}");
            }
        }

        // 10. Final Status Update
        let (phase, message) = if node.spec.suspended {
            ("Suspended", "Node is suspended".to_string())
        } else if !health_result.healthy {
            ("Creating", health_result.message.clone())
        } else if !health_result.synced {
            ("Syncing", health_result.message.clone())
        } else {
            ("Ready", "Node is healthy and synced".to_string())
        };

        apply_or_emit!(&ctx, &node, ActionType::Update, "Status (Final)", clones: [health_result, message], move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
            update_status_with_health(&client, &node, phase, Some(message.clone()), health_result.clone()).await?;

            let ready_replicas = get_ready_replicas(&client, &node).await.unwrap_or(0);
            update_status(&client, &node, phase, Some(message), ready_replicas, true).await?;
            Ok(())
        })
        .await?;

        // 9. Update status with ready replica count
        let phase = if node.spec.suspended {
            "Suspended"
        } else if node
            .status
            .as_ref()
            .and_then(|status| status.canary_version.as_ref())
            .is_some()
        {
            "Canary"
        } else {
            "Running"
        };

        // 10. Update ledger sequence metric if available
        if let Some(ref status) = node.status {
            #[cfg(feature = "metrics")]
            if let Some(seq) = status.ledger_sequence {
                let hardware_generation = hardware_generation_for_metrics(&client, &node).await;
                metrics::set_ledger_sequence(
                    &namespace,
                    &name,
                    &node.spec.node_type.to_string(),
                    node.spec.network_passphrase(),
                    &hardware_generation,
                    seq,
                );

                // Calculate ingestion lag if we can get the latest network ledger
                // For now we assume we have a way to track the "latest" known ledger across the cluster
                // or fetch it from a public horizon.
                if let Ok(network_latest) = get_latest_network_ledger(&node.spec.network).await {
                    let lag = (network_latest as i64) - (seq as i64);
                    metrics::set_ingestion_lag(
                        &namespace,
                        &name,
                        &node.spec.node_type.to_string(),
                        node.spec.network_passphrase(),
                        &hardware_generation,
                        lag.max(0),
                    );
                }
            }
        }

        // 10b. Update node sync status metric
        #[cfg(feature = "metrics")]
        {
            let hardware_generation = hardware_generation_for_metrics(&client, &node).await;
            metrics::set_node_sync_status(
                &namespace,
                &name,
                &node.spec.node_type.to_string(),
                node.spec.network_passphrase(),
                &hardware_generation,
                phase,
            );

            // 10c. Update node up status based on pod readiness
            metrics::set_node_up(
                &namespace,
                &name,
                &node.spec.node_type.to_string(),
                node.spec.network_passphrase(),
                &hardware_generation,
                health_result.healthy,
            );
        }

        // 10d. Proactive disk scaling check
        // Monitor PVC disk usage and automatically expand when threshold is exceeded
        if !ctx.dry_run {
            let disk_scaler_config = ctx.operator_config.disk_scaling.to_scaler_config();

            match disk_scaler::check_and_expand(&client, &node, &disk_scaler_config, ctx.dry_run).await {
                Ok(disk_scaler::ScalingResult::Expanded { old_size, new_size, expansion_count }) => {
                    info!(
                        "Expanded PVC for {}/{} from {} to {} (expansion #{})",
                        namespace, name, old_size, new_size, expansion_count
                    );

                    publish_stellar_event!(
                        &client,
                        &ctx.event_reporter,
                        &node,
                        EventType::Normal,
                        "DiskExpanded",
                        "Storage",
                        &format!(
                            "PVC automatically expanded from {} to {} (expansion #{}) due to high disk usage",
                            old_size, new_size, expansion_count
                        ),
                    )
                    .await
                    .ok();

                    #[cfg(feature = "metrics")]
                    {
                        let hardware_generation = hardware_generation_for_metrics(&client, &node).await;
                        metrics::increment_pvc_expansion_total(
                            &namespace,
                            &name,
                            &node.spec.node_type.to_string(),
                            node.spec.network_passphrase(),
                            &hardware_generation,
                        );
                        metrics::set_pvc_expansion_count(
                            &namespace,
                            &name,
                            &node.spec.node_type.to_string(),
                            node.spec.network_passphrase(),
                            &hardware_generation,
                            expansion_count as i64,
                        );
                    }
                }
                Ok(disk_scaler::ScalingResult::RateLimited { last_expansion, .. }) => {
                    debug!(
                        "Disk expansion rate-limited for {}/{} (last expansion: {})",
                        namespace, name, last_expansion
                    );
                }
                Ok(disk_scaler::ScalingResult::MaxExpansionsReached { count }) => {
                    warn!(
                        "Maximum disk expansions ({}) reached for {}/{}",
                        count, namespace, name
                    );

                    publish_stellar_event!(
                        &client,
                        &ctx.event_reporter,
                        &node,
                        EventType::Warning,
                        "MaxDiskExpansionsReached",
                        "Storage",
                        &format!(
                            "PVC has reached maximum expansion limit ({}). Manual intervention required.",
                            count
                        ),
                    )
                    .await
                    .ok();
                }
                Ok(disk_scaler::ScalingResult::NotSupported { storage_class }) => {
                    debug!(
                        "Disk expansion not supported for storage class {} on {}/{}",
                        storage_class, namespace, name
                    );
                }
                Ok(disk_scaler::ScalingResult::Failed { reason }) => {
                    warn!(
                        "Disk expansion failed for {}/{}: {}",
                        namespace, name, reason
                    );

                    publish_stellar_event!(
                        &client,
                        &ctx.event_reporter,
                        &node,
                        EventType::Warning,
                        "DiskExpansionFailed",
                        "Storage",
                        &format!("Failed to expand PVC: {}", reason),
                    )
                    .await
                    .ok();
                }
                Ok(disk_scaler::ScalingResult::NoActionNeeded) => {
                    // Disk usage is below threshold, no action needed
                }
                Err(e) => {
                    warn!(
                        "Error checking disk usage for {}/{}: {}",
                        namespace, name, e
                    );
                }
            }

            // Update disk usage metrics
            #[cfg(feature = "metrics")]
            if let Ok(Some(usage)) = disk_scaler::get_disk_usage(&client, &node).await {
                let hardware_generation = hardware_generation_for_metrics(&client, &node).await;
                metrics::set_pvc_disk_usage_percent(
                    &namespace,
                    &name,
                    &node.spec.node_type.to_string(),
                    node.spec.network_passphrase(),
                    &hardware_generation,
                    usage.usage_percent as i64,
                );
                metrics::set_pvc_size_bytes(
                    &namespace,
                    &name,
                    &node.spec.node_type.to_string(),
                    node.spec.network_passphrase(),
                    &hardware_generation,
                    usage.capacity_bytes as i64,
                );
            }
        }

        // 11. OCI snapshot push/pull Jobs
        if let Some(oci_cfg) = &node.spec.oci_snapshot {
            if oci_cfg.enabled {
                let ledger_seq = node
                    .status
                    .as_ref()
                    .and_then(|s| s.ledger_sequence)
                    .unwrap_or(0);

                // Push: trigger when node is healthy, synced, and we have a ledger number.
                if oci_cfg.push && health_result.healthy && health_result.synced && ledger_seq > 0 {
                    if let Err(e) =
                        oci_snapshot::ensure_snapshot_push_job(&client, &node, oci_cfg, ledger_seq).await
                    {
                        warn!(
                            "Failed to create OCI snapshot push Job for {}/{}: {}",
                            namespace, name, e
                        );
                        publish_stellar_event!(
                            &client,
                            &ctx.event_reporter,
                            &node,
                            EventType::Warning,
                            "OciSnapshotPushFailed",
                            "Snapshot",
                            &format!("Could not create snapshot push Job: {e}"),
                        )
                        .await
                        .ok();
                    }
                }

                // Pull: trigger on bootstrap when the node has never synced (ledger_seq == 0).
                // This extracts a prior snapshot so the node doesn't need a full catchup.
                if oci_cfg.pull && ledger_seq == 0 {
                    if let Err(e) =
                        oci_snapshot::ensure_snapshot_pull_job(&client, &node, oci_cfg, 0).await
                    {
                        warn!(
                            "Failed to create OCI snapshot pull Job for {}/{}: {}",
                            namespace, name, e
                        );
                        publish_stellar_event!(
                            &client,
                            &ctx.event_reporter,
                            &node,
                            EventType::Warning,
                            "OciSnapshotPullFailed",
                            "Snapshot",
                            &format!("Could not create snapshot pull Job: {e}"),
                        )
                        .await
                        .ok();
                    }
                }
            }
        }

        // 12. Service Mesh Configuration (Istio/Linkerd)
        if node.spec.service_mesh.is_some() {
            apply_or_emit!(
                &ctx,
                &node,
                ActionType::Update,
                "Service Mesh (Istio/Linkerd)",
                move |client: Client, _ctx: Arc<ControllerState>, node: Arc<StellarNode>| async move {
                    service_mesh::ensure_peer_authentication(&client, &node).await?;
                    service_mesh::ensure_destination_rule(&client, &node).await?;
                    service_mesh::ensure_virtual_service(&client, &node).await?;
                    service_mesh::ensure_request_authentication(&client, &node).await?;
                    Ok(())
                }
            )
            .await?;
        }

        // Cost estimation: annotate and export metric (non-fatal).
        {
            let cost = crate::controller::cost::estimate_monthly_cost(&node);
            if let Err(e) = crate::controller::cost::annotate_node_cost(&client, &node, cost).await {
                warn!(
                    "Failed to annotate node cost for {}/{}: {:?}",
                    namespace, name, e
                );
            }
            #[cfg(feature = "metrics")]
            crate::controller::cost::report_cost_metric(&namespace, &name, &node.spec.node_type.to_string(), cost);
        }

        // 13. Stamp audit annotations for the permanent reconcile trail.
        {
            use crate::controller::audit::actions;
            let action = match node.spec.node_type {
                crate::crd::NodeType::Validator => actions::UPDATED_STATEFULSET,
                crate::crd::NodeType::Horizon | crate::crd::NodeType::SorobanRpc => {
                    actions::UPDATED_DEPLOYMENT
                }
            };
            crate::controller::audit::patch_audit_annotations(&client, &node, action).await;
        }

        // 14. GitOps protocol upgrade — check if a timeline annotation is present and
        //     drive the next due upgrade step via ArgoCD or Flux.
        {
            use crate::controller::gitops_upgrade::{
                GitOpsEngine, GitOpsUpgradeController, ProtocolUpgradeTimeline,
            };

            let timeline_json = node
                .metadata
                .annotations
                .as_ref()
                .and_then(|a| a.get("stellar.org/protocol-upgrade-timeline"))
                .cloned();

            if let Some(json_str) = timeline_json {
                match serde_json::from_str::<ProtocolUpgradeTimeline>(&json_str) {
                    Ok(timeline) => {
                        let engine_str = node
                            .metadata
                            .annotations
                            .as_ref()
                            .and_then(|a| a.get("stellar.org/gitops-engine"))
                            .map(|s| s.as_str())
                            .unwrap_or("argocd");
                        let engine = if engine_str == "flux" {
                            GitOpsEngine::Flux
                        } else {
                            GitOpsEngine::ArgoCd
                        };
                        let controller = GitOpsUpgradeController::new(
                            engine,
                            std::time::Duration::from_secs(300),
                            0.95,
                        );
                        let current_protocol: u32 = node
                            .metadata
                            .annotations
                            .as_ref()
                            .and_then(|a| a.get("stellar.org/current-protocol"))
                            .and_then(|v| v.parse().ok())
                            .unwrap_or(0);
                        let now_unix = chrono::Utc::now().timestamp();
                        match controller
                            .plan_and_sync(&client, &node, &timeline, current_protocol, now_unix)
                            .await
                        {
                            Ok(Some(plan)) => {
                                info!(
                                    "GitOps upgrade planned for {}/{}: protocol v{} via {}",
                                    namespace, name, plan.target_protocol, engine_str
                                );
                            }
                            Ok(None) => {
                                debug!("No GitOps upgrade step due for {}/{}", namespace, name);
                            }
                            Err(e) => {
                                warn!(
                                    "GitOps upgrade planning failed for {}/{}: {}",
                                    namespace, name, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse protocol-upgrade-timeline annotation for {}/{}: {}",
                            namespace, name, e
                        );
                    }
                }
            }
        }

        // 15. Update status to Running with ready replica count
        // Use configured requeue interval for healthy reconciliation
        let requeue_interval = ctx.operator_config.reconciler.requeue_interval;

        // ── Plugin SDK: post_reconcile hooks ──────────────────────────────────
        ctx.plugin_registry.run_post_reconcile(&plugin_ctx).await;

        Ok(Action::requeue(Duration::from_secs(if phase == "Ready" {
            requeue_interval
        } else {
            // Use shorter interval for non-ready phases
            requeue_interval / 4
        })))
    }
    .boxed()
}
