//! doc-check — Stale documentation detector for Stellar-K8s
//!
//! This binary reads `doc-coverage.toml` to discover which documentation files
//! are linked to which source files.  It then uses the Git history to determine
//! whether any source file has been modified more recently than its paired
//! documentation file.  When that is the case the doc is considered **stale**
//! and the tool exits with a non-zero status so that CI or a pre-commit hook
//! can fail fast and notify the author.
//!
//! # Quick start
//!
//! ```text
//! # Check for stale docs against the last commit
//! cargo run --bin doc-check
//!
//! # Check only files that changed in the current PR (CI mode)
//! cargo run --bin doc-check -- --changed-files src/controller/health.rs
//!
//! # Show all mappings without checking staleness
//! cargo run --bin doc-check -- --list
//!
//! # Update the stored baseline hashes (after deliberately updating docs)
//! cargo run --bin doc-check -- --update-baseline
//! ```
//!
//! See `docs/stale-docs-detector.md` for the full user guide.

use std::{
    collections::{HashMap, HashSet},
    fmt, fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Automated stale-documentation detector.
///
/// Reads `doc-coverage.toml` and reports documentation files that have fallen
/// behind their linked source files according to Git history.
#[derive(Parser, Debug)]
#[command(
    name = "doc-check",
    version,
    about = "Detect documentation that has fallen behind its source code"
)]
struct Cli {
    /// Path to the coverage config file.
    #[arg(long, default_value = "doc-coverage.toml")]
    config: PathBuf,

    /// Path to the baseline file that stores the last-known-good commit SHAs.
    #[arg(long, default_value = ".doc-hashes.toml")]
    baseline: PathBuf,

    /// Only consider these source files as changed (space- or newline-separated
    /// list; typically supplied by CI with the files changed in the current PR).
    #[arg(long, value_delimiter = '\n', num_args = 0..)]
    changed_files: Vec<PathBuf>,

    /// Exit with code 0 even when stale docs are found (useful in warn-only mode).
    #[arg(long)]
    warn_only: bool,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Subcommand, Debug)]
enum CliCommand {
    /// Print all doc → source mappings and exit (no staleness check).
    List,
    /// Update the baseline file to mark all docs as current.
    UpdateBaseline,
    /// Print the current status of every entry (stale / ok / missing).
    Status,
}

// ── Config types ──────────────────────────────────────────────────────────────

/// Top-level structure of `doc-coverage.toml`.
#[derive(Deserialize, Debug)]
struct Coverage {
    entry: Vec<Entry>,
}

/// One doc → source mapping from the config file.
#[derive(Deserialize, Debug, Clone)]
struct Entry {
    /// Relative path to the documentation file.
    doc: PathBuf,
    /// One or more source globs (files or directories) this doc covers.
    src: Vec<String>,
    /// When `true` this entry is temporarily excluded from staleness checks.
    #[serde(default)]
    ignore: bool,
}

// ── Baseline (stored hashes) ──────────────────────────────────────────────────

/// Persisted map of `doc_path → last-seen commit SHA when doc was in sync`.
#[derive(Serialize, Deserialize, Debug, Default)]
struct Baseline {
    /// Map from doc path string to a [`BaselineEntry`].
    #[serde(flatten)]
    entries: HashMap<String, BaselineEntry>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct BaselineEntry {
    /// The commit SHA of the most recent source-side change at the time the
    /// doc was last marked as up-to-date.
    last_src_sha: String,
    /// The commit SHA of the doc file at the same point in time.
    last_doc_sha: String,
}

impl Baseline {
    fn load(path: &Path) -> Self {
        let content = fs::read_to_string(path).unwrap_or_default();
        toml::from_str(&content).unwrap_or_default()
    }

    fn save(&self, path: &Path) -> anyhow::Result<()> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }
}

// ── Git helpers ───────────────────────────────────────────────────────────────

