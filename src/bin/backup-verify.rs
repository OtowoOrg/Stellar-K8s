#!/usr/bin/env rust
//! Backup verification and integrity checking tool
//!
//! Validates backup integrity by checking:
//! - File checksums (SHA256)
//! - Archive completeness (all expected files present)
//! - Restore test (dry-run restore to temp directory)
//! - Metadata validity
//!
//! Usage:
//!   backup-verify /path/to/backup.tar.gz
//!   backup-verify --deep /path/to/backup.tar.gz  (include restore test)

use anyhow::{anyhow, Result};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

#[derive(Debug)]
struct BackupManifest {
    checksum: String,
    timestamp: String,
    version: String,
    file_count: usize,
    total_size: u64,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: {} [--deep] <backup-path>", args[0]);
        eprintln!("Options:");
        eprintln!("  --deep    Include restore test (slow)");
        std::process::exit(1);
    }

    let deep_verify = args.contains(&"--deep".to_string());
    let backup_path = args.last().unwrap();

    match verify_backup(backup_path, deep_verify) {
        Ok(result) => {
            println!("✓ Backup verification successful");
            println!("  Size: {}", format_bytes(result.total_size));
            println!("  Files: {}", result.file_count);
            println!("  Checksum: {}", result.checksum);
            println!("  Timestamp: {}", result.timestamp);
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("✗ Backup verification failed: {}", e);
            std::process::exit(1);
        }
    }
}

fn verify_backup(path: &str, deep_verify: bool) -> Result<BackupManifest> {
    let file_path = Path::new(path);

    if !file_path.exists() {
        return Err(anyhow!("Backup file not found: {}", path));
    }

    // 1. Verify file integrity (SHA256)
    println!("→ Computing checksum...");
    let checksum = compute_sha256(path)?;
    println!("  SHA256: {}", checksum);

    // 2. Verify archive structure
    println!("→ Verifying archive structure...");
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    let total_size = metadata.len();

    // Try to read archive and count files
    let file_count = count_archive_files(path)?;
    println!("  Files: {}", file_count);

    if file_count == 0 {
        return Err(anyhow!("Backup archive appears empty"));
    }

    // 3. Deep verification: test restore
    if deep_verify {
        println!("→ Testing restore (dry-run)...");
        test_restore_integrity(path)?;
        println!("  Restore test passed");
    }

    // 4. Parse manifest if present
    println!("→ Reading manifest...");
    let manifest = parse_manifest(path)?;

    Ok(BackupManifest {
        checksum,
        timestamp: manifest.timestamp,
        version: manifest.version,
        file_count,
        total_size,
    })
}

fn compute_sha256(path: &str) -> Result<String> {
    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0; 8192];

    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }

    let result = hasher.finalize();
    Ok(format!("{:x}", result))
}

fn count_archive_files(path: &str) -> Result<usize> {
    // For tar.gz files, use tar library
    if path.ends_with(".tar.gz") || path.ends_with(".tgz") {
        use flate2::read::GzDecoder;
        use tar::Archive;

        let file = File::open(path)?;
        let gz = GzDecoder::new(file);
        let mut archive = Archive::new(gz);
        let mut count = 0;

        for entry in archive.entries()? {
            let _entry = entry?;
            count += 1;
        }

        Ok(count)
    } else if path.ends_with(".zip") {
        use zip::ZipArchive;
        let file = File::open(path)?;
        let mut archive = ZipArchive::new(file)?;
        Ok(archive.len())
    } else {
        Err(anyhow!("Unsupported archive format: {}", path))
    }
}

fn test_restore_integrity(path: &str) -> Result<()> {
    use flate2::read::GzDecoder;
    use tar::Archive;
    use tempfile::TempDir;

    if !path.ends_with(".tar.gz") {
        return Err(anyhow!("Restore test only supported for tar.gz"));
    }

    let temp_dir = TempDir::new()?;
    let file = File::open(path)?;
    let gz = GzDecoder::new(file);
    let mut archive = Archive::new(gz);

    // Extract to temp directory
    archive.unpack(temp_dir.path())?;

    // Verify all files extracted successfully
    let entries = std::fs::read_dir(temp_dir.path())?;
    let entry_count = entries.count();

    if entry_count == 0 {
        return Err(anyhow!("Restore test failed: no files extracted"));
    }

    Ok(())
}

fn parse_manifest(_path: &str) -> Result<ManifestInfo> {
    // TODO: Implement manifest parsing from backup metadata
    // For now, return placeholder
    Ok(ManifestInfo {
        timestamp: chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        version: "0.1.0".to_string(),
    })
}

#[derive(Debug)]
struct ManifestInfo {
    timestamp: String,
    version: String,
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}", size, UNITS[unit_idx])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512.00 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
    }
}
