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
use crate::Error;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::api::Api;

pub async fn run_check_crd() -> Result<(), Error> {
    const EXPECTED_VERSION: &str = "v1alpha1";
    const CRD_NAME: &str = "stellarnodes.stellar.org";

    let client = kube::Client::try_default()
        .await
        .map_err(Error::KubeError)?;
    let crds: Api<CustomResourceDefinition> = Api::all(client);

    let crd = match crds.get(CRD_NAME).await {
        Ok(crd) => crd,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            return Err(Error::ConfigError(format!(
                "StellarNode CRD '{CRD_NAME}' is not installed. Install with: kubectl apply -f config/crd/stellarnode-crd.yaml"
            )));
        }
        Err(e) => return Err(Error::KubeError(e)),
    };

    let versions = crd.spec.versions;
    let installed_versions = versions
        .iter()
        .filter(|v| v.served)
        .map(|v| v.name.clone())
        .collect::<Vec<_>>();

    let expected_present = versions
        .iter()
        .any(|v| v.name == EXPECTED_VERSION && v.served);

    if !expected_present {
        return Err(Error::ConfigError(format!(
            "CRD '{}' is installed but expected served version '{}' is missing. Served versions: {}",
            CRD_NAME,
            EXPECTED_VERSION,
            if installed_versions.is_empty() {
                "<none>".to_string()
            } else {
                installed_versions.join(", ")
            }
        )));
    }

    println!("CRD check passed");
    println!("CRD: {CRD_NAME}");
    println!("Expected version: {EXPECTED_VERSION}");
    println!("Served versions: {}", installed_versions.join(", "));
    Ok(())
}
