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
use anyhow::Result;
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use crate::backup::providers::StorageProviderTrait;
use crate::error::diagnostic;

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

    /// Storage backend (file, s3, arweave, ipfs, filecoin)
    // long-only: short `-b` is already used by `--backup`
    #[arg(long, default_value = "file")]
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
        return Err(anyhow::anyhow!(
            "{}",
            diagnostic(
                "validate source",
                format!("path does not exist: {}", args.source.display())
            )
        ));
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
        files: files
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect(),
    };

    let backup_path = PathBuf::from(&args.destination).join(format!(
        "backup-{}.tar.gz",
        metadata.timestamp.format("%Y%m%d%H%M%S")
    ));

    // Storage backend handling - only file and s3 are supported
    match args.backend.as_str() {
        "file" => backup_to_file(&args, &metadata, &files).await?,
        "s3" => backup_to_s3(&args, &metadata, &files).await?,
        // Deprecated: arweave, ipfs, and filecoin backends removed in cleanup wave
        "arweave" | "ipfs" | "filecoin" => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "backend deprecated",
                    format!(
                        "backend {:?} has been removed; supported backends: file, s3",
                        args.backend
                    )
                )
            ))
        }
        _ => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "select backend",
                    format!(
                        "unsupported backend {:?}; expected file or s3",
                        args.backend
                    )
                )
            ))
        }
    }

    println!("Backup completed in {:?}", start.elapsed());

    if args.verify {
        println!("Verifying backup...");
        // Verify the most recent backup in destination (file or directory)
        let dest = PathBuf::from(&args.destination);
        let verify_target = if dest.is_dir() {
            // Find latest tar.gz in destination
            let mut archives: Vec<PathBuf> = fs::read_dir(&dest)
                .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|e| e=="gz").unwrap_or(false)).collect())
                .unwrap_or_default();
            archives.sort();
            archives.last().cloned().unwrap_or(dest)
        } else {
            dest
        };
        verify_backup_integrity(&verify_target.to_string_lossy()).await?;
        println!("✓ Backup verification passed");
        if args.backend == "file" {
            println!("Verifying backup...");
            verify_backup_integrity(&backup_path.to_string_lossy()).await?;
            println!("✓ Backup verification passed");
        } else {
            println!(
                "Skipping local verification: backend {:?} does not produce a local archive",
                args.backend
            );
        }
    }

    Ok(())
}

/// Verify backup integrity with checksum and structure validation
async fn verify_backup_integrity(backup_path: &str) -> Result<()> {
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::io::Read;

    let file = std::fs::File::open(backup_path)?;
    let mut reader = std::io::BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    let checksum = format!("{:x}", hasher.finalize());
    println!("  Checksum: {}", checksum);

    // Try to list archive contents
    if backup_path.ends_with(".tar.gz") {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let file = File::open(backup_path)?;
        let gz = GzDecoder::new(file);
        let mut archive = Archive::new(gz);
        let mut count = 0;

        for entry in archive.entries()? {
            let _entry = entry?;
            count += 1;
        }

        println!("  Files: {}", count);
        if count == 0 {
            return Err(anyhow::anyhow!("Backup appears empty"));
        }
    }

    Ok(())
}

pub async fn run_restore(args: RestoreArgs) -> Result<()> {
    println!("Restoring backup {} to {:?}", args.backup, args.destination);

    let start = Instant::now();

    // Create destination directory if it doesn't exist
    fs::create_dir_all(&args.destination)?;

    // TODO(exempt: pending storage backends): Implement restore based on backend
    match args.backend.as_str() {
        "file" => restore_from_file(&args).await?,
        "s3" => restore_from_s3(&args).await?,
        // Deprecated: arweave, ipfs, and filecoin backends removed in cleanup wave
        "arweave" | "ipfs" | "filecoin" => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "backend deprecated",
                    format!(
                        "backend {:?} has been removed; supported backends: file, s3",
                        args.backend
                    )
                )
            ))
        }
        _ => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "select backend",
                    format!(
                        "unsupported backend {:?}; expected file or s3",
                        args.backend
                    )
                )
            ))
        }
    }

    println!("Restore completed in {:?}", start.elapsed());

    Ok(())
}

pub async fn run_list(args: ListArgs) -> Result<()> {
    println!("Listing backups from {}", args.location);

    // TODO(exempt: pending storage backends): Implement list based on backend
    match args.backend.as_str() {
        "file" => list_from_file(&args).await?,
        "s3" => list_from_s3(&args).await?,
        // Deprecated: arweave, ipfs, and filecoin backends removed in cleanup wave
        "arweave" | "ipfs" | "filecoin" => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "backend deprecated",
                    format!(
                        "backend {:?} has been removed; supported backends: file, s3",
                        args.backend
                    )
                )
            ))
        }
        _ => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "select backend",
                    format!(
                        "unsupported backend {:?}; expected file or s3",
                        args.backend
                    )
                )
            ))
        }
    }

    Ok(())
}

