use crate::cli::GenerateRunbookArgs;
use crate::Error;
use kube::api::Api;
use kube::Client;
use stellar_k8s::runbook::generate_runbook;

pub async fn run_generate_runbook(args: GenerateRunbookArgs) -> Result<(), Error> {
    // Create Kubernetes client
    let client = Client::try_default()
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to create Kubernetes client: {e}")))?;

    // Get the StellarNode resource
    let api: Api<stellar_k8s::crd::StellarNode> = Api::namespaced(client, &args.namespace);
    let node = api
        .get(&args.node_name)
        .await
        .map_err(|_e| Error::NotFound {
            kind: "StellarNode".to_string(),
            name: args.node_name.clone(),
            namespace: args.namespace.clone(),
        })?;

    // Generate the runbook
    let runbook = generate_runbook(&node)?;

    // Output to file or stdout
    if let Some(output_path) = args.output {
        std::fs::write(&output_path, &runbook).map_err(Error::IoError)?;
        println!("Runbook generated successfully: {output_path}");
    } else {
        println!("{runbook}");
    }

    Ok(())
}
