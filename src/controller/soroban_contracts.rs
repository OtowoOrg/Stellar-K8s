use crate::crd::{
    ContractDeploymentPhase, ContractInstance, ContractInstanceStatus, ContractWASM,
    ContractWASMStatus,
};
use crate::error::{Error, Result};
use futures::StreamExt;
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::{Action, Controller};
use kube::runtime::watcher::Config;
use kube::{Client, ResourceExt};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const MAX_WASM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
struct ContractControllerContext {
    client: Client,
    http: reqwest::Client,
    is_leader: Arc<std::sync::atomic::AtomicBool>,
}

pub async fn run_contract_deployment_controller(
    client: Client,
    is_leader: Arc<std::sync::atomic::AtomicBool>,
    watch_namespace: Option<String>,
) -> Result<()> {
    let context = Arc::new(ContractControllerContext {
        client: client.clone(),
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|error| Error::ConfigError(error.to_string()))?,
        is_leader,
    });
    let wasms: Api<ContractWASM> = if let Some(namespace) = &watch_namespace {
        Api::namespaced(client.clone(), namespace)
    } else {
        Api::all(client.clone())
    };
    let instances: Api<ContractInstance> = if let Some(namespace) = &watch_namespace {
        Api::namespaced(client, namespace)
    } else {
        Api::all(client)
    };
    let wasm_context = context.clone();
    let wasm_controller = Controller::new(wasms, Config::default())
        .shutdown_on_signal()
        .run(reconcile_wasm, error_policy_wasm, wasm_context)
        .for_each(|_| async {});
    let instance_controller = Controller::new(instances, Config::default())
        .shutdown_on_signal()
        .run(reconcile_instance, error_policy_instance, context)
        .for_each(|_| async {});
    tokio::join!(wasm_controller, instance_controller);
    Ok(())
}

async fn reconcile_wasm(
    wasm: Arc<ContractWASM>,
    context: Arc<ContractControllerContext>,
) -> Result<Action> {
    if !context.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(Action::requeue(Duration::from_secs(5)));
    }
    let namespace = wasm.namespace().unwrap_or_else(|| "default".to_string());
    let name = wasm.name_any();
    let mut status = wasm.status.clone().unwrap_or_default();
    if matches!(
        status.phase,
        ContractDeploymentPhase::Ready | ContractDeploymentPhase::Failed
    ) {
        return Ok(Action::await_change());
    }

    if status.phase == ContractDeploymentPhase::Pending {
        let artifact = context
            .http
            .get(&wasm.spec.wasm_uri)
            .send()
            .await
            .map_err(config_error)?
            .error_for_status()
            .map_err(config_error)?;
        if artifact
            .content_length()
            .is_some_and(|length| length > MAX_WASM_BYTES as u64)
        {
            return fail_wasm(
                &context.client,
                &namespace,
                &name,
                status,
                "WASM exceeds 64 MiB",
            )
            .await;
        }
        let bytes = artifact.bytes().await.map_err(config_error)?;
        if bytes.len() > MAX_WASM_BYTES {
            return fail_wasm(
                &context.client,
                &namespace,
                &name,
                status,
                "WASM exceeds 64 MiB",
            )
            .await;
        }
        let hash = match crate::crd::contract_deployment::validate_sha256(&wasm.spec.sha256, &bytes)
        {
            Ok(hash) => hash,
            Err(error) => {
                return fail_wasm(&context.client, &namespace, &name, status, &error).await;
            }
        };
        status.wasm_hash = Some(hash);
        if ledger_entry_exists(
            &context.http,
            &wasm.spec.rpc_url,
            &wasm.spec.wasm_ledger_key_xdr,
        )
        .await?
        {
            status.phase = ContractDeploymentPhase::Ready;
            status.last_error = None;
            patch_wasm_status(&context.client, &namespace, &name, &status).await?;
            return Ok(Action::await_change());
        }
        let transaction_hash = submit_signed_transaction(
            &context.http,
            &wasm.spec.rpc_url,
            &wasm.spec.upload_transaction_xdr,
        )
        .await?;
        status.transaction_hash = Some(transaction_hash);
        status.phase = ContractDeploymentPhase::Submitted;
        status.last_error = None;
        patch_wasm_status(&context.client, &namespace, &name, &status).await?;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    match transaction_status(
        &context.http,
        &wasm.spec.rpc_url,
        status.transaction_hash.as_deref().unwrap_or_default(),
    )
    .await?
    .as_str()
    {
        "SUCCESS" => {
            status.phase = ContractDeploymentPhase::Ready;
            status.last_error = None;
            patch_wasm_status(&context.client, &namespace, &name, &status).await?;
            Ok(Action::await_change())
        }
        "FAILED" => {
            fail_wasm(
                &context.client,
                &namespace,
                &name,
                status,
                "WASM upload transaction failed on the network",
            )
            .await
        }
        _ => Ok(Action::requeue(Duration::from_secs(5))),
    }
}

