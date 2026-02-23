pub mod arweave;
pub mod filecoin;
pub mod ipfs;

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait StorageProviderTrait: Send + Sync {
    /// Upload data and return the content identifier
    async fn upload(&self, data: Vec<u8>, metadata: UploadMetadata) -> Result<String>;

    /// Check if content exists (for deduplication)
    async fn exists(&self, content_hash: &str) -> Result<bool>;

    /// Verify uploaded content
    async fn verify(&self, cid: &str, expected_hash: &str) -> Result<bool>;
}

#[derive(Debug, Clone)]
pub struct UploadMetadata {
    pub filename: String,
    pub content_type: String,
    pub size: usize,
    pub sha256: String,
    pub tags: Vec<(String, String)>,
}
