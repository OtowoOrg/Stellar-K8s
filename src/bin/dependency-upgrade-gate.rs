// Copyright 2026 Stellar-K8s Contributors
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
//! Dependency upgrade gate (#1999).
//!
//! Scans the repository for call sites of a dependency, generates consumer
//! contract tests from them, checks the upgrade against the known breaking
//! changes and exits non-zero when the upgrade is incompatible, naming the
//! consumers and `file:line` call sites responsible.
//!
//! ```text
//! dependency-upgrade-gate --dependency kube --from 0.94.0 --to 0.95.0 \
//!     --pr 123 --breaking-changes config/dependency-breaking-changes.json
//! ```
//!
//! Exit codes: `0` approved, `1` blocked, `2` the gate could not run.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, ValueEnum};

use stellar_k8s::dependency_contract::{
    DependencyKind, SemVer, UpgradeProposal, UpgradeValidator, Verdict,
};
use stellar_k8s::dependency_scan::{parse_breaking_changes, scan_tree};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Kind {
    Library,
    BaseImage,
    Operator,
}

impl From<Kind> for DependencyKind {
    fn from(kind: Kind) -> Self {
        match kind {
            Kind::Library => DependencyKind::Library,
            Kind::BaseImage => DependencyKind::BaseImage,
            Kind::Operator => DependencyKind::Operator,
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "dependency-upgrade-gate",
    about = "Block incompatible dependency upgrades using contract tests generated from call sites"
)]
struct Cli {
    /// Crate (or image) being upgraded.
    #[arg(long)]
    dependency: String,
    /// Version currently in use.
    #[arg(long)]
    from: String,
    /// Proposed version.
    #[arg(long)]
    to: String,
    /// Pull request proposing the upgrade, for attribution.
    #[arg(long, default_value_t = 0)]
    pr: u64,
    #[arg(long, value_enum, default_value_t = Kind::Library)]
    kind: Kind,
    /// Repository root to scan for call sites.
    #[arg(long, default_value = ".")]
    root: PathBuf,
    /// JSON file of known breaking changes: an array, or an object keyed by dependency.
    #[arg(long)]
    breaking_changes: Option<PathBuf>,
    /// Write the full report (verdict, attribution, matrix, artifact) as JSON.
    #[arg(long)]
    json_out: Option<PathBuf>,
    /// Write a Markdown summary, suitable for a PR comment.
    #[arg(long)]
    markdown_out: Option<PathBuf>,
    /// Write the generated contract tests to this file.
    #[arg(long)]
    tests_out: Option<PathBuf>,
}

fn run(cli: &Cli) -> Result<Verdict, String> {
    let from = SemVer::parse(&cli.from).map_err(|e| format!("--from: {e}"))?;
    let to = SemVer::parse(&cli.to).map_err(|e| format!("--to: {e}"))?;

    let breaking_changes = match &cli.breaking_changes {
        Some(path) => {
            let json = std::fs::read_to_string(path)
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            parse_breaking_changes(&json, &cli.dependency)?
        }
        None => Vec::new(),
    };

    let call_sites = scan_tree(&cli.root, &cli.dependency)
        .map_err(|e| format!("cannot scan {}: {e}", cli.root.display()))?;

    let mut builder = UpgradeProposal::builder(&cli.dependency, cli.kind.into())
        .pr(cli.pr)
        .from(&from.to_string())
        .to(&to.to_string());
    for change in breaking_changes {
        builder = builder.breaking_change(change);
    }
    for site in call_sites {
        builder = builder.call_site(site);
    }
    let proposal = builder.build();

    let validator = UpgradeValidator::new();
    let report = validator.validate(&proposal);

    println!(
        "{} {} -> {}: {} call site(s) scanned, verdict {}",
        cli.dependency,
        cli.from,
        cli.to,
        proposal.call_sites.len(),
        report.verdict
    );
    if report.is_blocked() {
        for line in report.block_reason_lines() {
            println!("{line}");
        }
    } else {
        println!("{}", report.reason);
    }

    if let Some(path) = &cli.json_out {
        let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, json).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    if let Some(path) = &cli.markdown_out {
        let mut markdown = format!(
            "### Dependency upgrade gate: `{}` `{}` -> `{}`\n\n**{}**\n\n",
            cli.dependency, cli.from, cli.to, report.verdict
        );
        for line in report.block_reason_lines() {
            markdown.push_str(&format!("{line}\n"));
        }
        markdown.push('\n');
        markdown.push_str(&report.matrix.to_markdown());
        if let Some(artifact) = &report.artifact {
            markdown.push_str("\n\n");
            markdown.push_str(&artifact.to_markdown(&report.matrix));
        }
        std::fs::write(path, markdown)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    if let Some(path) = &cli.tests_out {
        let suite = stellar_k8s::dependency_contract::ContractTestGenerator::new()
            .generate(&proposal.call_sites);
        std::fs::write(path, suite.to_rust_module())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }

    Ok(report.verdict)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(Verdict::Approved) => ExitCode::SUCCESS,
        Ok(Verdict::Blocked) => ExitCode::from(1),
        Err(message) => {
            eprintln!("dependency-upgrade-gate: {message}");
            ExitCode::from(2)
        }
    }
}