async fn reconcile_instance(
    instance: Arc<ContractInstance>,
    context: Arc<ContractControllerContext>,
) -> Result<Action> {
    if !context.is_leader.load(std::sync::atomic::Ordering::Relaxed) {
        return Ok(Action::requeue(Duration::from_secs(5)));
    }
    let namespace = instance
        .namespace()
        .unwrap_or_else(|| "default".to_string());
    let name = instance.name_any();
    let mut status = instance.status.clone().unwrap_or_default();
    if matches!(
        status.phase,
        ContractDeploymentPhase::Ready | ContractDeploymentPhase::Failed
    ) {
        return Ok(Action::await_change());
    }

    if status.phase == ContractDeploymentPhase::Pending {
        let wasms: Api<ContractWASM> = Api::namespaced(context.client.clone(), &namespace);
        let wasm = wasms
            .get(&instance.spec.wasm)
            .await
            .map_err(Error::KubeError)?;
        let wasm_status: ContractWASMStatus = wasm.status.unwrap_or_default();
        if wasm_status.phase != ContractDeploymentPhase::Ready {
            return Ok(Action::requeue(Duration::from_secs(5)));
        }
        if !wasm_status
            .wasm_hash
            .as_deref()
            .is_some_and(|hash| hash.eq_ignore_ascii_case(&instance.spec.wasm_hash))
        {
            return fail_instance(
                &context.client,
                &namespace,
                &name,
                status,
                "referenced WASM hash does not match spec.wasmHash",
            )
            .await;
        }
        if ledger_entry_exists(
            &context.http,
            &instance.spec.rpc_url,
            &instance.spec.contract_ledger_key_xdr,
        )
        .await?
        {
            status.contract_id = Some(instance.spec.contract_id.clone());
            status.deployment_complete = true;
            status.last_error = None;
            return advance_storage(&instance, &context, &namespace, &name, status).await;
        }
        let transaction_hash = submit_signed_transaction(
            &context.http,
            &instance.spec.rpc_url,
            &instance.spec.deployment_transaction_xdr,
        )
        .await?;
        status.contract_id = Some(instance.spec.contract_id.clone());
        status.transaction_hash = Some(transaction_hash);
        status.phase = ContractDeploymentPhase::Submitted;
        status.last_error = None;
        patch_instance_status(&context.client, &namespace, &name, &status).await?;
        return Ok(Action::requeue(Duration::from_secs(5)));
    }

    match transaction_status(
        &context.http,
        &instance.spec.rpc_url,
        status.transaction_hash.as_deref().unwrap_or_default(),
    )
    .await?
    .as_str()
    {
        "SUCCESS" => {
            status.deployment_complete = true;
            status.last_error = None;
            advance_storage(&instance, &context, &namespace, &name, status).await
        }
        "FAILED" => {
            fail_instance(
                &context.client,
                &namespace,
                &name,
                status,
                "contract deployment transaction failed on the network",
            )
            .await
        }
        _ => Ok(Action::requeue(Duration::from_secs(5))),
    }
}

async fn advance_storage(
    instance: &ContractInstance,
    context: &ContractControllerContext,
    namespace: &str,
    name: &str,
    mut status: ContractInstanceStatus,
) -> Result<Action> {
    while let Some(entry) = instance.spec.storage.get(status.storage_index as usize) {
        if !ledger_entry_exists(&context.http, &instance.spec.rpc_url, &entry.key_xdr).await? {
            let transaction_hash = submit_signed_transaction(
                &context.http,
                &instance.spec.rpc_url,
                &entry.transaction_xdr,
            )
            .await?;
            status.transaction_hash = Some(transaction_hash);
            status.storage_index += 1;
            status.phase = ContractDeploymentPhase::Submitted;
            patch_instance_status(&context.client, namespace, name, &status).await?;
            return Ok(Action::requeue(Duration::from_secs(5)));
        }
        status.storage_index += 1;
    }

    status.phase = ContractDeploymentPhase::Ready;
    status.last_error = None;
    patch_instance_status(&context.client, namespace, name, &status).await?;
    Ok(Action::await_change())
}