/// Return the most recent commit SHA that touched `path`, or `None` if the
/// file is untracked / the repo has no commits yet.
fn git_last_commit_sha(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args([
            "log",
            "--follow",
            "--format=%H",
            "-1",
            "--",
            path.to_string_lossy().as_ref(),
        ])
        .output()
        .ok()?;

    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Return the commit SHA of `HEAD`.
fn git_head_sha() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    let sha = String::from_utf8(output.stdout).ok()?;
    let sha = sha.trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Return `true` if commit `newer` is strictly more recent than `older` in the
/// Git DAG (i.e. `newer` is a descendant of `older`).
///
/// When the two SHAs are equal the function returns `false` (not newer).
/// Falls back to `true` (conservative: assume stale) on any git error.
fn is_newer_commit(newer: &str, older: &str) -> bool {
    if newer == older {
        return false;
    }
    // `git merge-base --is-ancestor older newer` exits 0 if older is an
    // ancestor of newer (i.e. newer is more recent).
    let status = Command::new("git")
        .args(["merge-base", "--is-ancestor", older, newer])
        .status();

    match status {
        Ok(s) => s.success(),
        Err(_) => true, // conservative: treat as stale on error
    }
}

/// Resolve a glob pattern relative to `repo_root` and return all matching
/// regular-file paths.  Directories expand to all files inside them
/// recursively.  Missing paths are silently skipped.
fn resolve_glob(pattern: &str, repo_root: &Path) -> Vec<PathBuf> {
    let full_pattern = repo_root.join(pattern);

    // First try exact path (file or directory).
    let exact = repo_root.join(pattern);
    if exact.is_dir() {
        return walk_dir(&exact);
    }
    if exact.is_file() {
        return vec![exact];
    }

    // Fall back to glob expansion.
    let pattern_str = full_pattern.to_string_lossy();
    match glob::glob(&pattern_str) {
        Ok(paths) => paths
            .filter_map(|p| p.ok())
            .flat_map(|p| if p.is_dir() { walk_dir(&p) } else { vec![p] })
            .collect(),
        Err(_) => vec![],
    }
}

/// Recursively collect all regular files under `dir`.
fn walk_dir(dir: &Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect()
}

// ── Staleness check ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
enum EntryStatus {
    /// The doc is up-to-date relative to all its source files.
    Ok,
    /// One or more source files were committed more recently than the doc.
    Stale {
        doc: PathBuf,
        /// The newest source-side SHA that has no matching doc update.
        newest_src_sha: String,
        /// The SHA of the most recent doc commit (may be `None` for new docs).
        doc_sha: Option<String>,
        /// The source files that caused the staleness.
        offending_src: Vec<PathBuf>,
    },
    /// The doc file itself does not exist on disk.
    MissingDoc { doc: PathBuf },
    /// Silenced by `ignore = true` in the config.
    Ignored,
}

impl fmt::Display for EntryStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryStatus::Ok => write!(f, "ok"),
            EntryStatus::Stale {
                doc,
                newest_src_sha,
                doc_sha,
                offending_src,
            } => {
                write!(
                    f,
                    "STALE  {}  (src @ {}, doc @ {})",
                    doc.display(),
                    &newest_src_sha[..8],
                    doc_sha
                        .as_deref()
                        .map(|s| &s[..8.min(s.len())])
                        .unwrap_or("—"),
                )?;
                for src in offending_src {
                    write!(f, "\n         ↳ {}", src.display())?;
                }
                Ok(())
            }
            EntryStatus::MissingDoc { doc } => {
                write!(f, "MISSING  {}", doc.display())
            }
            EntryStatus::Ignored => write!(f, "ignored"),
        }
    }
}

