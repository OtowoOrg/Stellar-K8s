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
//! check-unreachable-modules — Static reachability audit for Rust modules
//!
//! Walks `src/**/*.rs`, builds a module graph from crate roots (`lib.rs`,
//! `main.rs`, and every `[[bin]]` path declared in `Cargo.toml`), and reports:
//!
//! 1. **Orphan source files** — `.rs` files that are never reached via `mod`
//!    declarations from any crate root (unreachable modules).
//! 2. **Ambiguous module paths** — directories that contain both `foo.rs` and
//!    `foo/mod.rs`, which rustc rejects with E0761.
//! 3. **Dead code-path markers** — `todo!()`, `unimplemented!()`, and
//!    `unreachable!()` macros outside of `#[cfg(test)]` modules (informational
//!    when `--warn-only`, hard failures otherwise for `todo!`/`unimplemented!`
//!    when `--strict-dead-paths` is set).
//!
//! Module resolution matches rustc: `mod x;` in a crate root (lib.rs, main.rs,
//! or a declared bin such as `src/kubectl_plugin.rs`) resolves to a sibling of
//! the root file, while `mod x;` in any other `foo.rs` resolves under
//! `foo/x.rs` or `foo/x/mod.rs`.

//!
//! # Usage
//!
//! ```text
//! cargo run --bin check-unreachable-modules
//! cargo run --bin check-unreachable-modules -- --report
//! cargo run --bin check-unreachable-modules -- --warn-only
//! ```
//!
//! See `docs/unreachable-modules-check.md` for the full guide.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct Args {
    /// Print findings but always exit 0.
    warn_only: bool,
    /// Alias for warn_only (matches shell checker convention).
    report: bool,
    /// Also fail on todo!/unimplemented! outside tests.
    strict_dead_paths: bool,
    /// Repository root (defaults to cwd).
    root: PathBuf,
    /// Optional allowlist of known orphan paths (relative, forward slashes).
    allowlist: Option<PathBuf>,
}

fn parse_args() -> Args {
    let mut args = Args {
        root: env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        ..Args::default()
    };
    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--warn-only" => args.warn_only = true,
            "--report" => args.report = true,
            "--strict-dead-paths" => args.strict_dead_paths = true,
            "--root" => {
                if let Some(value) = iter.next() {
                    args.root = PathBuf::from(value);
                }
            }
            "--allowlist" => {
                if let Some(value) = iter.next() {
                    args.allowlist = Some(PathBuf::from(value));
                }
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                print_help();
                std::process::exit(2);
            }
        }
    }
    args
}

fn print_help() {
    eprintln!(
        "\
check-unreachable-modules — static reachability audit for Rust modules

USAGE:
    check-unreachable-modules [OPTIONS]

OPTIONS:
    --root <PATH>         Repository root (default: cwd)
    --allowlist <PATH>    Known-orphan allowlist (default: config/unreachable-modules-allowlist.txt)
    --warn-only           Exit 0 even when findings exist
    --report              Same as --warn-only (CI report mode)
    --strict-dead-paths   Fail on unfinished macros (todo / unimplemented) outside tests
    -h, --help            Show this help"
    );
}

// ── Findings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum FindingKind {
    OrphanFile,
    AmbiguousModule,
    DeadPathMarker,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    kind: FindingKind,
    path: PathBuf,
    detail: String,
    /// Allowlisted orphans are reported but do not fail CI.
    allowlisted: bool,
}

// ── Module graph ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ModDecl {
    name: String,
    /// Optional `#[path = "..."]` override relative to the declaring file's dir.
    path_attr: Option<PathBuf>,
}

