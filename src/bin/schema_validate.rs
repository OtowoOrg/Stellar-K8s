//! schema-validate — Repository-wide schema validation for YAML manifests.
//!
//! Loads the `openAPIV3Schema` embedded in every CRD under `config/crd/`
//! and validates every custom-resource manifest found under the given
//! search paths (default: `examples/`, `config/samples/`) against the
//! `spec` schema for its `kind`. Manifests whose `kind` is not one of our
//! own CRDs (core Kubernetes objects, third-party CRDs such as Kafka or
//! PrometheusRule, ...) are silently skipped — this tool only knows about
//! schemas we own.
//!
//! This complements `helm lint`'s `values.schema.json` check (which
//! validates chart *inputs*) and the `examples-smoke-test` CI job (which
//! `kubectl apply --dry-run=server`s a handful of files against a live
//! cluster): this tool needs no cluster and covers every example file, not
//! just the hand-picked few.
//!
//! # Quick start
//!
//! ```text
//! # Validate examples/ and config/samples/ (default) against config/crd/
//! cargo run --bin schema-validate
//!
//! # Validate specific paths
//! cargo run --bin schema-validate -- examples/validator-mainnet.yaml
//!
//! # List every CRD kind/version this tool knows how to validate
//! cargo run --bin schema-validate -- --list
//! ```
//!
//! See `docs/schema-validation.md` for the full user guide, including the
//! `.schema-validate-ignore` suppression mechanism and the JSON Schema
//! subset that is supported.

use std::{
    collections::HashMap,
    fmt, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use clap::Parser;
use regex::Regex;
use serde::Deserialize as _;
use serde_json::Value;
use walkdir::WalkDir;

// ── CLI ───────────────────────────────────────────────────────────────────────

/// Repository-wide schema validator for YAML manifests.
///
/// Validates custom-resource manifests against the OpenAPI v3 schemas
/// embedded in this repository's own CRDs (`config/crd/*.yaml`).
#[derive(Parser, Debug)]
#[command(
    name = "schema-validate",
    version,
    about = "Validate YAML manifests against this repo's CRD schemas"
)]
struct Cli {
    /// Directory containing CRD YAML files.
    #[arg(long, default_value = "config/crd")]
    crd_dir: PathBuf,

    /// File listing glob-style path prefixes to skip (one per line, `#` comments allowed).
    #[arg(long, default_value = ".schema-validate-ignore")]
    ignore_file: PathBuf,

    /// List every CRD kind/group/version this tool has a schema for, then exit.
    #[arg(long)]
    list: bool,

    /// Paths (files or directories) to scan. Defaults to `examples/` and `config/samples/`.
    paths: Vec<PathBuf>,
}

// ── CRD loading ──────────────────────────────────────────────────────────────

/// One CRD's schema(s), keyed by version name (e.g. `v1alpha1`).
struct CrdSchema {
    group: String,
    kind: String,
    versions: HashMap<String, Value>,
}

fn load_crds(dir: &Path) -> anyhow::Result<Vec<CrdSchema>> {
    let mut crds = Vec::new();

    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.extension()
                .is_some_and(|ext| ext == "yaml" || ext == "yml")
        })
        .collect();
    entries.sort();

    for path in entries {
        let content = fs::read_to_string(&path)?;
        for doc in yaml_documents(&content) {
            let Some(doc) = doc else { continue };
            if doc.get("kind").and_then(Value::as_str) != Some("CustomResourceDefinition") {
                continue;
            }
            let spec = &doc["spec"];
            let group = spec
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let kind = spec["names"]["kind"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if group.is_empty() || kind.is_empty() {
                continue;
            }

            let mut versions = HashMap::new();
            if let Some(vs) = spec.get("versions").and_then(Value::as_array) {
                for v in vs {
                    let Some(name) = v.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if let Some(schema) = v.get("schema").and_then(|s| s.get("openAPIV3Schema")) {
                        versions.insert(name.to_string(), schema.clone());
                    }
                }
            }

            crds.push(CrdSchema {
                group,
                kind,
                versions,
            });
        }
    }

    Ok(crds)
}

