use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

#[derive(Serialize)]
struct DocEntry {
    path: String,
    title: String,
    content: String,
}

fn main() {
    // Set build date
    println!(
        "cargo:rustc-env=BUILD_DATE={}",
        chrono::Utc::now().to_rfc3339()
    );

    // Set Git SHA
    let git_sha = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_SHA={git_sha}");

    // Set Rust version
    let rust_version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=RUST_VERSION={rust_version}");

    // Index documentation
    index_docs();
}

fn index_docs() {
    let mut docs = Vec::new();
    let root = Path::new(".");

    // Files to index
    let dirs = ["docs"];
    let root_files = ["README.md", "CONTRIBUTING.md", "DEVELOPMENT.md", "SECURITY.md", "CHANGELOG.md"];

    for dir in dirs {
        for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
            if entry.path().extension().map_or(false, |ext| ext == "md") {
                if let Some(doc) = process_file(entry.path(), root) {
                    docs.push(doc);
                }
            }
        }
    }

    for file in root_files {
        let path = root.join(file);
        if path.exists() {
            if let Some(doc) = process_file(&path, root) {
                docs.push(doc);
            }
        }
    }

    let out_dir = std::env::var("OUT_DIR").unwrap();
    let dest_path = Path::new(&out_dir).join("search_index.json");
    let json = serde_json::to_string(&docs).unwrap();
    fs::write(dest_path, json).unwrap();

    // Rerun if any doc changes
    println!("cargo:rerun-if-changed=docs");
    for file in root_files {
        println!("cargo:rerun-if-changed={file}");
    }
}

fn process_file(path: &Path, root: &Path) -> Option<DocEntry> {
    let content = fs::read_to_string(path).ok()?;
    let title = content
        .lines()
        .find(|l| l.starts_with("# "))
        .map(|l| l.trim_start_matches("# ").to_string())
        .unwrap_or_else(|| path.file_stem().unwrap().to_string_lossy().to_string());

    let rel_path = path.strip_prefix(root).ok()?.to_string_lossy().to_string();

    Some(DocEntry {
        path: rel_path,
        title,
        content,
    })
}