async fn rpc(
    client: &reqwest::Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<Value> {
    let response: Value = client
        .post(rpc_url)
        .json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
        .send()
        .await
        .map_err(config_error)?
        .error_for_status()
        .map_err(config_error)?
        .json()
        .await
        .map_err(config_error)?;
    if let Some(error) = response.get("error") {
        return Err(Error::ConfigError(format!(
            "Soroban RPC {method} failed: {error}"
        )));
    }
    Ok(response.get("result").cloned().unwrap_or(response))
}

async fn submit_signed_transaction(
    client: &reqwest::Client,
    rpc_url: &str,
    transaction_xdr: &str,
) -> Result<String> {
    let result = rpc(
        client,
        rpc_url,
        "sendTransaction",
        json!({"transaction": transaction_xdr}),
    )
    .await?;
    submit_transaction_hash(&result)
}

fn submit_transaction_hash(result: &Value) -> Result<String> {
    match result.get("status").and_then(Value::as_str) {
        Some("PENDING") | Some("DUPLICATE") => result
            .get("hash")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| Error::ConfigError("sendTransaction omitted transaction hash".into())),
        Some("TRY_AGAIN_LATER") => Err(Error::ConfigError(
            "Soroban RPC asked to retry transaction submission later".into(),
        )),
        Some("ERROR") => Err(Error::ConfigError(format!(
            "Soroban RPC rejected signed transaction: {}",
            result
                .get("errorResultXdr")
                .and_then(Value::as_str)
                .unwrap_or("unspecified transaction error")
        ))),
        Some(status) => Err(Error::ConfigError(format!(
            "unexpected sendTransaction status: {status}"
        ))),
        None => Err(Error::ConfigError(
            "sendTransaction response omitted status".into(),
        )),
    }
}

async fn transaction_status(client: &reqwest::Client, rpc_url: &str, hash: &str) -> Result<String> {
    let result = rpc(client, rpc_url, "getTransaction", json!({"hash": hash})).await?;
    Ok(result
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("NOT_FOUND")
        .to_string())
}

async fn ledger_entry_exists(
    client: &reqwest::Client,
    rpc_url: &str,
    key_xdr: &str,
) -> Result<bool> {
    let result = rpc(
        client,
        rpc_url,
        "getLedgerEntries",
        json!({"keys": [key_xdr]}),
    )
    .await?;
    Ok(result
        .get("entries")
        .and_then(Value::as_array)
        .is_some_and(|entries| !entries.is_empty()))
}

async fn patch_wasm_status(
    client: &Client,
    namespace: &str,
    name: &str,
    status: &ContractWASMStatus,
) -> Result<()> {
    let api: Api<ContractWASM> = Api::namespaced(client.clone(), namespace);
    api.patch_status(
        name,
        &PatchParams::default(),
        &Patch::Merge(&json!({"status":status})),
    )
    .await
    .map_err(Error::KubeError)?;
    Ok(())
}

async fn patch_instance_status(
    client: &Client,
    namespace: &str,
    name: &str,
    status: &ContractInstanceStatus,
) -> Result<()> {
    let api: Api<ContractInstance> = Api::namespaced(client.clone(), namespace);
    api.patch_status(
        name,
        &PatchParams::default(),
        &Patch::Merge(&json!({"status":status})),
    )
    .await
    .map_err(Error::KubeError)?;
    Ok(())
}

async fn fail_wasm(
    client: &Client,
    namespace: &str,
    name: &str,
    mut status: ContractWASMStatus,
    message: &str,
) -> Result<Action> {
    status.phase = ContractDeploymentPhase::Failed;
    status.last_error = Some(message.to_string());
    patch_wasm_status(client, namespace, name, &status).await?;
    Ok(Action::await_change())
}

async fn fail_instance(
    client: &Client,
    namespace: &str,
    name: &str,
    mut status: ContractInstanceStatus,
    message: &str,
) -> Result<Action> {
    status.phase = ContractDeploymentPhase::Failed;
    status.last_error = Some(message.to_string());
    patch_instance_status(client, namespace, name, &status).await?;
    Ok(Action::await_change())
}

fn config_error(error: impl std::fmt::Display) -> Error {
    Error::ConfigError(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::submit_transaction_hash;
    use serde_json::json;

    #[test]
    fn recognizes_accepted_duplicate_and_rejected_submissions() {
        assert_eq!(
            submit_transaction_hash(&json!({"status":"PENDING", "hash":"abc"})).unwrap(),
            "abc"
        );
        assert_eq!(
            submit_transaction_hash(&json!({"status":"DUPLICATE", "hash":"abc"})).unwrap(),
            "abc"
        );
        assert!(submit_transaction_hash(&json!({"status":"TRY_AGAIN_LATER"})).is_err());
        assert!(submit_transaction_hash(&json!({"status":"ERROR"})).is_err());
    }
}

fn error_policy_wasm(
    _: Arc<ContractWASM>,
    error: &Error,
    _: Arc<ContractControllerContext>,
) -> Action {
    tracing::warn!(%error, "ContractWASM reconciliation failed");
    Action::requeue(Duration::from_secs(15))
}

fn error_policy_instance(
    _: Arc<ContractInstance>,
    error: &Error,
    _: Arc<ContractControllerContext>,
) -> Action {
    tracing::warn!(%error, "ContractInstance reconciliation failed");
    Action::requeue(Duration::from_secs(15))
}