/// Strip line comments and crude block comments so `mod` inside comments is ignored.
fn strip_comments(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let bytes = source.as_bytes();
    let mut i = 0;
    let mut in_block = false;
    while i < bytes.len() {
        if in_block {
            if bytes[i] == b'*' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
                in_block = false;
                out.push(' ');
                out.push(' ');
                i += 2;
            } else {
                out.push(if bytes[i] == b'\n' { '\n' } else { ' ' });
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            // line comment
            while i < bytes.len() && bytes[i] != b'\n' {
                out.push(' ');
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            in_block = true;
            out.push(' ');
            out.push(' ');
            i += 2;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Extract `mod` / `pub mod` declarations (file modules only, not inline bodies).
fn parse_mod_decls(source: &str) -> Vec<ModDecl> {
    let cleaned = strip_comments(source);
    let mut decls = Vec::new();
    let lines: Vec<&str> = cleaned.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // Look for optional #[path = "..."] on previous non-empty lines.
        let mut path_attr = None;
        if i > 0 {
            for back in (0..i).rev() {
                let prev = lines[back].trim();
                if prev.is_empty() {
                    continue;
                }
                if let Some(p) = extract_path_attr(prev) {
                    path_attr = Some(p);
                }
                break;
            }
        }

        if let Some(name) = extract_mod_name(line) {
            // Inline module: `mod foo {` — no separate file.
            if line.contains('{') && !line.contains(';') {
                i += 1;
                continue;
            }
            // `mod foo;` file module
            if line.contains(';') || looks_like_file_mod(line, lines.get(i + 1).copied()) {
                decls.push(ModDecl { name, path_attr });
            }
        }
        i += 1;
    }
    decls
}

fn extract_path_attr(line: &str) -> Option<PathBuf> {
    // #[path = "foo/bar.rs"] or #[path="foo/bar.rs"]
    let trimmed = line.trim();
    if !trimmed.starts_with("#[") || !trimmed.contains("path") {
        return None;
    }
    let start = trimmed.find('"')?;
    let rest = &trimmed[start + 1..];
    let end = rest.find('"')?;
    Some(PathBuf::from(&rest[..end]))
}

fn extract_mod_name(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let after = if let Some(rest) = trimmed.strip_prefix("pub(crate) mod ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("pub mod ") {
        rest
    } else if let Some(rest) = trimmed.strip_prefix("mod ") {
        rest
    } else {
        return None;
    };
    let name: String = after
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn looks_like_file_mod(line: &str, next: Option<&str>) -> bool {
    if line.ends_with(';') {
        return true;
    }
    // `mod foo` followed by `;` on next line
    next.map(|n| n.trim() == ";").unwrap_or(false)
}

/// Resolve a `mod name;` declaration relative to the declaring file.
///
/// `is_crate_root` must be true for crate roots (lib.rs, main.rs, and every
/// `[[bin]]` path declared in Cargo.toml). rustc resolves `mod x;` in a crate
/// root as a *sibling* of the root file (`src/kubectl_plugin.rs` +
/// `mod explain;` → `src/explain.rs`), not under a `kubectl_plugin/`
/// subdirectory. Only non-root files follow the `foo.rs → foo/` rule.
fn resolve_mod_file(declaring_file: &Path, decl: &ModDecl, is_crate_root: bool) -> Vec<PathBuf> {
    let parent = declaring_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();

    if let Some(ref path_attr) = decl.path_attr {
        return vec![parent.join(path_attr)];
    }

    let stem = declaring_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("");

    let search_dir = if is_crate_root || stem == "mod" || stem == "lib" || stem == "main" {
        parent
    } else {
        // foo.rs → foo/
        parent.join(stem)
    };

    vec![
        search_dir.join(format!("{}.rs", decl.name)),
        search_dir.join(&decl.name).join("mod.rs"),
    ]
}

fn collect_rs_files(src_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![src_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn parse_cargo_bin_paths(cargo_toml: &str) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut in_bin = false;
    for line in cargo_toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[[bin]]" {
            in_bin = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_bin = false;
            continue;
        }
        if in_bin {
            if let Some(rest) = trimmed.strip_prefix("path") {
                let rest = rest.trim().trim_start_matches('=').trim();
                if let Some(p) = rest.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                    paths.push(PathBuf::from(p));
                }
            }
        }
    }
    paths
}

fn crate_roots(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut roots = BTreeSet::new();
    let lib = repo_root.join("src/lib.rs");
    if lib.is_file() {
        roots.insert(lib);
    }
    let main = repo_root.join("src/main.rs");
    if main.is_file() {
        roots.insert(main);
    }
    let cargo = fs::read_to_string(repo_root.join("Cargo.toml"))
        .map_err(|e| format!("read Cargo.toml: {e}"))?;
    for rel in parse_cargo_bin_paths(&cargo) {
        let abs = repo_root.join(&rel);
        if abs.is_file() {
            roots.insert(abs);
        }
    }
    Ok(roots.into_iter().collect())
}

fn build_reachable(repo_root: &Path, roots: &[PathBuf]) -> Result<HashSet<PathBuf>, String> {
    let mut reachable = HashSet::new();
    let root_set: HashSet<PathBuf> = roots.iter().map(|r| normalize(repo_root, r)).collect();
    let mut queue: VecDeque<PathBuf> = roots.iter().cloned().collect();

    while let Some(file) = queue.pop_front() {
        let canon = normalize(repo_root, &file);
        if !reachable.insert(canon.clone()) {
            continue;
        }
        if !canon.is_file() {
            continue;
        }
        let is_crate_root = root_set.contains(&canon);
        let source =
            fs::read_to_string(&canon).map_err(|e| format!("read {}: {e}", canon.display()))?;
        for decl in parse_mod_decls(&source) {
            for candidate in resolve_mod_file(&canon, &decl, is_crate_root) {
                let candidate = normalize(repo_root, &candidate);
                if candidate.is_file() {
                    queue.push_back(candidate);
                }
            }
        }
    }
    Ok(reachable)
}

fn normalize(repo_root: &Path, path: &Path) -> PathBuf {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    // Best-effort canonicalize; fall back to cleaned components.
    fs::canonicalize(&abs).unwrap_or_else(|_| {
        let mut out = PathBuf::new();
        for c in abs.components() {
            match c {
                std::path::Component::ParentDir => {
                    out.pop();
                }
                std::path::Component::CurDir => {}
                other => out.push(other.as_os_str()),
            }
        }
        out
    })
}

fn find_ambiguous_modules(src_root: &Path) -> Result<Vec<Finding>, String> {
    let mut findings = Vec::new();
    let mut stack = vec![src_root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).map_err(|e| format!("read_dir {}: {e}", dir.display()))?;
        let mut names: HashMap<String, PathBuf> = HashMap::new();
        for entry in entries {
            let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path.clone());
                // Directory `foo/` with mod.rs conflicts with sibling `foo.rs`
                let name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let sibling_rs = dir.join(format!("{name}.rs"));
                let mod_rs = path.join("mod.rs");
                if sibling_rs.is_file() && mod_rs.is_file() {
                    findings.push(Finding {
                        kind: FindingKind::AmbiguousModule,
                        path: sibling_rs.clone(),
                        detail: format!(
                            "both `{}.rs` and `{}/mod.rs` exist (rustc E0761)",
                            name, name
                        ),
                        allowlisted: false,
                    });
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if stem != "mod" && stem != "lib" && stem != "main" {
                        names.insert(stem.to_string(), path);
                    }
                }
            }
        }
        let _ = names; // used via sibling checks above
    }
    Ok(findings)
}

