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
use super::{StorageProviderTrait, UploadMetadata};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

pub struct FilecoinProvider {
    client: Client,
    lotus_api: String,
    wallet_address: String,
}

impl FilecoinProvider {
    pub fn new(lotus_api: String, wallet_address: String) -> Self {
        Self {
            client: Client::new(),
            lotus_api,
            wallet_address,
        }
    }
}

#[async_trait]
impl StorageProviderTrait for FilecoinProvider {
    async fn upload(&self, data: Vec<u8>, metadata: UploadMetadata) -> Result<String> {
        use base64::Engine;
        let import_data = serde_json::json!({
            "data": base64::engine::general_purpose::STANDARD.encode(&data),
            "wallet": self.wallet_address,
            "filename": metadata.filename,
        });

        let response: Value = self
            .client
            .post(format!("{}/api/v0/client/import", self.lotus_api))
            .json(&import_data)
            .send()
            .await
            .context("Failed to import data to Filecoin")?
            .json()
            .await?;

        let cid = response["Root"]["/"]
            .as_str()
            .context("Missing CID in Filecoin response")?
            .to_string();

        Ok(cid)
    }

    async fn exists(&self, content_hash: &str) -> Result<bool> {
        let response: Value = self
            .client
            .post(format!("{}/api/v0/client/has-local", self.lotus_api))
            .json(&serde_json::json!({ "cid": content_hash }))
            .send()
            .await?
            .json()
            .await?;

        Ok(response["result"].as_bool().unwrap_or(false))
    }

    async fn verify(&self, cid: &str, expected_hash: &str) -> Result<bool> {
        let data = self
            .client
            .post(format!("{}/api/v0/client/retrieve", self.lotus_api))
            .json(&serde_json::json!({ "cid": cid }))
            .send()
            .await?
            .bytes()
            .await?;

        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&data);
        let hash = format!("{:x}", hasher.finalize());

        Ok(hash == expected_hash)
    }
}
