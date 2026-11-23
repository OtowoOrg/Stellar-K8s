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
//! Call-site discovery for dependency upgrade validation (#1999).
//!
//! [`crate::dependency_contract`] turns call sites into contract tests and
//! blocks incompatible upgrades. This module finds the call sites by reading
//! the repository's own source, so the generated suite is derived from the code
//! that really calls the dependency rather than from a hand-kept list.
//!
//! A *call site* is a line that reaches into the dependency by path:
//!
//! - `use kube::runtime::Controller;`
//! - `use kube::{Api, Client, runtime::{watcher, Controller}};` (also across
//!   several lines)
//! - `kube::Client::try_default()`
//!
//! Each becomes one [`CallSite`] carrying the `file:line` and the full API
//! paths it reaches (`kube::Api`, `kube::runtime::Controller`, ...). Comments
//! are ignored. Later uses of an already imported name (`Api::namespaced(..)`)
//! are covered by the import line that brought it in.
//!
//! The consumer a call site is attributed to comes from where it lives:
//!
//! | Path | Consumer |
//! |------|----------|
//! | `src/main.rs` | `stellar-operator` |
//! | `src/bin/<name>.rs` | `<name>` |
//! | `src/<module>/…` or `src/<module>.rs` | `stellar-operator::<module>` |
//! | `tests/…`, `benches/…` | `tests::<file>`, `benches::<file>` |

use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;

use crate::dependency_contract::{BreakingChange, CallSite};

/// Directories never scanned.
const SKIPPED_DIRS: &[&str] = &["target", ".git", "node_modules", "vendor"];

/// Name used for the operator crate's own `src/` tree.
pub const OPERATOR_CONSUMER: &str = "stellar-operator";

/// The identifier a crate is imported as: `serde_yaml` for `serde-yaml`.
pub fn crate_ident(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

/// Consumer attributed to a source file, from its path relative to the
/// repository root (forward slashes).
pub fn consumer_for(relative_path: &str) -> String {
    let parts: Vec<&str> = relative_path.split('/').collect();
    let stem = |name: &str| name.trim_end_matches(".rs").to_string();
    match parts.as_slice() {
        ["src", "main.rs"] => OPERATOR_CONSUMER.to_string(),
        ["src", "bin", file] => stem(file),
        ["src", "bin", dir, ..] => (*dir).to_string(),
        ["src", file] if file.ends_with(".rs") => {
            format!("{OPERATOR_CONSUMER}::{}", stem(file))
        }
        ["src", module, ..] => format!("{OPERATOR_CONSUMER}::{module}"),
        ["tests", rest @ ..] if !rest.is_empty() => {
            format!("tests::{}", stem(rest[rest.len() - 1]))
        }
        ["benches", rest @ ..] if !rest.is_empty() => {
            format!("benches::{}", stem(rest[rest.len() - 1]))
        }
        [first, ..] => (*first).to_string(),
        [] => OPERATOR_CONSUMER.to_string(),
    }
}

/// Split `text` on commas that are not inside braces.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (i, c) in text.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&text[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// Expand a `use` brace group under `prefix` into full paths.
///
/// `expand("kube", "Api, runtime::{watcher, Controller}")` yields
/// `kube::Api`, `kube::runtime::watcher`, `kube::runtime::Controller`.
/// `self` maps to the prefix itself and `as` renames are dropped.
pub fn expand_group(prefix: &str, group: &str) -> Vec<String> {
    let mut out = Vec::new();
    for item in split_top_level(group) {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        if let Some(open) = item.find('{') {
            let head = item[..open].trim().trim_end_matches("::");
            let inner = item[open + 1..].trim_end().trim_end_matches('}');
            let nested_prefix = if head.is_empty() {
                prefix.to_string()
            } else {
                format!("{prefix}::{head}")
            };
            out.extend(expand_group(&nested_prefix, inner));
            continue;
        }
        let path = item.split(" as ").next().unwrap_or(item).trim();
        if path == "self" {
            out.push(prefix.to_string());
        } else if !path.is_empty() {
            out.push(format!("{prefix}::{path}"));
        }
    }
    out
}

/// The text between a leading `{` and its matching `}`. When the brace is
/// never closed, everything after the opening brace.
fn group_inner(group: &str) -> &str {
    let mut depth = 0usize;
    for (i, c) in group.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return &group[1..i];
                }
            }
            _ => {}
        }
    }
    group.get(1..).unwrap_or("")
}