fn scan_dead_path_markers(files: &[PathBuf], reachable: &HashSet<PathBuf>) -> Vec<Finding> {
    let mut findings = Vec::new();
    let markers = ["todo!", "unimplemented!", "unreachable!"];
    for file in files {
        if !reachable.contains(file) {
            continue;
        }
        let Ok(source) = fs::read_to_string(file) else {
            continue;
        };
        let cleaned = strip_comments(&source);
        let mut in_string = false;
        for (idx, line) in cleaned.lines().enumerate() {
            let started_in_string = in_string;
            // Update string-literal state for the next line (handles multi-line "\ ...").
            let mut escaped = false;
            for c in line.chars() {
                if escaped {
                    escaped = false;
                    continue;
                }
                if in_string {
                    if c == '\\' {
                        escaped = true;
                    } else if c == '"' {
                        in_string = false;
                    }
                } else if c == '"' {
                    in_string = true;
                }
            }

            if started_in_string {
                continue;
            }
            let trimmed = line.trim();
            if trimmed.starts_with("#[cfg(test)]") {
                continue;
            }
            for marker in markers {
                if let Some(pos) = line.find(marker) {
                    let before = &line[..pos];
                    if before.matches('"').count() % 2 == 1 {
                        continue;
                    }
                    findings.push(Finding {
                        kind: FindingKind::DeadPathMarker,
                        path: file.clone(),
                        detail: format!("line {}: contains `{marker}`", idx + 1),
                        allowlisted: false,
                    });
                }
            }
        }
    }
    findings
}

fn load_allowlist(path: &Path) -> Result<HashSet<String>, String> {
    if !path.is_file() {
        return Ok(HashSet::new());
    }
    let content =
        fs::read_to_string(path).map_err(|e| format!("read allowlist {}: {e}", path.display()))?;
    let mut set = HashSet::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        set.insert(trimmed.replace('\\', "/"));
    }
    Ok(set)
}

