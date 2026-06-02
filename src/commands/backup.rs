use anyhow::{Context, Result};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::backup::providers::{StorageProviderTrait, UploadMetadata};
use crate::backup::*;

#[derive(Parser, Debug)]
pub struct BackupArgs {
    /// Path to the data to backup
    #[arg(short, long)]
    pub source: PathBuf,

    /// Storage backend (file, s3, arweave, ipfs, filecoin
    #[arg(short, long, default_value = "file")]
    pub backend: String,

    /// Destination path or bucket
    #[arg(short, long)]
    pub destination: String,

    /// Enable incremental backup
    #[arg(long)]
    pub incremental: bool,

    /// Verify backup after creation
    #[arg(long)]
    pub verify: bool,
}

#[derive(Parser, Debug)]
pub struct RestoreArgs {
    /// Backup identifier or path to restore from
    #[arg(short, long)]
    pub backup: String,

    /// Destination directory to restore to
    #[arg(short, long)]
    pub destination: PathBuf,

    /// Storage backend (file, s3, arweave, ipfs, filecoin
    #[arg(short, long, default_value = "file")]
    pub backend: String,

    /// Verify restore
    #[arg(long)]
    pub verify: bool,
}

#[derive(Parser, Debug)]
pub struct ListArgs {
    /// Storage backend (file, s3, arweave, ipfs, filecoin
    #[arg(short, long, default_value = "file")]
    pub backend: String,

    /// Location to list backups from
    #[arg(short, long)]
    pub location: String,
}

#[derive(Parser, Debug)]
pub struct CleanupArgs {
    /// Storage backend (file, s3, arweave, ipfs, filecoin
    #[arg(short, long, default_value = "file")]
    pub backend: String,

    /// Location
    #[arg(short, long)]
    pub location: String,

    /// Keep last N backups
    #[arg(long, default_value_t = 10)]
    pub keep: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupMetadata {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub source: String,
    pub size: u64,
    pub checksum: String,
    pub incremental: bool,
    pub files: Vec<String>,
}

pub async fn run_backup(args: BackupArgs) -> Result<()> {
    println!("Starting backup from {:?}", args.source);

    let start = Instant::now();

    // Validate source exists
    if !args.source.exists() {
        return Err(anyhow::anyhow!("Source path does not exist"));
    }

    // Collect files to backup
    let files = collect_files(&args.source)?;
    println!("Found {} files to backup", files.len());

    // Create backup metadata
    let metadata = BackupMetadata {
        timestamp: chrono::Utc::now(),
        source: args.source.to_string_lossy().to_string(),
        size: 0,
        checksum: "".to_string(),
        incremental: args.incremental,
        files: files.iter().map(|p| p.to_string_lossy().to_string()).collect(),
    };

    // TODO: Implement storage backend handling
    match args.backend.as_str() {
        "file" => backup_to_file(&args, &metadata, &files).await?,
        "s3" => backup_to_s3(&args, &metadata, &files).await?,
        "arweave" => backup_to_arweave(&args, &metadata, &files).await?,
        "ipfs" => backup_to_ipfs(&args, &metadata, &files).await?,
        "filecoin" => backup_to_filecoin(&args, &metadata, &files).await?,
        _ => return Err(anyhow::anyhow!("Unsupported backend: {}", args.backend)),
    }

    println!("Backup completed in {:?}", start.elapsed());

    if args.verify {
        println!("Verifying backup...");
        // TODO: Implement verification
    }

    Ok(())
}

pub async fn run_restore(args: RestoreArgs) -> Result<()> {
    println!("Restoring backup {} to {:?}", args.backup, args.destination);

    let start = Instant::now();

    // Create destination directory if it doesn't exist
    fs::create_dir_all(&args.destination)?;

    // TODO: Implement restore based on backend
    match args.backend.as_str() {
        "file" => restore_from_file(&args).await?,
        "s3" => restore_from_s3(&args).await?,
        "arweave" => restore_from_arweave(&args).await?,
        "ipfs" => restore_from_ipfs(&args).await?,
        "filecoin" => restore_from_filecoin(&args).await?,
        _ => return Err(anyhow::anyhow!("Unsupported backend: {}", args.backend)),
    }

    println!("Restore completed in {:?}", start.elapsed());

    Ok(())
}

pub async fn run_list(args: ListArgs) -> Result<()> {
    println!("Listing backups from {}", args.location);

    // TODO: Implement list based on backend
    match args.backend.as_str() {
        "file" => list_from_file(&args).await?,
        "s3" => list_from_s3(&args).await?,
        "arweave" => list_from_arweave(&args).await?,
        "ipfs" => list_from_ipfs(&args).await?,
        "filecoin" => list_from_filecoin(&args).await?,
        _ => return Err(anyhow::anyhow!("Unsupported backend: {}", args.backend)),
    }

    Ok(())
}

pub async fn run_cleanup(args: CleanupArgs) -> Result<()> {
    println!("Cleaning up backups at {}, keeping last {}", args.location, args.keep);

    // TODO: Implement cleanup based on backend
    match args.backend.as_str() {
        "file" => cleanup_from_file(&args).await?,
        "s3" => cleanup_from_s3(&args).await?,
        "arweave" => cleanup_from_arweave(&args).await?,
        "ipfs" => cleanup_from_ipfs(&args).await?,
        "filecoin" => cleanup_from_filecoin(&args).await?,
        _ => return Err(anyhow::anyhow!("Unsupported backend: {}", args.backend)),
    }

    Ok(())
}

fn collect_files(path: &PathBuf) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_files(&path)?);
            } else {
                files.push(path);
            }
        }
    } else {
        files.push(path.clone());
    }
    Ok(files)
}

