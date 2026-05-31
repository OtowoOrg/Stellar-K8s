use schemars::gen::SchemaGenerator;
use schemars::schema::{InstanceType, Metadata, Schema, SchemaObject, SingleOrVec};
use serde_json::json;

/// Generate a Kubernetes-compatible structural schema for IntOrString
pub fn int_or_string_schema(_: &mut SchemaGenerator) -> Schema {
    SchemaObject {
        metadata: Some(Box::new(Metadata {
            description: Some("IntOrString".to_string()),
            ..Default::default()
        })),
        instance_type: None,
        format: None,
        enum_values: None,
        const_value: None,
        subschemas: Some(Box::new(schemars::schema::SubschemaValidation {
            any_of: Some(vec![
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Integer))),
                    ..Default::default()
                }),
                Schema::Object(SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::String))),
                    ..Default::default()
                }),
            ]),
            ..Default::default()
        })),
        extensions: [("x-kubernetes-int-or-string".to_string(), json!(true))].into(),
        ..Default::default()
    }
    .into()
}

/// Generate a Kubernetes-compatible structural schema for arbitrary objects
/// that preserves unknown fields (needed for k8s types like NodeAffinity)
pub fn object_schema(_: &mut SchemaGenerator) -> Schema {
    SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
        extensions: [(
            "x-kubernetes-preserve-unknown-fields".to_string(),
            json!(true),
        )]
        .into(),
        ..Default::default()
    }
    .into()
}

/// Generate a Kubernetes-compatible structural schema for arrays of objects
pub fn array_of_objects_schema(_: &mut SchemaGenerator) -> Schema {
    SchemaObject {
        instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Array))),
        array: Some(Box::new(schemars::schema::ArrayValidation {
            items: Some(SingleOrVec::Single(Box::new(Schema::Object(
                SchemaObject {
                    instance_type: Some(SingleOrVec::Single(Box::new(InstanceType::Object))),
                    extensions: [(
                        "x-kubernetes-preserve-unknown-fields".to_string(),
                        json!(true),
                    )]
                    .into(),
                    ..Default::default()
                },
            )))),
            ..Default::default()
        })),
        ..Default::default()
    }
    .into()
}