/// Strip a trailing `//` comment, leaving string contents that contain `//`
/// alone only when the comment marker is clearly outside a literal.
fn strip_line_comment(line: &str) -> &str {
    let mut in_string = false;
    let mut prev = '\0';
    for (i, c) in line.char_indices() {
        match c {
            '"' if prev != '\\' => in_string = !in_string,
            '/' if !in_string && prev == '/' => return &line[..i - 1],
            _ => {}
        }
        prev = c;
    }
    line
}

/// Find the call sites of `crate_name` in one source file.
///
/// `file` is the path recorded in each site (relative to the repository root).
pub fn scan_source(consumer: &str, file: &str, source: &str, crate_name: &str) -> Vec<CallSite> {
    let ident = crate_ident(crate_name);
    // Group 1: complete path segments after the crate (`runtime::`), possibly
    // none. Group 2: either a `{` opening a use-group or the final segment.
    let pattern = format!(
        r"\b{}::((?:[A-Za-z_][A-Za-z0-9_]*::)*)(\{{|[A-Za-z_][A-Za-z0-9_]*)",
        regex::escape(&ident)
    );
    let Ok(re) = Regex::new(&pattern) else {
        return Vec::new();
    };

    let lines: Vec<&str> = source.lines().collect();
    let mut sites = Vec::new();
    let mut in_block_comment = false;
    let mut skip_until = 0usize;

    for (index, raw) in lines.iter().enumerate() {
        if index < skip_until {
            continue;
        }
        // Block comments: drop the commented span, keep any code around it.
        let mut code = String::new();
        let mut rest: &str = raw;
        loop {
            if in_block_comment {
                match rest.find("*/") {
                    Some(end) => {
                        in_block_comment = false;
                        rest = &rest[end + 2..];
                    }
                    None => break,
                }
            } else {
                match rest.find("/*") {
                    Some(start) => {
                        code.push_str(&rest[..start]);
                        in_block_comment = true;
                        rest = &rest[start + 2..];
                    }
                    None => {
                        code.push_str(rest);
                        break;
                    }
                }
            }
        }
        let code = strip_line_comment(&code).to_string();
        if code.trim().is_empty() {
            continue;
        }

        let mut apis: Vec<String> = Vec::new();
        for capture in re.captures_iter(&code) {
            let segments = capture.get(1).map(|m| m.as_str()).unwrap_or("");
            let matched = capture.get(2).map(|m| m.as_str()).unwrap_or("");
            if matched == "{" {
                // `ident::path::{ ... }`: gather the group, which may span lines.
                let prefix = format!("{ident}::{segments}");
                let prefix = prefix.trim_end_matches("::").to_string();
                let brace_at = capture.get(2).map(|m| m.start()).unwrap_or(0);
                let mut group = code[brace_at..].to_string();
                let mut consumed = index;
                let mut depth: i32 = group
                    .chars()
                    .map(|c| match c {
                        '{' => 1,
                        '}' => -1,
                        _ => 0,
                    })
                    .sum();
                while depth > 0 && consumed + 1 < lines.len() && consumed - index < 50 {
                    consumed += 1;
                    let next = strip_line_comment(lines[consumed]);
                    depth += next
                        .chars()
                        .map(|c| match c {
                            '{' => 1,
                            '}' => -1,
                            _ => 0,
                        })
                        .sum::<i32>();
                    group.push(' ');
                    group.push_str(next);
                }
                skip_until = consumed + 1;
                apis.extend(expand_group(&prefix, group_inner(&group)));
            } else {
                apis.push(format!("{ident}::{segments}{matched}"));
            }
        }

        if !apis.is_empty() {
            apis.sort();
            apis.dedup();
            let mut site = CallSite::new(consumer, file, (index + 1) as u32);
            for api in apis {
                site = site.touching(&api);
            }
            sites.push(site);
        }
    }
    sites
}

