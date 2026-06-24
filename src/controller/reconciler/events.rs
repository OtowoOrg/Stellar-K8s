//! Event recording helpers.

use super::prelude::*;

fn recorder_for(client: &Client, reporter: &Reporter, node: &StellarNode) -> Recorder {
    Recorder::new(client.clone(), reporter.clone(), node.object_ref(&()))
}

/// Publish a Kubernetes Event attached to the StellarNode using kube-rs [`Recorder`].
async fn publish_object_event(
    recorder: &Recorder,
    type_: EventType,
    reason: &str,
    action: &str,
    note: &str,
) -> Result<()> {
    recorder
        .publish(K8sRecorderEvent {
            type_,
            reason: reason.to_string(),
            action: action.to_string(),
            note: Some(note.to_string()),
            secondary: None,
        })
        .await
        .map_err(Error::KubeError)
}

pub(crate) fn emit_event_owned(
    client: Client,
    reporter: Reporter,
    node: Arc<StellarNode>,
    type_: EventType,
    reason: String,
    action: String,
    note: String,
) -> BoxFuture<'static, Result<()>> {
    async move {
        let recorder = recorder_for(&client, &reporter, &node);
        publish_object_event(&recorder, type_, &reason, &action, &note).await
    }
    .boxed()
}

pub(crate) fn publish_stellar_event_owned(
    client: Client,
    reporter: Reporter,
    node: Arc<StellarNode>,
    type_: EventType,
    reason: String,
    action: String,
    note: String,
) -> BoxFuture<'static, Result<()>> {
    emit_event_owned(client, reporter, node, type_, reason, action, note)
}

/// Returns whether the primary workload (Deployment or StatefulSet) for this node already exists.
async fn workload_resource_exists(client: &Client, node: &StellarNode) -> Result<bool> {
    let namespace = node.namespace().unwrap_or_else(|| "default".to_string());
    let name = node.name_any();
    match node.spec.node_type {
        NodeType::Validator => {
            let api: Api<StatefulSet> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(_) => Ok(true),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
        NodeType::Horizon | NodeType::SorobanRpc => {
            let api: Api<Deployment> = Api::namespaced(client.clone(), &namespace);
            match api.get(&name).await {
                Ok(_) => Ok(true),
                Err(kube::Error::Api(e)) if e.code == 404 => Ok(false),
                Err(e) => Err(Error::KubeError(e)),
            }
        }
    }
}

/// Format structured spec validation errors into a user-friendly message
fn format_spec_validation_errors(errors: &[SpecValidationError]) -> String {
    let mut msg = String::from("Spec validation failed with the following issues:\n");
    for e in errors {
        msg.push_str(&format!(
            "- Field `{}`: {}\n  How to fix: {}\n",
            e.field, e.message, e.how_to_fix
        ));
    }
    msg.trim_end().to_string()
}

/// Emit a single grouped Kubernetes Event for all spec validation errors
async fn emit_spec_validation_event(
    client: &Client,
    reporter: &Reporter,
    node: &StellarNode,
    errors: &[SpecValidationError],
) -> Result<()> {
    let message = format_spec_validation_errors(errors);
    publish_stellar_event!(
        client,
        reporter,
        node,
        EventType::Warning,
        "SpecValidationFailed",
        "ValidationFailed",
        &message,
    )
    .await
}
/// Action types for apply_or_emit helper
#[derive(Debug, Clone, Copy)]
pub enum ActionType {
    Create,
    Update,
    Delete,
}

impl std::fmt::Display for ActionType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ActionType::Create => write!(f, "create"),
            ActionType::Update => write!(f, "update"),
            ActionType::Delete => write!(f, "delete"),
        }
    }
}

/// Helper to perform an action or emit a "WouldPatch" event in dry-run mode
pub(crate) fn apply_or_emit_owned<Fut>(
    ctx: Arc<ControllerState>,
    node: Arc<StellarNode>,
    action: ActionType,
    resource_info: String,
    fut: Fut,
) -> BoxFuture<'static, Result<()>>
where
    Fut: std::future::Future<Output = Result<()>> + Send + 'static,
{
    async move {
        if ctx.dry_run {
            let reason = match action {
                ActionType::Create => "WouldCreate",
                ActionType::Update => "WouldUpdate",
                ActionType::Delete => "WouldDelete",
            };
            let message = format!("Dry Run: Would {action} {resource_info}");
            info!("{}", message);
            publish_stellar_event!(
                ctx.client,
                ctx.event_reporter,
                node,
                EventType::Normal,
                reason,
                "DryRun",
                message
            )
            .await?;
        } else {
            fut.await?;
        }
        Ok(())
    }
    .boxed()
}

/// The core reconciliation state machine for StellarNode resources.
///
/// This function is triggered by the kube-rs runtime whenever a StellarNode is
/// created, updated, or deleted, or when a child resource (like a Pod or Service)
/// changes state.
///
/// # Design Philosophy: "Declarative Convergence"
/// The reconciler does not perform imperative "actions". Instead, it:
/// 1. **Observes** the current cluster state.
/// 2. **Computes** the delta between Spec and Status.
/// 3. **Applies** patches to drive the cluster toward the desired state.
///
/// # Critical Sections
/// - **Finalizer Handling**: Ensures that data volumes (PVCs) are preserved or cleaned up
///   according to the `retentionPolicy` before the CRD is deleted.
/// - **Leader Election**: Only the leading operator instance executes the full logic
///   to prevent conflicting patches.
/// - **Network Safety**: Verifies that Mainnet and Testnet nodes are never co-located
///   in a way that risks ledger corruption.
///
/// # Error Handling
/// Returns a `Result<Action, Error>`. Retriable errors (like K8s API timeouts)