/// Parse every YAML document in `content`. Non-mapping documents (e.g. `---\n`
/// separators producing an empty doc) yield `None` and are skipped by the caller.
fn yaml_documents(content: &str) -> Vec<Option<Value>> {
    serde_yaml::Deserializer::from_str(content)
        .map(|doc| Value::deserialize(doc).ok().filter(|v| v.is_object()))
        .collect()
}

// ── Manifest discovery ──────────────────────────────────────────────────────

fn discover_yaml_files(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_file() {
            files.push(path.clone());
            continue;
        }
        for entry in WalkDir::new(path).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let p = entry.path();
                if p.extension()
                    .is_some_and(|ext| ext == "yaml" || ext == "yml")
                {
                    files.push(p.to_path_buf());
                }
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

/// Load `.schema-validate-ignore`: newline-separated path prefixes, `#` comments allowed.
fn load_ignore_list(path: &Path) -> Vec<String> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

fn is_ignored(rel_path: &str, ignore_list: &[String]) -> bool {
    ignore_list
        .iter()
        .any(|prefix| rel_path == prefix || rel_path.starts_with(prefix))
}

// ── Validation ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ValidationError {
    path: String,
    message: String,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.path, self.message)
    }
}

/// Validate `value` against a (subset of) JSON Schema / OpenAPI v3 `schema`.
///
/// Supported keywords: `type`, `enum`, `properties`, `required`,
/// `additionalProperties` (bool only), `items`, `minimum`, `maximum`,
/// `minLength`, `maxLength`, `pattern`, `nullable`,
/// `x-kubernetes-int-or-string`, `x-kubernetes-preserve-unknown-fields`.
/// CEL rules (`x-kubernetes-validations`) are intentionally not evaluated —
/// they require a CEL interpreter and are out of scope for a static check.
fn validate(schema: &Value, value: &Value, path: &str, errors: &mut Vec<ValidationError>) {
    let Some(schema_obj) = schema.as_object() else {
        return;
    };

    let nullable = schema_obj
        .get("nullable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if value.is_null() {
        if !nullable {
            errors.push(ValidationError {
                path: path.to_string(),
                message: "value is null but the field is not nullable".to_string(),
            });
        }
        return;
    }

    if let Some(enum_vals) = schema_obj.get("enum").and_then(Value::as_array) {
        if !enum_vals.contains(value) {
            errors.push(ValidationError {
                path: path.to_string(),
                message: format!(
                    "value {value} is not one of the allowed values {}",
                    Value::Array(enum_vals.clone())
                ),
            });
        }
    }

    let int_or_string = schema_obj
        .get("x-kubernetes-int-or-string")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if int_or_string {
        if !(value.is_string() || value.is_number()) {
            errors.push(ValidationError {
                path: path.to_string(),
                message: "expected an int-or-string value".to_string(),
            });
        }
        return;
    }

    let Some(ty) = schema_obj.get("type").and_then(Value::as_str) else {
        // No `type` keyword: fall through to structural checks below when
        // `properties` is present, otherwise nothing further to check.
        if schema_obj.contains_key("properties") {
            if !value.is_object() {
                errors.push(type_error(path, "object", value));
                return;
            }
            validate_object(schema_obj, value, path, errors);
        }
        return;
    };

    match ty {
        "object" => {
            if !value.is_object() {
                errors.push(type_error(path, "object", value));
                return;
            }
            validate_object(schema_obj, value, path, errors);
        }
        "array" => {
            let Some(items) = value.as_array() else {
                errors.push(type_error(path, "array", value));
                return;
            };
            if let Some(item_schema) = schema_obj.get("items") {
                for (i, item) in items.iter().enumerate() {
                    validate(item_schema, item, &format!("{path}[{i}]"), errors);
                }
            }
        }
        "string" => {
            let Some(s) = value.as_str() else {
                errors.push(type_error(path, "string", value));
                return;
            };
            if let Some(min) = schema_obj.get("minLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) < min {
                    errors.push(ValidationError {
                        path: path.to_string(),
                        message: format!("string is shorter than minLength {min}"),
                    });
                }
            }
            if let Some(max) = schema_obj.get("maxLength").and_then(Value::as_u64) {
                if (s.chars().count() as u64) > max {
                    errors.push(ValidationError {
                        path: path.to_string(),
                        message: format!("string is longer than maxLength {max}"),
                    });
                }
            }
            if let Some(pattern) = schema_obj.get("pattern").and_then(Value::as_str) {
                match Regex::new(pattern) {
                    Ok(re) if !re.is_match(s) => errors.push(ValidationError {
                        path: path.to_string(),
                        message: format!("value {s:?} does not match pattern {pattern:?}"),
                    }),
                    _ => {}
                }
            }
        }
        "integer" => {
            if value.as_i64().is_none() && value.as_u64().is_none() {
                errors.push(type_error(path, "integer", value));
                return;
            }
            validate_numeric_bounds(schema_obj, value, path, errors);
        }
        "number" => {
            if !value.is_number() {
                errors.push(type_error(path, "number", value));
                return;
            }
            validate_numeric_bounds(schema_obj, value, path, errors);
        }
        "boolean" => {
            if !value.is_boolean() {
                errors.push(type_error(path, "boolean", value));
            }
        }
        _ => {}
    }
}

