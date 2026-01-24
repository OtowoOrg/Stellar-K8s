use super::*;
use anyhow::{Context, Result};
use cron::Schedule;
use std::str::FromStr;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn, error};

pub struct BackupScheduler {
    config: DecentralizedBackupConfig,
    provider: Arc<dyn StorageProviderTrait>,
    uploaded_hashes: Arc<RwLock<HashSet<String>>>,
}

impl BackupScheduler {
    pub fn new(
        config: DecentralizedBackupConfig,
        provider: Arc<dyn StorageProviderTrait>,
    ) -> Self {
        Self {
            config,
            provider,
            uploaded_hashes: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub async fn start(&self, history_archive_path: String) -> Result<()> {
        let schedule = Schedule::from_str(&self.config.schedule)
            .context("Invalid cron schedule")?;

        info!("Starting backup scheduler with schedule: {}", self.config.schedule);

        loop {
            let now = chrono::Utc::now();
            let next = schedule.upcoming(chrono::Utc).next()
                .context("No upcoming schedule")?;
            
            let duration = (next - now).to_std()
                .unwrap_or(Duration::from_secs(60));

            info!("Next backup scheduled in {:?}", duration);
            sleep(duration).await;

            if let Err(e) = self.run_backup(&history_archive_path).await {
                error!("Backup failed: {}", e);
            }
        }
    }

    async fn run_backup(&self, archive_path: &str) -> Result<()> {
        info!("Starting backup of history archive: {}", archive_path);

        // Discover new archive segments
        let segments = self.discover_new_segments(archive_path).await?;
        info!("Found {} segments to backup", segments.len());

        // Upload with concurrency control
        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.max_concurrent_uploads
        ));

        let mut tasks = vec![];
        for segment in segments {
            let sem = semaphore.clone();
            let provider = self.provider.clone();
            let uploaded = self.uploaded_hashes.clone();
            let compression = self.config.compression_enabled;

            let task = tokio::spawn(async move {
                let _permit = sem.acquire().await.unwrap();
                Self::upload_segment(segment, provider, uploaded, compression).await
            });

            tasks.push(task);
        }

        let results = futures::future::join_all(tasks).await;
        let successful = results.iter().filter(|r| r.is_ok()).count();
        
        info!("Backup completed: {}/{} successful", successful, results.len());

        Ok(())
    }

    async fn discover_new_segments(&self, _archive_path: &str) -> Result<Vec<ArchiveSegment>> {
        // In production, scan the history archive directory structure
        // History archives follow: /bucket/hex/hex/hex/history-hexhexhex.xdr.gz
        // This is a simplified placeholder
        Ok(vec![])
    }

    async fn upload_segment(
        segment: ArchiveSegment,
        provider: Arc<dyn StorageProviderTrait>,
        uploaded_hashes: Arc<RwLock<HashSet<String>>>,
        compression_enabled: bool,
    ) -> Result<()> {
        // Check if already uploaded (deduplication)
        {
            let hashes = uploaded_hashes.read().await;
            if hashes.contains(&segment.hash) {
                info!("Segment {} already uploaded, skipping", segment.filename);
                return Ok(());
            }
        }

        // Read segment data
        let mut data = tokio::fs::read(&segment.path).await
            .context("Failed to read segment")?;

        // Apply additional compression if enabled and not already compressed
        if compression_enabled && !segment.filename.ends_with(".gz") {
            data = compress_data(&data)?;
        }

        let metadata = UploadMetadata {
            filename: segment.filename.clone(),
            content_type: "application/octet-stream".to_string(),
            size: data.len(),
            sha256: segment.hash.clone(),
            tags: vec![
                ("Ledger".to_string(), segment.ledger.to_string()),
                ("Type".to_string(), segment.segment_type.clone()),
            ],
        };

        // Upload
        let cid = provider.upload(data, metadata).await
            .context("Upload failed")?;

        info!("Uploaded {} -> {}", segment.filename, cid);

        // Mark as uploaded
        {
            let mut hashes = uploaded_hashes.write().await;
            hashes.insert(segment.hash.clone());
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
struct ArchiveSegment {
    filename: String,
    path: String,
    hash: String,
    ledger: u64,
    segment_type: String,
}

fn compress_data(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}
