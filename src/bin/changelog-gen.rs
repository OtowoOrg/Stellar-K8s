#!/usr/bin/env rust
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
//! Changelog generator from conventional commits
//!
//! Generates CHANGELOG.md organized by commit type (Features, Fixes, etc.)
//! from git log following Conventional Commits format.
//!
//! Usage:
//!   changelog-gen --output CHANGELOG.md --since v0.1.0 --until v0.2.0
//!   changelog-gen --output CHANGELOG.md --range 0.1.0..0.2.0

use chrono::Local;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

#[derive(Debug, Clone)]
struct Commit {
    commit_type: String,
    scope: Option<String>,
    description: String,
    hash: String,
    breaking: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut output_file = "CHANGELOG.md".to_string();
    let mut since_version = None;
    let mut until_version = None;

    // Parse arguments
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--output" => {
                i += 1;
                if i < args.len() {
                    output_file = args[i].clone();
                }
            }
            "--since" => {
                i += 1;
                if i < args.len() {
                    since_version = Some(args[i].clone());
                }
            }
            "--until" => {
                i += 1;
                if i < args.len() {
                    until_version = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    // Fetch commits from git log
    let commits = match fetch_commits(&since_version, &until_version) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error fetching commits: {}", e);
            std::process::exit(1);
        }
    };

    // Organize by type
    let mut grouped: BTreeMap<String, Vec<Commit>> = BTreeMap::new();
    let mut breaking_changes = Vec::new();

    for commit in commits {
        if commit.breaking {
            breaking_changes.push(commit.clone());
        }
        grouped
            .entry(commit.commit_type.clone())
            .or_insert_with(Vec::new)
            .push(commit);
    }

    // Generate changelog
    let version = until_version.unwrap_or_else(|| "Unreleased".to_string());
    let mut changelog = format!(
        "# Changelog\n\n## [{}] - {}\n\n",
        version,
        Local::now().format("%Y-%m-%d")
    );

    // Breaking changes section
    if !breaking_changes.is_empty() {
        changelog.push_str("### ⚠️ Breaking Changes\n\n");
        for commit in breaking_changes {
            changelog.push_str(&format!(
                "- **{}{}**: {} ({})\n",
                commit.commit_type,
                commit.scope.map(|s| format!("({})", s)).unwrap_or_default(),
                commit.description,
                &commit.hash[..7]
            ));
        }
        changelog.push('\n');
    }

    // Organized by type
    let type_display = [
        ("feat", "✨ Features"),
        ("fix", "🐛 Fixes"),
        ("perf", "⚡ Performance"),
        ("docs", "📚 Documentation"),
        ("refactor", "♻️ Refactoring"),
        ("test", "✅ Tests"),
        ("ci", "🔧 CI/CD"),
        ("build", "🏗️ Build"),
        ("chore", "🧹 Chore"),
    ];

    for (commit_type, display_name) in type_display {
        if let Some(commits) = grouped.get(commit_type) {
            changelog.push_str(&format!("### {}\n\n", display_name));
            for commit in commits {
                changelog.push_str(&format!(
                    "- **{}{}**: {} ({})\n",
                    commit.commit_type,
                    commit
                        .scope
                        .as_ref()
                        .map(|s| format!("({})", s))
                        .unwrap_or_default(),
                    commit.description,
                    &commit.hash[..7]
                ));
            }
            changelog.push('\n');
        }
    }

    // Write to file
    match write_changelog(&output_file, &changelog) {
        Ok(_) => println!("✓ Changelog generated: {}", output_file),
        Err(e) => {
            eprintln!("Error writing changelog: {}", e);
            std::process::exit(1);
        }
    }
}

fn fetch_commits(since: &Option<String>, until: &Option<String>) -> Result<Vec<Commit>, String> {
    let mut cmd = Command::new("git");
    cmd.arg("log").arg("--pretty=format:%H|%s|%b");

    if let Some(s) = since {
        cmd.arg(&format!("{}..HEAD", s));
    }

    if let Some(u) = until {
        cmd.arg("--until").arg(u);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("Failed to run git log: {}", e))?;

    if !output.status.success() {
        return Err("git log failed".to_string());
    }

    let stdout = String::from_utf8(output.stdout).map_err(|e| format!("Invalid UTF-8: {}", e))?;

    let mut commits = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('|').collect();
        if parts.len() >= 2 {
            let hash = parts[0].to_string();
            let subject = parts[1];
            let body = if parts.len() > 2 { parts[2] } else { "" };

            if let Some((t, s, d)) = parse_commit_message(subject) {
                let breaking = body.contains("BREAKING CHANGE:");
                commits.push(Commit {
                    commit_type: t,
                    scope: s,
                    description: d,
                    hash,
                    breaking,
                });
            }
        }
    }

    Ok(commits)
}

fn parse_commit_message(subject: &str) -> Option<(String, Option<String>, String)> {
    // Simple parser: type(scope): description
    if let Some(colon_pos) = subject.find(':') {
        let type_scope = &subject[..colon_pos];
        let description = subject[colon_pos + 1..].trim().to_string();

        let (commit_type, scope) = if let Some(paren_pos) = type_scope.find('(') {
            let t = type_scope[..paren_pos].trim();
            let s = type_scope[paren_pos + 1..]
                .trim_end_matches(')')
                .to_string();
            (t.to_string(), Some(s))
        } else {
            (type_scope.trim().to_string(), None)
        };

        return Some((commit_type, scope, description));
    }

    None
}

fn write_changelog(path: &str, content: &str) -> Result<(), String> {
    let mut file = File::create(path).map_err(|e| format!("Failed to create file: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write file: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_feat_with_scope() {
        let result = parse_commit_message("feat(api): add webhook support");
        assert!(result.is_some());
        let (t, s, d) = result.unwrap();
        assert_eq!(t, "feat");
        assert_eq!(s, Some("api".to_string()));
        assert_eq!(d, "add webhook support");
    }

    #[test]
    fn test_parse_fix_without_scope() {
        let result = parse_commit_message("fix: prevent race condition");
        assert!(result.is_some());
        let (t, s, d) = result.unwrap();
        assert_eq!(t, "fix");
        assert_eq!(s, None);
        assert_eq!(d, "prevent race condition");
    }
}