/// Compute the staleness status for a single `Entry`.
///
/// `changed_files_filter` — when non-empty, only source files present in this
/// set are considered.  This allows CI to restrict the check to the files
/// that actually changed in the current PR.
fn check_entry(
    entry: &Entry,
    repo_root: &Path,
    baseline: &Baseline,
    changed_files_filter: &HashSet<PathBuf>,
) -> EntryStatus {
    if entry.ignore {
        return EntryStatus::Ignored;
    }

    let doc_abs = repo_root.join(&entry.doc);
    if !doc_abs.exists() {
        return EntryStatus::MissingDoc {
            doc: entry.doc.clone(),
        };
    }

    // Resolve the doc SHA.
    let doc_sha = git_last_commit_sha(&doc_abs);

    // Collect all concrete source files matched by this entry's globs.
    let src_files: Vec<PathBuf> = entry
        .src
        .iter()
        .flat_map(|pattern| resolve_glob(pattern, repo_root))
        .collect();

    // If a filter is active, restrict to files in the filter.
    let effective_src: Vec<&PathBuf> = if changed_files_filter.is_empty() {
        src_files.iter().collect()
    } else {
        src_files
            .iter()
            .filter(|p| {
                // Compare by stripping the repo_root prefix so that relative
                // paths in --changed-files match absolute paths on disk.
                let rel = p.strip_prefix(repo_root).unwrap_or(p);
                changed_files_filter.contains(rel) || changed_files_filter.contains(p.as_path())
            })
            .collect()
    };

    if effective_src.is_empty() {
        // No src files to check (either glob matched nothing or filter excluded all).
        return EntryStatus::Ok;
    }

    // Find the newest src commit.
    let mut newest_src_sha: Option<String> = None;
    let mut offending: Vec<PathBuf> = vec![];

    for src_path in &effective_src {
        let src_sha = match git_last_commit_sha(src_path) {
            Some(s) => s,
            None => continue, // untracked – skip
        };

        // Is this src commit newer than the doc commit?
        let stale = match &doc_sha {
            Some(d) => is_newer_commit(&src_sha, d),
            // Doc has never been committed → always stale when src exists.
            None => true,
        };

        if stale {
            offending.push((*src_path).clone());

            // Track the newest offending SHA.
            newest_src_sha = Some(match &newest_src_sha {
                None => src_sha.clone(),
                Some(prev) => {
                    if is_newer_commit(&src_sha, prev) {
                        src_sha.clone()
                    } else {
                        prev.clone()
                    }
                }
            });
        }
    }

    // Additionally check the baseline: if the doc was previously recorded as
    // in-sync, verify that the doc hasn't regressed since then.
    let doc_key = entry.doc.to_string_lossy().to_string();
    if let Some(bl) = baseline.entries.get(&doc_key) {
        if let Some(ref current_doc_sha) = doc_sha {
            if is_newer_commit(&bl.last_src_sha, current_doc_sha) {
                // A source commit recorded in the baseline is newer than the
                // current doc commit — the doc regressed.
                offending.push(PathBuf::from("<baseline record>"));
                newest_src_sha = Some(bl.last_src_sha.clone());
            }
        }
    }

    if offending.is_empty() {
        EntryStatus::Ok
    } else {
        EntryStatus::Stale {
            doc: entry.doc.clone(),
            newest_src_sha: newest_src_sha.unwrap_or_else(|| "unknown".into()),
            doc_sha,
            offending_src: offending,
        }
    }
}

// ── Baseline update ───────────────────────────────────────────────────────────

/// Write a fresh baseline that marks every entry as current at HEAD.
fn update_baseline(entries: &[Entry], repo_root: &Path, baseline_path: &Path) -> anyhow::Result<()> {
    let head = git_head_sha().unwrap_or_else(|| "unknown".into());
    let mut baseline = Baseline::default();

    for entry in entries {
        if entry.ignore {
            continue;
        }
        let doc_abs = repo_root.join(&entry.doc);
        let doc_sha = git_last_commit_sha(&doc_abs)
            .unwrap_or_else(|| head.clone());

        baseline.entries.insert(
            entry.doc.to_string_lossy().to_string(),
            BaselineEntry {
                last_src_sha: head.clone(),
                last_doc_sha: doc_sha,
            },
        );
    }

    baseline.save(baseline_path)?;
    println!(
        "✓ Baseline updated for {} entries at HEAD {}",
        baseline.entries.len(),
        &head[..8.min(head.len())]
    );
    Ok(())
}

// ── Report formatting ─────────────────────────────────────────────────────────

