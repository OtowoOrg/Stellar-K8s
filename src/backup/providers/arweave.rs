use super::{StorageProviderTrait, UploadMetadata};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub struct ArweaveProvider {
    client: Client,
    gateway: String,
    wallet_jwk: Value,
}

impl ArweaveProvider {
    pub async fn new(gateway: String, wallet_jwk: Value) -> Result<Self> {
        Ok(Self {
            client: Client::new(),
            gateway,
            wallet_jwk,
        })
    }
}

#[async_trait]
impl StorageProviderTrait for ArweaveProvider {
    async fn upload(&self, data: Vec<u8>, metadata: UploadMetadata) -> Result<String> {
        // Create Arweave transaction
        let mut tags = vec![
            ("Content-Type".to_string(), metadata.content_type.clone()),
            ("File-Name".to_string(), metadata.filename.clone()),
            ("SHA-256".to_string(), metadata.sha256.clone()),
            ("App-Name".to_string(), "Stellar-Archive-Backup".to_string()),
        ];
        tags.extend(metadata.tags);

        // In production, use arweave-rs or bundlr for actual implementation
        // This is a simplified example
        let tx_data = json!({
            "data": base64::encode(&data),
            "tags": tags.iter().map(|(k, v)| json!({
                "name": base64::encode(k),
                "value": base64::encode(v)
            })).collect::<Vec<_>>(),
        });

        // Sign and submit transaction (simplified)
        let response = self.client
            .post(format!("{}/tx", self.gateway))
            .json(&tx_data)
            .send()
            .await
            .context("Failed to submit Arweave transaction")?;

        let tx_id = response.text().await?;
        
        Ok(tx_id)
    }

    async fn exists(&self, content_hash: &str) -> Result<bool> {
        // Query Arweave for existing content with this hash
        let query = json!({
            "query": format!(
                "{{ transactions(tags: [{{name: \"SHA-256\", values: [\"{}\"]}}]) {{ edges {{ node {{ id }} }} }} }}",
                content_hash
            )
        });

        let response: Value = self.client
            .post(format!("{}/graphql", self.gateway))
            .json(&query)
            .send()
            .await?
            .json()
            .await?;

        Ok(!response["data"]["transactions"]["edges"].as_array()
            .map(|a| a.is_empty())
            .unwrap_or(true))
    }

    async fn verify(&self, cid: &str, expected_hash: &str) -> Result<bool> {
        let data = self.client
            .get(format!("{}/{}", self.gateway, cid))
            .send()
            .await?
            .bytes()
            .await?;

        let mut hasher = Sha256::new();
        hasher.update(&data);
        let hash = format!("{:x}", hasher.finalize());

        Ok(hash == expected_hash)
    }
}