fn relative_display(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

fn default_allowlist_path(repo_root: &Path) -> PathBuf {
    repo_root.join("config/unreachable-modules-allowlist.txt")
}

fn run(args: &Args) -> Result<Vec<Finding>, String> {
    let repo_root = normalize(&PathBuf::from("."), &args.root);
    let src_root = repo_root.join("src");
    if !src_root.is_dir() {
        return Err(format!("src/ not found under {}", repo_root.display()));
    }

    let roots = crate_roots(&repo_root)?;
    if roots.is_empty() {
        return Err("no crate roots found".into());
    }

    let allowlist_path = args
        .allowlist
        .clone()
        .unwrap_or_else(|| default_allowlist_path(&repo_root));
    let allowlist = load_allowlist(&allowlist_path)?;

    let reachable = build_reachable(&repo_root, &roots)?;
    let all_files: Vec<PathBuf> = collect_rs_files(&src_root)?
        .into_iter()
        .map(|p| normalize(&repo_root, &p))
        .collect();

    let mut findings = Vec::new();

    // Orphan files: present on disk but not reachable from any root.
    let root_set: HashSet<PathBuf> = roots.iter().map(|r| normalize(&repo_root, r)).collect();
    for file in &all_files {
        if root_set.contains(file) || reachable.contains(file) {
            continue;
        }
        let rel = relative_display(&repo_root, file);
        let allowlisted = allowlist.contains(&rel);
        findings.push(Finding {
            kind: FindingKind::OrphanFile,
            path: file.clone(),
            detail: if allowlisted {
                "allowlisted orphan (known WIP / not yet wired into a crate root)".into()
            } else {
                "source file is not reachable via any `mod` declaration from a crate root".into()
            },
            allowlisted,
        });
    }

    for mut finding in find_ambiguous_modules(&src_root)? {
        finding.allowlisted = false;
        findings.push(finding);
    }

    findings.extend(scan_dead_path_markers(&all_files, &reachable));

    findings.sort();
    findings.dedup();
    Ok(findings)
}

fn print_report(repo_root: &Path, findings: &[Finding], roots: &[PathBuf]) {
    println!("check-unreachable-modules — static module reachability audit");
    println!("crate roots ({}):", roots.len());
    for r in roots {
        println!("  - {}", relative_display(repo_root, r));
    }
    println!();

    if findings.is_empty() {
        println!("✓ No unreachable modules or ambiguous module paths found.");
        return;
    }

    let mut by_kind: BTreeMap<FindingKind, Vec<&Finding>> = BTreeMap::new();
    for f in findings {
        by_kind.entry(f.kind.clone()).or_default().push(f);
    }

    for (kind, items) in by_kind {
        let title = match kind {
            FindingKind::OrphanFile => "Unreachable / orphan source files",
            FindingKind::AmbiguousModule => "Ambiguous module paths (E0761)",
            FindingKind::DeadPathMarker => "Dead code-path markers",
        };
        println!("{title} ({}):", items.len());
        for item in items {
            let marker = if item.allowlisted { "⚠" } else { "✗" };
            println!(
                "  {marker} {} — {}",
                relative_display(repo_root, &item.path),
                item.detail
            );
        }
        println!();
    }
}

fn main() -> ExitCode {
    let args = parse_args();
    let repo_root = normalize(&PathBuf::from("."), &args.root);

    let findings = match run(&args) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };

    let roots = crate_roots(&repo_root).unwrap_or_default();
    print_report(&repo_root, &findings, &roots);

    let hard: Vec<_> = findings
        .iter()
        .filter(|f| {
            if f.allowlisted {
                return false;
            }
            match f.kind {
                FindingKind::OrphanFile | FindingKind::AmbiguousModule => true,
                FindingKind::DeadPathMarker => {
                    args.strict_dead_paths
                        && (f.detail.contains("todo!") || f.detail.contains("unimplemented!"))
                }
            }
        })
        .collect();

    if hard.is_empty() || args.warn_only || args.report {
        let allowlisted = findings.iter().filter(|f| f.allowlisted).count();
        if allowlisted > 0 {
            println!(
                "note: {allowlisted} allowlisted orphan(s) acknowledged in config/unreachable-modules-allowlist.txt"
            );
        }
        if !findings.is_empty() && (args.warn_only || args.report) {
            println!(
                "report mode: {} finding(s) noted, exiting 0",
                findings.len()
            );
        } else if hard.is_empty() {
            println!("✓ No hard findings (new orphans or ambiguous module paths).");
        }
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "\n{} hard finding(s) — wire modules, update the allowlist, or pass --warn-only",
            hard.len()
        );
        ExitCode::from(1)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_mod_decls() {
        let src = r#"
            pub mod foo;
            mod bar;
            // mod commented;
            pub(crate) mod baz;
            mod inline { fn x() {} }
        "#;
        let decls = parse_mod_decls(src);
        let names: Vec<_> = decls.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn parse_path_attr() {
        let src = r#"
            #[path = "custom/thing.rs"]
            mod thing;
        "#;
        let decls = parse_mod_decls(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "thing");
        assert_eq!(
            decls[0].path_attr.as_ref().unwrap(),
            &PathBuf::from("custom/thing.rs")
        );
    }

    #[test]
    fn strip_comments_removes_mod_in_comment() {
        let src = "// mod hidden;\nmod visible;\n";
        let decls = parse_mod_decls(src);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].name, "visible");
    }

    #[test]
    fn resolve_from_lib_rs() {
        let lib = PathBuf::from("/repo/src/lib.rs");
        let decl = ModDecl {
            name: "controller".into(),
            path_attr: None,
        };
        let candidates = resolve_mod_file(&lib, &decl, true);
        assert!(candidates
            .iter()
            .any(|p| p.ends_with("src/controller.rs") || p.ends_with("src\\controller.rs")));
        assert!(candidates
            .iter()
            .any(|p| p.ends_with("controller/mod.rs") || p.ends_with("controller\\mod.rs")));
    }

    #[test]
    fn resolve_from_nested_rs_file() {
        let file = PathBuf::from("/repo/src/controller.rs");
        let decl = ModDecl {
            name: "health".into(),
            path_attr: None,
        };
        let candidates = resolve_mod_file(&file, &decl, false);
        assert!(candidates
            .iter()
            .any(|p| p.ends_with("controller/health.rs") || p.ends_with("controller\\health.rs")));
    }

    #[test]
    fn resolve_from_bin_crate_root_is_sibling() {
        // rustc treats a `[[bin]]` path (e.g. src/kubectl_plugin.rs) as a crate
        // root, so `mod explain;` resolves to the SIBLING src/explain.rs — not
        // src/kubectl_plugin/explain.rs. Regression test for the cleanup that
        // removed the false-positive orphans src/{explain,audit_report,sql}.rs.
        let file = PathBuf::from("/repo/src/kubectl_plugin.rs");
        let decl = ModDecl {
            name: "explain".into(),
            path_attr: None,
        };
        let candidates = resolve_mod_file(&file, &decl, true);
        assert!(
            candidates
                .iter()
                .any(|p| p.ends_with("src/explain.rs") || p.ends_with("src\\explain.rs")),
            "expected sibling resolution, got {candidates:?}"
        );
        assert!(!candidates.iter().any(|p| {
            p.ends_with("kubectl_plugin/explain.rs") || p.ends_with("kubectl_plugin\\explain.rs")
        }));
    }

    #[test]
    fn parse_cargo_bins() {
        let toml = r#"
[[bin]]
name = "doc-check"
path = "src/bin/doc_check.rs"

[dependencies]
foo = "1"

[[bin]]
name = "other"
path = "src/other.rs"
"#;
        let paths = parse_cargo_bin_paths(toml);
        assert_eq!(
            paths,
            vec![
                PathBuf::from("src/bin/doc_check.rs"),
                PathBuf::from("src/other.rs")
            ]
        );
    }

    #[test]
    fn extract_mod_name_variants() {
        assert_eq!(extract_mod_name("mod foo;").as_deref(), Some("foo"));
        assert_eq!(extract_mod_name("pub mod bar;").as_deref(), Some("bar"));
        assert_eq!(
            extract_mod_name("pub(crate) mod baz;").as_deref(),
            Some("baz")
        );
        assert_eq!(extract_mod_name("use mod_thing;"), None);
    }

    #[test]
    fn load_allowlist_ignores_comments_and_blanks() {
        let dir =
            std::env::temp_dir().join(format!("unreachable-allowlist-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("allowlist.txt");
        fs::write(
            &path,
            "# comment\n\nsrc/foo.rs\nsrc\\bar.rs\n  src/baz.rs  \n",
        )
        .unwrap();
        let set = load_allowlist(&path).unwrap();
        assert!(set.contains("src/foo.rs"));
        assert!(set.contains("src/bar.rs"));
        assert!(set.contains("src/baz.rs"));
        assert_eq!(set.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_allowlist_is_empty() {
        let set = load_allowlist(Path::new("/nonexistent/allowlist.txt")).unwrap();
        assert!(set.is_empty());
    }
}
