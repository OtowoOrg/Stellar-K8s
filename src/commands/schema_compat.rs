//! Offline schema-registry compatibility gate for pull requests.

use crate::schema_registry::{RegistryOverride, SchemaRegistry};
use crate::Error;
use clap::Parser;
use std::fs;

#[derive(Parser, Debug, Clone)]
#[command(about = "Check a proposed schema against every pinned consumer")]
pub struct SchemaCompatArgs {
    /// Versioned registry JSON committed to the repository.
    #[arg(long, default_value = "schemas/registry.json")]
    pub registry: String,
    /// Schema subject to change.
    #[arg(long)]
    pub subject: String,
    /// Candidate JSON Schema file.
    #[arg(long)]
    pub schema: String,
    /// Optional explicit override id (must already be present in the registry).
    #[arg(long)]
    pub override_id: Option<String>,
    /// Write the machine-readable consumer impact report to this file.
    #[arg(long)]
    pub report: Option<String>,
}

pub fn run_schema_compat(args: SchemaCompatArgs) -> Result<(), Error> {
    let registry_text = fs::read_to_string(&args.registry)
        .map_err(|e| Error::config_step("read schema registry", e))?;
    let registry = SchemaRegistry::from_json(&registry_text)
        .map_err(|e| Error::config_step("parse schema registry", e))?;
    let definition: serde_json::Value = fs::read_to_string(&args.schema)
        .map_err(|e| Error::config_step("read proposed schema", e))?
        .parse()
        .map_err(|e| Error::config_step("parse proposed schema", e))?;
    let impact = registry
        .try_impact(&args.subject, &definition)
        .map_err(|e| Error::validation_step("schema compatibility", e))?;
    let report_json = serde_json::to_string_pretty(&impact).map_err(Error::SerializationError)?;
    if let Some(path) = &args.report {
        fs::write(path, format!("{report_json}\n"))
            .map_err(|e| Error::config_step("write impact report", e))?;
    }
    if impact.breaking {
        let authorized = args.override_id.as_deref().is_some_and(|id| {
            registry
                .snapshot()
                .overrides
                .iter()
                .any(|o: &RegistryOverride| o.id == id && o.is_valid(&args.subject) && !o.consumed)
        });
        if !authorized {
            eprintln!("{report_json}");
            return Err(Error::validation_step(
                "schema compatibility",
                "breaking change rejected; an explicit unconsumed registry override is required",
            ));
        }
    }
    println!("{report_json}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn args_parse() {
        let a = SchemaCompatArgs::try_parse_from([
            "check",
            "--subject",
            "events",
            "--schema",
            "new.json",
        ])
        .unwrap();
        assert_eq!(a.registry, "schemas/registry.json");
    }

    fn fixture(dir: &std::path::Path) -> (String, String) {
        let registry = dir.join("registry.json");
        fs::write(&registry, include_str!("../../schemas/registry.json")).unwrap();
        let schema = dir.join("candidate.json");
        fs::write(
            &schema,
            r#"{"type":"object","required":["id"],"properties":{"id":{"type":"string"}}}"#,
        )
        .unwrap();
        (
            registry.to_string_lossy().into_owned(),
            schema.to_string_lossy().into_owned(),
        )
    }

    #[test]
    fn compatible_candidate_passes_and_reports_every_consumer() {
        let dir = tempfile::tempdir().unwrap();
        let (registry, schema) = fixture(dir.path());
        run_schema_compat(SchemaCompatArgs {
            registry,
            subject: "stellar.ledger.events".into(),
            schema,
            override_id: None,
            report: None,
        })
        .unwrap();
    }

    #[test]
    fn breaking_candidate_without_override_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let (registry, schema) = fixture(dir.path());
        fs::write(
            &schema,
            r#"{"type":"object","required":[],"properties":{}}"#,
        )
        .unwrap();
        let err = run_schema_compat(SchemaCompatArgs {
            registry,
            subject: "stellar.ledger.events".into(),
            schema,
            override_id: None,
            report: None,
        })
        .unwrap_err();
        assert!(err.to_string().contains("override"));
    }
}