fn validate_numeric_bounds(
    schema_obj: &serde_json::Map<String, Value>,
    value: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let n = value.as_f64().unwrap_or_default();
    if let Some(min) = schema_obj.get("minimum").and_then(Value::as_f64) {
        if n < min {
            errors.push(ValidationError {
                path: path.to_string(),
                message: format!("value {n} is less than minimum {min}"),
            });
        }
    }
    if let Some(max) = schema_obj.get("maximum").and_then(Value::as_f64) {
        if n > max {
            errors.push(ValidationError {
                path: path.to_string(),
                message: format!("value {n} is greater than maximum {max}"),
            });
        }
    }
}

fn validate_object(
    schema_obj: &serde_json::Map<String, Value>,
    value: &Value,
    path: &str,
    errors: &mut Vec<ValidationError>,
) {
    let preserve_unknown = schema_obj
        .get("x-kubernetes-preserve-unknown-fields")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let obj = value
        .as_object()
        .expect("caller ensured value is an object");
    let props = schema_obj.get("properties").and_then(Value::as_object);

    if let Some(required) = schema_obj.get("required").and_then(Value::as_array) {
        for req in required {
            if let Some(name) = req.as_str() {
                if !obj.contains_key(name) {
                    errors.push(ValidationError {
                        path: path.to_string(),
                        message: format!("missing required field {name:?}"),
                    });
                }
            }
        }
    }

    for (key, val) in obj {
        let child_path = format!("{path}.{key}");
        match props.and_then(|p| p.get(key)) {
            Some(child_schema) => validate(child_schema, val, &child_path, errors),
            None => {
                let additional_ok = schema_obj
                    .get("additionalProperties")
                    .map(|ap| ap.as_bool().unwrap_or(true))
                    .unwrap_or(true)
                    || preserve_unknown;
                if !additional_ok {
                    errors.push(ValidationError {
                        path: child_path,
                        message: "unknown field not permitted by the schema".to_string(),
                    });
                }
            }
        }
    }
}

fn type_error(path: &str, expected: &str, actual: &Value) -> ValidationError {
    ValidationError {
        path: path.to_string(),
        message: format!("expected type {expected}, found {}", describe_type(actual)),
    }
}

fn describe_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// ── main ─────────────────────────────────────────────────────────────────────

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