/// Breaking changes for `dependency` from a JSON document.
///
/// Two shapes are accepted so one file can describe many dependencies:
///
/// - an array of changes, used as-is: `[{"api": "...", "kind": "removed_api", "detail": "..."}]`
/// - an object keyed by dependency name: `{"kube": [ ... ], "chrono": [ ... ]}`;
///   a dependency with no entry has no known breaking changes.
pub fn parse_breaking_changes(json: &str, dependency: &str) -> Result<Vec<BreakingChange>, String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("invalid breaking-changes JSON: {e}"))?;
    let entries = match value {
        serde_json::Value::Array(_) => value,
        serde_json::Value::Object(mut map) => match map.remove(dependency) {
            Some(entry) => entry,
            None => return Ok(Vec::new()),
        },
        _ => {
            return Err(
                "breaking-changes JSON must be an array or an object keyed by dependency"
                    .to_string(),
            )
        }
    };
    serde_json::from_value(entries)
        .map_err(|e| format!("invalid breaking changes for `{dependency}`: {e}"))
}

/// Every `.rs` file under `dir`, in name order, skipping [`SKIPPED_DIRS`].
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let skipped = entry
                .file_name()
                .to_str()
                .map(|name| SKIPPED_DIRS.contains(&name))
                .unwrap_or(false);
            if !skipped {
                rust_files(&path, out)?;
            }
        } else if file_type.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Walk `root` and collect the call sites of `crate_name` in every `.rs`