pub async fn run_cleanup(args: CleanupArgs) -> Result<()> {
    println!(
        "Cleaning up backups at {}, keeping last {}",
        args.location, args.keep
    );

    // Only file and s3 are supported
    match args.backend.as_str() {
        "file" => cleanup_from_file(&args).await?,
        "s3" => cleanup_from_s3(&args).await?,
        // Deprecated: arweave, ipfs, and filecoin backends removed in cleanup wave
        "arweave" | "ipfs" | "filecoin" => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "backend deprecated",
                    format!(
                        "backend {:?} has been removed; supported backends: file, s3",
                        args.backend
                    )
                )
            ))
        }
        _ => {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "select backend",
                    format!(
                        "unsupported backend {:?}; expected file or s3",
                        args.backend
                    )
                )
            ))
        }
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
async fn backup_to_file(
    args: &BackupArgs,
    metadata: &BackupMetadata,
    files: &[PathBuf],
) -> Result<()> {
    let dest_dir = PathBuf::from(&args.destination);
    fs::create_dir_all(&dest_dir)?;

    let backup_name = format!(
        "backup-{}.tar.gz",
        metadata.timestamp.format("%Y%m%d%H%M%S")
    );
    let backup_path = dest_dir.join(&backup_name);

    // Write metadata
    let metadata_path = dest_dir.join(format!("{}.metadata.json", backup_name));
    fs::write(&metadata_path, serde_json::to_string_pretty(metadata)?)?;

    // Create tar.gz
    use flate2::write::GzEncoder;
    use flate2::Compression;

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
    let mut backup_path = PathBuf::from(&args.backup);

    // Accept a directory containing backups: restore the most recent archive.
    if backup_path.is_dir() {
        let mut archives: Vec<PathBuf> = fs::read_dir(&backup_path)?
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(|name| name.ends_with(".tar.gz"))
                    .unwrap_or(false)
            })
            .collect();
        if archives.is_empty() {
            return Err(anyhow::anyhow!(
                "{}",
                diagnostic(
                    "open backup archive",
                    format!("no backup archives found in {}", backup_path.display())
                )
            ));
        }
        archives.sort();
        backup_path = archives
            .pop()
            .expect("archives is non-empty, checked above");
    }

    if !backup_path.exists() {
        return Err(anyhow::anyhow!(
            "{}",
            diagnostic(
                "open backup archive",
                format!("backup file not found: {}", backup_path.display())
            )
        ));
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
        return Err(anyhow::anyhow!(
            "{}",
            diagnostic(
                "list backups",
                format!("location is not a directory: {}", location.display())
            )
        ));
    }

    let backups: Vec<_> = fs::read_dir(location)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".tar.gz"))
                .unwrap_or(false)
        })
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
        return Err(anyhow::anyhow!(
            "{}",
            diagnostic(
                "cleanup backups",
                format!("location is not a directory: {}", location.display())
            )
        ));
    }

    let mut backups: Vec<_> = fs::read_dir(location)?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry
                .path()
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.ends_with(".tar.gz"))
                .unwrap_or(false)
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
async fn backup_to_s3(
    _args: &BackupArgs,
    _metadata: &BackupMetadata,
    _files: &[PathBuf],
) -> Result<()> {
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
async fn backup_to_arweave(
    _args: &BackupArgs,
    _metadata: &BackupMetadata,
    _files: &[PathBuf],
) -> Result<()> {
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
async fn backup_to_ipfs(
    _args: &BackupArgs,
    _metadata: &BackupMetadata,
    _files: &[PathBuf],
) -> Result<()> {
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
async fn backup_to_filecoin(
    _args: &BackupArgs,
    _metadata: &BackupMetadata,
    _files: &[PathBuf],
) -> Result<()> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::diagnostic;

    #[test]
    fn backup_source_missing_error_names_step() {
        let args = BackupArgs {
            source: PathBuf::from("/definitely/missing/stellar-backup-source"),
            backend: "file".to_string(),
            destination: "/tmp/out".to_string(),
            incremental: false,
            verify: false,
        };

        let err = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(run_backup(args))
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("[validate source]"));
        assert!(msg.contains("path does not exist"));
    }

    #[test]
    fn diagnostic_format_matches_shell_style() {
        assert_eq!(
            diagnostic("format check", "code is not formatted"),
            "[format check] code is not formatted"
        );
    }
}
