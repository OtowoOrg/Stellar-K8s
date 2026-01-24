use super::{StorageProviderTrait, UploadMetadata};
use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde_json::Value;

pub struct IPFSProvider {
    client: Client,
    api_url: String,
    pinning_service: Option<PinningConfig>,
}

#[derive(Clone)]
pub struct PinningConfig {
    pub service_url: String,
    pub api_key: String,
}

impl IPFSProvider {
    pub fn new(api_url: String, pinning_service: Option<PinningConfig>) -> Self {
        Self {
            client: Client::new(),
            api_url,
            pinning_service,
        }
    }
}

#[async_trait]
impl StorageProviderTrait for IPFSProvider {
    async fn upload(&self, data: Vec<u8>, metadata: UploadMetadata) -> Result<String> {
        // Upload to IPFS
        let form = reqwest::multipart::Form::new()
            .part("file", reqwest::multipart::Part::bytes(data)
                .file_name(metadata.filename.clone()));

        let response: Value = self.client
            .post(format!("{}/api/v0/add", self.api_url))
            .multipart(form)
            .send()
            .await
            .context("Failed to upload to IPFS")?
            .json()
            .await?;

        let cid = response["Hash"].as_str()
            .context("Missing CID in IPFS response")?
            .to_string();

        // Pin if pinning service configured
        if let Some(ref pinning) = self.pinning_service {
            self.pin_to_service(&cid, &metadata, pinning).await?;
        }

        Ok(cid)
    }

    async fn exists(&self, content_hash: &str) -> Result<bool> {
        // Check if CID is pinned locally
        let response = self.client
            .post(format!("{}/api/v0/pin/ls", self.api_url))
            .send()
            .await?;

        let pins: Value = response.json().await?;
        Ok(pins["Keys"].as_object()
            .map(|keys| keys.contains_key(content_hash))
            .unwrap_or(false))
    }

    async fn verify(&self, cid: &str, expected_hash: &str) -> Result<bool> {
        // Cat the file and verify hash
        let data = self.client
            .post(format!("{}/api/v0/cat?arg={}", self.api_url, cid))
            .send()
            .await?
            .bytes()
            .await?;

        let mut hasher = sha2::Sha256::new();
        hasher.update(&data);
        let hash = format!("{:x}", hasher.finalize());

        Ok(hash == expected_hash)
    }
}

impl IPFSProvider {
    async fn pin_to_service(&self, cid: &str, metadata: &UploadMetadata, config: &PinningConfig) -> Result<()> {
        let pin_data = serde_json::json!({
            "cid": cid,
            "name": metadata.filename,
            "meta": {
                "sha256": metadata.sha256,
                "size": metadata.size,
            }
        });

        self.client
            .post(&config.service_url)
            .bearer_auth(&config.api_key)
            .json(&pin_data)
            .send()
            .await
            .context("Failed to pin to service")?;

        Ok(())
    }
}