struct Report {
    stale: Vec<EntryStatus>,
    missing: Vec<EntryStatus>,
    ok_count: usize,
    ignored_count: usize,
}

impl Report {
    fn new(statuses: Vec<EntryStatus>) -> Self {
        let mut stale = vec![];
        let mut missing = vec![];
        let mut ok_count = 0usize;
        let mut ignored_count = 0usize;

        for s in statuses {
            match &s {
                EntryStatus::Ok => ok_count += 1,
                EntryStatus::Ignored => ignored_count += 1,
                EntryStatus::MissingDoc { .. } => missing.push(s),
                EntryStatus::Stale { .. } => stale.push(s),
            }
        }

        Self {
            stale,
            missing,
            ok_count,
            ignored_count,
        }
    }

    fn has_problems(&self) -> bool {
        !self.stale.is_empty() || !self.missing.is_empty()
    }

    fn print(&self) {
        if self.missing.is_empty() && self.stale.is_empty() {
            println!(
                "\n✅  All {} tracked doc(s) are up-to-date ({} ignored).",
                self.ok_count, self.ignored_count
            );
            return;
        }

        if !self.missing.is_empty() {
            println!("\n🔴  Missing documentation files:");
            for s in &self.missing {
                println!("    {s}");
            }
        }

        if !self.stale.is_empty() {
            println!("\n🟡  Stale documentation files:");
            for s in &self.stale {
                println!("    {s}");
            }
            println!();
            println!("  → Source file(s) above were committed after their linked doc.");
            println!("  → Update the listed doc(s) and commit, or run:");
            println!("      make check-stale-docs");
            println!("  → To record the current state as baseline after intentional updates:");
            println!("      cargo run --bin doc-check -- update-baseline");
        }

        println!(
            "\n  Summary: {} stale, {} missing, {} ok, {} ignored",
            self.stale.len(),
            self.missing.len(),
            self.ok_count,
            self.ignored_count,
        );
    }
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Discover repo root (current working directory is assumed to be the repo
    // root; if not, walk upwards until we find Cargo.toml).
    let repo_root = find_repo_root().unwrap_or_else(|| PathBuf::from("."));

    // Load the coverage config.
    let config_path = repo_root.join(&cli.config);
    let config_content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: cannot read config file '{}': {e}",
                config_path.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let coverage: Coverage = match toml::from_str(&config_content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: failed to parse '{}': {e}",
                config_path.display()
            );
            return ExitCode::FAILURE;
        }
    };

    let baseline_path = repo_root.join(&cli.baseline);

    // Dispatch subcommands.
    match cli.command {
        Some(CliCommand::List) => {
            println!("doc-coverage.toml — {} entries:\n", coverage.entry.len());
            for entry in &coverage.entry {
                let flag = if entry.ignore { " [ignored]" } else { "" };
                println!("  {}{flag}", entry.doc.display());
                for src in &entry.src {
                    println!("    ↳ {src}");
                }
            }
            return ExitCode::SUCCESS;
        }

        Some(CliCommand::UpdateBaseline) => {
            match update_baseline(&coverage.entry, &repo_root, &baseline_path) {
                Ok(()) => return ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("error: failed to update baseline: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }

        Some(CliCommand::Status) | None => {
            // Fall through to the staleness check below.
        }
    }

    // Build the changed-files filter (normalised to repo-relative paths).
    let changed_files_filter: HashSet<PathBuf> = cli
        .changed_files
        .iter()
        .map(|p| {
            let abs = if p.is_absolute() {
                p.clone()
            } else {
                repo_root.join(p)
            };
            // Normalise to repo-relative.
            abs.strip_prefix(&repo_root)
                .map(|r| r.to_path_buf())
                .unwrap_or(p.clone())
        })
        .collect();

    // Load the baseline.
    let baseline = Baseline::load(&baseline_path);

    // Evaluate every entry.
    let statuses: Vec<EntryStatus> = coverage
        .entry
        .iter()
        .map(|entry| check_entry(entry, &repo_root, &baseline, &changed_files_filter))
        .collect();

    // Build and print the report.
    let report = Report::new(statuses);
    report.print();

    if report.has_problems() {
        if cli.warn_only {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        }
    } else {
        ExitCode::SUCCESS
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Walk upwards from the current directory until we find a `Cargo.toml`.
fn find_repo_root() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join("Cargo.toml").exists() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn write_file(dir: &Path, rel: &str, content: &str) -> PathBuf {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parse_coverage_toml() {
        let raw = r#"
[[entry]]
doc = "docs/foo.md"
src = ["src/foo.rs"]

[[entry]]
doc = "docs/bar.md"
src = ["src/bar.rs", "src/baz.rs"]
ignore = true
"#;
        let cov: Coverage = toml::from_str(raw).unwrap();
        assert_eq!(cov.entry.len(), 2);
        assert!(!cov.entry[0].ignore);
        assert!(cov.entry[1].ignore);
    }

    #[test]
    fn baseline_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("baseline.toml");

        let mut bl = Baseline::default();
        bl.entries.insert(
            "docs/foo.md".to_string(),
            BaselineEntry {
                last_src_sha: "abc123".to_string(),
                last_doc_sha: "def456".to_string(),
            },
        );
        bl.save(&path).unwrap();

        let loaded = Baseline::load(&path);
        let entry = loaded.entries.get("docs/foo.md").unwrap();
        assert_eq!(entry.last_src_sha, "abc123");
        assert_eq!(entry.last_doc_sha, "def456");
    }

    #[test]
    fn ignored_entry_always_ok() {
        let entry = Entry {
            doc: PathBuf::from("docs/ignored.md"),
            src: vec!["src/something.rs".to_string()],
            ignore: true,
        };
        let dir = TempDir::new().unwrap();
        let status = check_entry(&entry, dir.path(), &Baseline::default(), &HashSet::new());
        assert_eq!(status, EntryStatus::Ignored);
    }

    #[test]
    fn missing_doc_detected() {
        let dir = TempDir::new().unwrap();
        // Write a src file but not the doc file.
        write_file(dir.path(), "src/foo.rs", "fn foo() {}");

        let entry = Entry {
            doc: PathBuf::from("docs/foo.md"),
            src: vec!["src/foo.rs".to_string()],
            ignore: false,
        };
        let status = check_entry(&entry, dir.path(), &Baseline::default(), &HashSet::new());
        assert!(matches!(status, EntryStatus::MissingDoc { .. }));
    }

    #[test]
    fn ok_when_no_src_files_exist() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "docs/foo.md", "# Foo");

        let entry = Entry {
            doc: PathBuf::from("docs/foo.md"),
            src: vec!["src/does_not_exist.rs".to_string()],
            ignore: false,
        };
        // No source files → nothing to be stale against.
        let status = check_entry(&entry, dir.path(), &Baseline::default(), &HashSet::new());
        assert_eq!(status, EntryStatus::Ok);
    }

    #[test]
    fn filter_excludes_unrelated_files() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "docs/foo.md", "# Foo");
        write_file(dir.path(), "src/foo.rs", "fn foo() {}");

        let entry = Entry {
            doc: PathBuf::from("docs/foo.md"),
            src: vec!["src/foo.rs".to_string()],
            ignore: false,
        };

        // Filter contains an unrelated file → src/foo.rs is excluded.
        let mut filter = HashSet::new();
        filter.insert(PathBuf::from("src/unrelated.rs"));

        let status = check_entry(&entry, dir.path(), &Baseline::default(), &filter);
        assert_eq!(status, EntryStatus::Ok);
    }

    #[test]
    fn resolve_glob_directory() {
        let dir = TempDir::new().unwrap();
        write_file(dir.path(), "src/a.rs", "");
        write_file(dir.path(), "src/b.rs", "");

        let files = resolve_glob("src", dir.path());
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn report_all_ok() {
        let statuses = vec![EntryStatus::Ok, EntryStatus::Ok, EntryStatus::Ignored];
        let report = Report::new(statuses);
        assert!(!report.has_problems());
        assert_eq!(report.ok_count, 2);
        assert_eq!(report.ignored_count, 1);
    }
}