// File backend implementations
async fn backup_to_file(args: &BackupArgs, metadata: &BackupMetadata, files: &[PathBuf]) -> Result<()> {
    let dest_dir = PathBuf::from(&args.destination);
    fs::create_dir_all(&dest_dir)?;

    let backup_name = format!("backup-{}.tar.gz", metadata.timestamp.format("%Y%m%d%H%M%S"));
    let backup_path = dest_dir.join(&backup_name);

    // Write metadata
    let metadata_path = dest_dir.join(format!("{}.metadata.json", backup_name));
    fs::write(&metadata_path, serde_json::to_string_pretty(metadata)?)?;

    // Create tar.gz
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;
    use tar::Builder;

    let file = fs::File::create(&backup_path)?;
    let enc = GzEncoder::new(file, Compression::default());
    let mut tar = Builder::new(enc);

    for file in files {
        let rel_path = file.strip_prefix(&args.source)?;
        tar.append_path_with_name(file, rel_path)?;
    }

    tar.into_inner()?.finish()?;

    println!("Backup created at {:?}", backup_path);
    Ok(())
}

async fn restore_from_file(args: &RestoreArgs) -> Result<()> {
    let backup_path = PathBuf::from(&args.backup);

    if !backup_path.exists() {
        return Err(anyhow::anyhow!("Backup file not found"));
    }

    // Extract tar.gz
    use flate2::read::GzDecoder;
    use tar::Archive;

    let file = fs::File::open(backup_path)?;
    let dec = GzDecoder::new(file);
    let mut archive = Archive::new(dec);
    archive.unpack(&args.destination)?;

    println!("Restore to {:?}", args.destination);
    Ok(())
}

async fn list_from_file(args: &ListArgs) -> Result<()> {
    let location = PathBuf::from(&args.location);
    if !location.exists() || !location.is_dir() {
        return Err(anyhow::anyhow!("Location is not a directory"));
    }

    let mut backups: Vec<_> = fs::read_dir(location)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("tar.gz"))
        .collect();

    println!("Found {} backups", backups.len());
    for backup in backups {
        println!("  {}", backup.file_name().to_string_lossy());
    }

    Ok(())
}

