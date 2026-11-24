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
use crate::cli::GenerateRunbookArgs;
use crate::runbook::generate_runbook;
use crate::Error;
use kube::api::Api;
use kube::Client;

pub async fn run_generate_runbook(args: GenerateRunbookArgs) -> Result<(), Error> {
    // Create Kubernetes client
    let client = Client::try_default()
        .await
        .map_err(|e| Error::ConfigError(format!("Failed to create Kubernetes client: {e}")))?;

    // Get the StellarNode resource
    let api: Api<crate::crd::StellarNode> = Api::namespaced(client, &args.namespace);
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