/// file, ordered by file then line so the generated suite is deterministic.
pub fn scan_tree(root: &Path, crate_name: &str) -> io::Result<Vec<CallSite>> {
    let mut files = Vec::new();
    rust_files(root, &mut files)?;

    let mut sites = Vec::new();
    for path in files {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        // Non-UTF-8 sources cannot contain a path we could match.
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let consumer = consumer_for(&relative);
        sites.extend(scan_source(&consumer, &relative, &source, crate_name));
    }
    Ok(sites)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apis(sites: &[CallSite]) -> Vec<String> {
        sites.iter().flat_map(|s| s.apis.clone()).collect()
    }

    #[test]
    fn crate_names_become_identifiers() {
        assert_eq!(crate_ident("serde-yaml"), "serde_yaml");
        assert_eq!(crate_ident("kube"), "kube");
    }

    #[test]
    fn consumers_follow_the_path() {
        assert_eq!(consumer_for("src/main.rs"), "stellar-operator");
        assert_eq!(consumer_for("src/bin/backup-verify.rs"), "backup-verify");
        assert_eq!(
            consumer_for("src/controller/reconcile.rs"),
            "stellar-operator::controller"
        );
        assert_eq!(
            consumer_for("src/telemetry.rs"),
            "stellar-operator::telemetry"
        );
        assert_eq!(consumer_for("tests/e2e/flow.rs"), "tests::flow");
        assert_eq!(
            consumer_for("benches/crd_operations.rs"),
            "benches::crd_operations"
        );
    }

    #[test]
    fn finds_a_plain_use() {
        let sites = scan_source("c", "src/a.rs", "use kube::runtime::Controller;\n", "kube");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].line, 1);
        assert_eq!(sites[0].location(), "src/a.rs:1");
        assert_eq!(sites[0].apis, vec!["kube::runtime::Controller"]);
    }

    #[test]
    fn expands_brace_groups_including_nested_ones() {
        let src = "use kube::{Api, Client, runtime::{watcher, Controller}};\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(sites.len(), 1);
        assert_eq!(
            sites[0].apis,
            vec![
                "kube::Api",
                "kube::Client",
                "kube::runtime::Controller",
                "kube::runtime::watcher",
            ]
        );
    }

    #[test]
    fn expands_a_brace_group_spread_over_several_lines() {
        let src = "use kube::{\n    Api,\n    Client,\n};\nfn main() {}\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(sites.len(), 1);
        assert_eq!(sites[0].line, 1);
        assert_eq!(sites[0].apis, vec!["kube::Api", "kube::Client"]);
    }

    #[test]
    fn two_groups_on_one_line_do_not_bleed_into_each_other() {
        let src = "use kube::{Api}; use other::{X};\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(sites[0].apis, vec!["kube::Api"]);
    }

    #[test]
    fn handles_self_and_renames() {
        let src = "use kube::runtime::{self, watcher as w};\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(
            sites[0].apis,
            vec!["kube::runtime", "kube::runtime::watcher"]
        );
    }

    #[test]
    fn finds_fully_qualified_uses_in_code() {
        let src = "fn f() { let c = kube::Client::try_default(); }\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(apis(&sites), vec!["kube::Client::try_default"]);
    }

    #[test]
    fn ignores_comments() {
        let src = "// use kube::Api;\n/* use kube::Client; */\n/*\n kube::Config\n*/\nlet x = 1; // kube::Resource\n";
        assert!(scan_source("c", "f.rs", src, "kube").is_empty());
    }

    #[test]
    fn keeps_code_that_surrounds_a_block_comment() {
        let src = "let c = /* note */ kube::Client::new();\n";
        let sites = scan_source("c", "f.rs", src, "kube");
        assert_eq!(apis(&sites), vec!["kube::Client::new"]);
    }

    #[test]
    fn does_not_match_a_longer_crate_name() {
        let src = "use kube_runtime::Controller;\nuse my_kube::Thing;\n";
        assert!(scan_source("c", "f.rs", src, "kube").is_empty());
    }

    #[test]
    fn hyphenated_crates_match_their_underscore_identifier() {
        let src = "let v: serde_yaml::Value = serde_yaml::from_str(s)?;\n";
        let sites = scan_source("c", "f.rs", src, "serde-yaml");
        assert_eq!(
            sites[0].apis,
            vec!["serde_yaml::Value", "serde_yaml::from_str"]
        );
    }

    #[test]
    fn results_feed_the_upgrade_validator() {
        use crate::dependency_contract::{
            BreakingChange, DependencyKind, UpgradeProposal, UpgradeValidator, Verdict,
        };

        let sites = scan_source(
            "stellar-operator::controller",
            "src/controller/reconcile.rs",
            "use kube::runtime::Controller;\n",
            "kube",
        );
        let mut builder = UpgradeProposal::builder("kube", DependencyKind::Library)
            .from("0.94.0")
            .to("1.0.0")
            .breaking_change(BreakingChange::removed_api("kube::runtime::Controller"));
        for site in sites {
            builder = builder.call_site(site);
        }
        let report = UpgradeValidator::new().validate(&builder.build());

        assert_eq!(report.verdict, Verdict::Blocked);
        assert_eq!(
            report.attribution[0].consumer,
            "stellar-operator::controller"
        );
        assert_eq!(
            report.attribution[0].call_sites,
            vec!["src/controller/reconcile.rs:1".to_string()]
        );
    }

    #[test]
    fn breaking_changes_parse_from_an_array() {
        let json = r#"[{"api":"kube::Api::all","kind":"removed_api","detail":"gone"}]"#;
        let changes = parse_breaking_changes(json, "kube").unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].api, "kube::Api::all");
        assert!(changes[0].is_fatal());
    }

    #[test]
    fn breaking_changes_parse_from_a_map_by_dependency() {
        let json = r#"{"kube":[{"api":"kube::Client","kind":"changed_signature","detail":"x"}],
                      "chrono":[]}"#;
        assert_eq!(parse_breaking_changes(json, "kube").unwrap().len(), 1);
        assert!(parse_breaking_changes(json, "chrono").unwrap().is_empty());
        assert!(parse_breaking_changes(json, "serde_yaml")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn breaking_changes_reject_bad_input() {
        assert!(parse_breaking_changes("not json", "kube").is_err());
        assert!(parse_breaking_changes("42", "kube").is_err());
        assert!(
            parse_breaking_changes(r#"[{"api":"x","kind":"bogus","detail":""}]"#, "kube").is_err()
        );
    }

    #[test]
    fn scan_tree_walks_rust_files_and_skips_target() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("src/controller")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("src/controller/a.rs"), "use kube::Client;\n").unwrap();
        std::fs::write(root.join("src/main.rs"), "use kube::Api;\n").unwrap();
        std::fs::write(root.join("target/debug/gen.rs"), "use kube::Config;\n").unwrap();
        std::fs::write(root.join("notes.txt"), "kube::Nope\n").unwrap();

        let sites = scan_tree(root, "kube").unwrap();
        let located: Vec<(String, String)> = sites
            .iter()
            .map(|s| (s.consumer.clone(), s.location()))
            .collect();
        assert_eq!(
            located,
            vec![
                (
                    "stellar-operator::controller".to_string(),
                    "src/controller/a.rs:1".to_string()
                ),
                ("stellar-operator".to_string(), "src/main.rs:1".to_string()),
            ]
        );
    }
}