fn main() -> ExitCode {
    let cli = Cli::parse();
    let repo_root = find_repo_root().unwrap_or_else(|| PathBuf::from("."));

    let crds = match load_crds(&repo_root.join(&cli.crd_dir)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: failed to load CRDs from {}: {e}",
                cli.crd_dir.display()
            );
            return ExitCode::FAILURE;
        }
    };

    if cli.list {
        println!("{} CRD(s) known to schema-validate:\n", crds.len());
        for crd in &crds {
            let mut versions: Vec<&String> = crd.versions.keys().collect();
            versions.sort();
            println!(
                "  {}.{} — versions: {}",
                crd.kind,
                crd.group,
                versions
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return ExitCode::SUCCESS;
    }

    let search_paths: Vec<PathBuf> = if cli.paths.is_empty() {
        vec![repo_root.join("examples"), repo_root.join("config/samples")]
            .into_iter()
            .filter(|p| p.exists())
            .collect()
    } else {
        cli.paths.iter().map(|p| repo_root.join(p)).collect()
    };

    let ignore_list = load_ignore_list(&repo_root.join(&cli.ignore_file));
    let files = discover_yaml_files(&search_paths);

    let mut checked = 0usize;
    let mut skipped_ignored = 0usize;
    let mut skipped_unknown_kind = 0usize;
    let mut files_with_errors: Vec<(PathBuf, Vec<ValidationError>)> = Vec::new();

    for file in &files {
        let rel = file
            .strip_prefix(&repo_root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();
        if is_ignored(&rel, &ignore_list) {
            skipped_ignored += 1;
            continue;
        }

        let Ok(content) = fs::read_to_string(file) else {
            continue;
        };

        let mut file_errors = Vec::new();
        for (doc_idx, doc) in yaml_documents(&content).into_iter().enumerate() {
            let Some(doc) = doc else { continue };
            let Some(kind) = doc.get("kind").and_then(Value::as_str) else {
                continue;
            };
            let Some(api_version) = doc.get("apiVersion").and_then(Value::as_str) else {
                continue;
            };
            let Some((group, version)) = api_version.split_once('/') else {
                continue; // core v1 kinds (no group) are out of scope
            };

            let Some(crd) = crds.iter().find(|c| c.group == group && c.kind == kind) else {
                skipped_unknown_kind += 1;
                continue;
            };

            let Some(schema) = crd.versions.get(version) else {
                file_errors.push(ValidationError {
                    path: format!("doc[{doc_idx}]"),
                    message: format!(
                        "apiVersion {api_version} — version {version:?} is not defined by the {kind} CRD (known versions: {:?})",
                        crd.versions.keys().collect::<Vec<_>>()
                    ),
                });
                continue;
            };

            checked += 1;
            let Some(spec_schema) = schema.get("properties").and_then(|p| p.get("spec")) else {
                continue;
            };
            let spec_value = doc
                .get("spec")
                .cloned()
                .unwrap_or(Value::Object(Default::default()));
            validate(
                spec_schema,
                &spec_value,
                &format!("doc[{doc_idx}].spec"),
                &mut file_errors,
            );
        }

        if !file_errors.is_empty() {
            files_with_errors.push((file.clone(), file_errors));
        }
    }

    let total_errors: usize = files_with_errors.iter().map(|(_, e)| e.len()).sum();

    if !files_with_errors.is_empty() {
        eprintln!("\n🔴  Schema violations found:\n");
        for (file, errs) in &files_with_errors {
            let rel = file.strip_prefix(&repo_root).unwrap_or(file);
            eprintln!("  {}", rel.display());
            for err in errs {
                eprintln!("    {err}");
            }
        }
        eprintln!(
            "\n  Summary: {total_errors} violation(s) in {} file(s), {checked} document(s) checked, {skipped_unknown_kind} skipped (unknown kind), {skipped_ignored} skipped (ignored)",
            files_with_errors.len()
        );
        return ExitCode::FAILURE;
    }

    println!(
        "✅  {checked} document(s) validated against their CRD schema, 0 violations ({skipped_unknown_kind} skipped: unknown kind, {skipped_ignored} skipped: ignored)"
    );
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn errs(schema: Value, value: Value) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        validate(&schema, &value, "$", &mut errors);
        errors
    }

    #[test]
    fn accepts_matching_type() {
        let schema = json!({"type": "string"});
        assert!(errs(schema, json!("hello")).is_empty());
    }

    #[test]
    fn rejects_wrong_type() {
        let schema = json!({"type": "string"});
        let e = errs(schema, json!(42));
        assert_eq!(e.len(), 1);
        assert!(e[0].message.contains("expected type string"));
    }

    #[test]
    fn enforces_enum() {
        let schema = json!({"type": "string", "enum": ["mainnet", "testnet"]});
        assert!(errs(schema.clone(), json!("mainnet")).is_empty());
        assert_eq!(errs(schema, json!("bogus")).len(), 1);
    }

    #[test]
    fn enforces_required_fields() {
        let schema = json!({
            "type": "object",
            "required": ["nodeType"],
            "properties": {"nodeType": {"type": "string"}}
        });
        assert_eq!(errs(schema.clone(), json!({})).len(), 1);
        assert!(errs(schema, json!({"nodeType": "Validator"})).is_empty());
    }

    #[test]
    fn nullable_allows_null() {
        let schema = json!({"type": "string", "nullable": true});
        assert!(errs(schema, Value::Null).is_empty());
    }

    #[test]
    fn non_nullable_rejects_null() {
        let schema = json!({"type": "string"});
        assert_eq!(errs(schema, Value::Null).len(), 1);
    }

    #[test]
    fn validates_nested_objects_recursively() {
        let schema = json!({
            "type": "object",
            "properties": {
                "horizonConfig": {
                    "type": "object",
                    "required": ["databaseSecretRef"],
                    "properties": {"databaseSecretRef": {"type": "string"}}
                }
            }
        });
        let e = errs(schema, json!({"horizonConfig": {}}));
        assert_eq!(e.len(), 1);
        assert!(e[0].path.ends_with(".horizonConfig"));
    }

    #[test]
    fn validates_array_items() {
        let schema = json!({"type": "array", "items": {"type": "integer"}});
        assert!(errs(schema.clone(), json!([1, 2, 3])).is_empty());
        assert_eq!(errs(schema, json!([1, "two", 3])).len(), 1);
    }

    #[test]
    fn enforces_pattern() {
        let schema = json!({"type": "string", "pattern": "^[a-z]+$"});
        assert!(errs(schema.clone(), json!("abc")).is_empty());
        assert_eq!(errs(schema, json!("ABC")).len(), 1);
    }

    #[test]
    fn enforces_numeric_bounds() {
        let schema = json!({"type": "integer", "minimum": 1, "maximum": 10});
        assert!(errs(schema.clone(), json!(5)).is_empty());
        assert_eq!(errs(schema.clone(), json!(0)).len(), 1);
        assert_eq!(errs(schema, json!(11)).len(), 1);
    }

    #[test]
    fn int_or_string_accepts_both() {
        let schema = json!({"x-kubernetes-int-or-string": true});
        assert!(errs(schema.clone(), json!(5)).is_empty());
        assert!(errs(schema, json!("25%")).is_empty());
    }

    #[test]
    fn preserve_unknown_fields_allows_extra_keys() {
        let schema = json!({
            "type": "object",
            "x-kubernetes-preserve-unknown-fields": true,
            "properties": {}
        });
        assert!(errs(schema, json!({"anything": "goes"})).is_empty());
    }

    #[test]
    fn additional_properties_false_rejects_unknown_keys() {
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"known": {"type": "string"}}
        });
        assert!(errs(schema.clone(), json!({"known": "ok"})).is_empty());
        assert_eq!(errs(schema, json!({"unknown": "nope"})).len(), 1);
    }

    #[test]
    fn ignore_list_matches_prefix() {
        let list = vec![
            "examples/broken.yaml".to_string(),
            "config/samples/invalid-".to_string(),
        ];
        assert!(is_ignored("examples/broken.yaml", &list));
        assert!(is_ignored(
            "config/samples/invalid-network-empty.yaml",
            &list
        ));
        assert!(!is_ignored("examples/validator-mainnet.yaml", &list));
    }

    #[test]
    fn yaml_documents_splits_multi_doc_files() {
        let content = "kind: A\n---\nkind: B\n---\n";
        let docs = yaml_documents(content);
        assert_eq!(docs.len(), 3);
        assert!(docs[0].is_some());
        assert!(docs[1].is_some());
        assert!(docs[2].is_none());
    }
}