async fn cleanup_from_file(args: &CleanupArgs) -> Result<()> {
    let location = PathBuf::from(&args.location);
    if !location.exists() || !location.is_dir() {
        return Err(anyhow::anyhow!("Location is not a directory"));
    }

    let mut backups: Vec<_> = fs::read_dir(location)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension().and_then(|ext| ext.to_str()) == Some("tar.gz")
        })
        .collect();

    backups.sort_by_key(|entry| entry.metadata().unwrap().modified().unwrap());
    backups.reverse();

    if backups.len() > args.keep {
        let to_delete = &backups[args.keep..];
        for backup in to_delete {
            fs::remove_file(backup.path())?;
            println!("Deleted {}", backup.file_name().to_string_lossy());
        }
        println!("Deleted {} old backups", to_delete.len());
    } else {
        println!("No backups to delete");
    }

    Ok(())
}

// S3 backend stubs
async fn backup_to_s3(_args: &BackupArgs, _metadata: &BackupMetadata, _files: &[PathBuf]) -> Result<()> {
    println!("S3 backup not fully implemented yet");
    Ok(())
}

async fn restore_from_s3(_args: &RestoreArgs) -> Result<()> {
    println!("S3 restore not fully implemented yet");
    Ok(())
}

async fn list_from_s3(_args: &ListArgs) -> Result<()> {
    println!("S3 list not fully implemented yet");
    Ok(())
}

async fn cleanup_from_s3(_args: &CleanupArgs) -> Result<()> {
    println!("S3 cleanup not fully implemented yet");
    Ok(())
}

// Arweave backend stubs
async fn backup_to_arweave(_args: &BackupArgs, _metadata: &BackupMetadata, _files: &[PathBuf]) -> Result<()> {
    println!("Arweave backup not fully implemented yet");
    Ok(())
}

async fn restore_from_arweave(_args: &RestoreArgs) -> Result<()> {
    println!("Arweave restore not fully implemented yet");
    Ok(())
}

async fn list_from_arweave(_args: &ListArgs) -> Result<()> {
    println!("Arweave list not fully implemented yet");
    Ok(())
}

async fn cleanup_from_arweave(_args: &CleanupArgs) -> Result<()> {
    println!("Arweave cleanup not fully implemented yet");
    Ok(())
}

// IPFS backend stubs
async fn backup_to_ipfs(_args: &BackupArgs, _metadata: &BackupMetadata, _files: &[PathBuf]) -> Result<()> {
    println!("IPFS backup not fully implemented yet");
    Ok(())
}

async fn restore_from_ipfs(_args: &RestoreArgs) -> Result<()> {
    println!("IPFS restore not fully implemented yet");
    Ok(())
}

async fn list_from_ipfs(_args: &ListArgs) -> Result<()> {
    println!("IPFS list not fully implemented yet");
    Ok(())
}

async fn cleanup_from_ipfs(_args: &CleanupArgs) -> Result<()> {
    println!("IPFS cleanup not fully implemented yet");
    Ok(())
}

// Filecoin backend stubs
async fn backup_to_filecoin(_args: &BackupArgs, _metadata: &BackupMetadata, _files: &[PathBuf]) -> Result<()> {
    println!("Filecoin backup not fully implemented yet");
    Ok(())
}

async fn restore_from_filecoin(_args: &RestoreArgs) -> Result<()> {
    println!("Filecoin restore not fully implemented yet");
    Ok(())
}

async fn list_from_filecoin(_args: &ListArgs) -> Result<()> {
    println!("Filecoin list not fully implemented yet");
    Ok(())
}

async fn cleanup_from_filecoin(_args: &CleanupArgs) -> Result<()> {
    println!("Filecoin cleanup not fully implemented yet");
    Ok(())
}
