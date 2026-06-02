//! OpenAPI/Swagger Documentation Generator
//!
//! Generates OpenAPI 3.0 specifications from gateway routes
//! and provides interactive API explorer UI.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// OpenAPI 3.0 Document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiDocument {
    #[serde(rename = "openapi")]
    pub openapi_version: String,
    pub info: OpenApiInfo,
    pub servers: Vec<OpenApiServer>,
    pub paths: HashMap<String, OpenApiPathItem>,
    pub components: OpenApiComponents,
    pub tags: Vec<OpenApiTag>,
    pub security: Vec<SecurityRequirement>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiInfo {
    pub title: String,
    pub description: String,
    pub version: String,
    pub contact: Option<OpenApiContact>,
    pub license: Option<OpenApiLicense>,
    pub terms_of_service: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiContact {
    pub name: String,
    pub url: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiLicense {
    pub name: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiServer {
    pub url: String,
    pub description: Option<String>,
    pub variables: Option<HashMap<String, OpenApiServerVariable>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiServerVariable {
    #[serde(rename = "enum")]
    pub enum_values: Option<Vec<String>>,
    pub default: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiPathItem {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub get: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub put: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<OpenApiOperation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<OpenApiOperation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiOperation {
    pub tags: Option<Vec<String>>,
    pub summary: Option<String>,
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
    pub parameters: Option<Vec<OpenApiParameter>>,
    pub request_body: Option<OpenApiRequestBody>,
    pub responses: HashMap<String, OpenApiResponse>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<SecurityRequirement>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_docs: Vec<OpenApiExternalDocs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiParameter {
    pub name: String,
    #[serde(rename = "in")]
    pub location: String,
    pub description: Option<String>,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<OpenApiSchema>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub example: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiRequestBody {
    pub description: Option<String>,
    pub content: HashMap<String, OpenApiMediaType>,
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiMediaType {
    pub schema: Option<OpenApiSchema>,
    pub example: Option<serde_json::Value>,
    pub examples: Option<HashMap<String, OpenApiExample>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiExample {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub value: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiResponse {
    pub description: String,
    pub headers: Option<HashMap<String, OpenApiHeader>>,
    pub content: Option<HashMap<String, OpenApiMediaType>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub links: Option<HashMap<String, OpenApiLink>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiHeader {
    pub description: Option<String>,
    pub required: bool,
    pub schema: Option<OpenApiSchema>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiLink {
    pub operation_ref: Option<String>,
    pub operation_id: Option<String>,
    pub parameters: Option<HashMap<String, serde_json::Value>>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiComponents {
    pub schemas: HashMap<String, OpenApiSchema>,
    pub security_schemes: HashMap<String, OpenApiSecurityScheme>,
    pub responses: Option<HashMap<String, OpenApiResponse>>,
    pub parameters: Option<HashMap<String, OpenApiParameter>>,
    pub examples: Option<HashMap<String, OpenApiExample>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OpenApiSchema {
    Object(OpenApiObjectSchema),
    Array(OpenApiArraySchema),
    String(OpenApiStringSchema),
    Integer(OpenApiIntegerSchema),
    Number(OpenApiNumberSchema),
    Boolean(OpenApiBooleanSchema),
    Ref { $ref: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiObjectSchema {
    pub title: Option<String>,
    pub description: Option<String>,
    pub required: Option<Vec<String>>,
    pub properties: Option<HashMap<String, OpenApiSchema>>,
    pub example: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiArraySchema {
    pub items: Box<OpenApiSchema>,
    pub min_items: Option<usize>,
    pub max_items: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiStringSchema {
    pub format: Option<String>,
    pub enum_values: Option<Vec<String>>,
    pub pattern: Option<String>,
    pub min_length: Option<usize>,
    pub max_length: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiIntegerSchema {
    pub format: Option<String>,
    pub minimum: Option<i64>,
    pub maximum: Option<i64>,
    pub enum_values: Option<Vec<i64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiNumberSchema {
    pub format: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    pub enum_values: Option<Vec<f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiBooleanSchema {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiTag {
    pub name: String,
    pub description: Option<String>,
    pub external_docs: Option<OpenApiExternalDocs>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiExternalDocs {
    pub url: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    #[serde(flatten)]
    pub schemes: HashMap<String, Vec<String>>,
}

/// Security scheme types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiSecurityScheme {
    #[serde(rename = "type")]
    pub scheme_type: String,
    pub description: Option<String>,
    pub name: Option<String>,
    #[serde(rename = "in")]
    pub location: Option<String>,
    pub scheme: Option<String>,
    pub bearer_format: Option<String>,
    pub flows: Option<OpenApiOAuthFlows>,
    pub open_id_connect_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiOAuthFlows {
    pub authorization_code: Option<OpenApiOAuthFlow>,
    pub implicit: Option<OpenApiOAuthFlow>,
    pub password: Option<OpenApiOAuthFlow>,
    pub client_credentials: Option<OpenApiOAuthFlow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenApiOAuthFlow {
    pub authorization_url: Option<String>,
    pub token_url: Option<String>,
    pub refresh_url: Option<String>,
    pub scopes: HashMap<String, String>,
}

/// OpenAPI Generator
pub struct OpenApiGenerator {
    title: String,
    version: String,
    description: String,
    servers: Vec<OpenApiServer>,
    routes: Vec<ApiRoute>,
}

#[derive(Debug, Clone)]
pub struct ApiRoute {
    pub path: String,
    pub method: String,
    pub summary: String,
    pub description: String,
    pub tags: Vec<String>,
    pub parameters: Vec<RouteParameter>,
    pub request_body: Option<RouteRequestBody>,
    pub responses: Vec<RouteResponse>,
    pub auth_required: bool,
}

#[derive(Debug, Clone)]
pub struct RouteParameter {
    pub name: String,
    pub location: String, // query, path, header
    pub required: bool,
    pub schema_type: String,
    pub description: String,
}

#[derive(Debug, Clone)]
pub struct RouteRequestBody {
    pub content_type: String,
    pub schema: String,
    pub required: bool,
}

#[derive(Debug, Clone)]
pub struct RouteResponse {
    pub status: u16,
    pub description: String,
    pub schema: Option<String>,
}

impl OpenApiGenerator {
    pub fn new(title: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            version: version.into(),
            description: String::new(),
            servers: vec![],
            routes: vec![],
        }
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    pub fn add_server(mut self, url: impl Into<String>, description: Option<String>) -> Self {
        self.servers.push(OpenApiServer {
            url: url.into(),
            description,
            variables: None,
        });
        self
    }

    pub fn add_route(mut self, route: ApiRoute) -> Self {
        self.routes.push(route);
        self
    }

    pub fn generate(self) -> OpenApiDocument {
        let mut paths: HashMap<String, OpenApiPathItem> = HashMap::new();
        let mut schemas: HashMap<String, OpenApiSchema> = HashMap::new();
        let mut security_schemes: HashMap<String, OpenApiSecurityScheme> = HashMap::new();

        // Add default security schemes
        security_schemes.insert("bearerAuth".to_string(), OpenApiSecurityScheme {
            scheme_type: "http".to_string(),
            description: Some("JWT Bearer token authentication".to_string()),
            name: Some("Authorization".to_string()),
            location: Some("header".to_string()),
            scheme: Some("bearer".to_string()),
            bearer_format: Some("JWT".to_string()),
            flows: None,
            open_id_connect_url: None,
        });

        security_schemes.insert("apiKeyAuth".to_string(), OpenApiSecurityScheme {
            scheme_type: "apiKey".to_string(),
            description: Some("API Key authentication".to_string()),
            name: Some("X-API-Key".to_string()),
            location: Some("header".to_string()),
            scheme: None,
            bearer_format: None,
            flows: None,
            open_id_connect_url: None,
        });

        // Convert routes to paths
        for route in &self.routes {
            let path_item = paths.entry(route.path.clone()).or_insert_with(|| OpenApiPathItem {
                summary: None,
                description: None,
                get: None,
                post: None,
                put: None,
                patch: None,
                delete: None,
                options: None,
                head: None,
                trace: None,
            });

            let operation = self.route_to_operation(route);
            
            match route.method.to_uppercase().as_str() {
                "GET" => path_item.get = Some(operation),
                "POST" => path_item.post = Some(operation),
                "PUT" => path_item.put = Some(operation),
                "PATCH" => path_item.patch = Some(operation),
                "DELETE" => path_item.delete = Some(operation),
                "OPTIONS" => path_item.options = Some(operation),
                "HEAD" => path_item.head = Some(operation),
                "TRACE" => path_item.trace = Some(operation),
                _ => {}
            }

            // Add schemas for request/response bodies
            if let Some(ref req_body) = route.request_body {
                if !schemas.contains_key(&req_body.schema) {
                    schemas.insert(req_body.schema.clone(), OpenApiSchema::Object(OpenApiObjectSchema {
                        title: Some(req_body.schema.clone()),
                        description: None,
                        required: None,
                        properties: None,
                        example: None,
                    }));
                }
            }

            for resp in &route.responses {
                if let Some(ref schema) = resp.schema {
                    if !schemas.contains_key(schema) {
                        schemas.insert(schema.clone(), OpenApiSchema::Object(OpenApiObjectSchema {
                            title: Some(schema.clone()),
                            description: None,
                            required: None,
                            properties: None,
                            example: None,
                        }));
                    }
                }
            }
        }

        // Create tags from route tags
        let mut tags_map: HashMap<String, String> = HashMap::new();
        for route in &self.routes {
            for tag in &route.tags {
                tags_map.entry(tag.clone()).or_insert_with(|| tag.clone());
            }
        }
        let tags: Vec<OpenApiTag> = tags_map.into_iter()
            .map(|(name, _)| OpenApiTag {
                name,
                description: None,
                external_docs: None,
            })
            .collect();

        OpenApiDocument {
            openapi_version: "3.0.3".to_string(),
            info: OpenApiInfo {
                title: self.title,
                description: self.description,
                version: self.version,
                contact: Some(OpenApiContact {
                    name: "Stellar K8s".to_string(),
                    url: Some("https://stellar.org".to_string()),
                    email: Some("[email]".to_string()),
                }),
                license: Some(OpenApiLicense {
                    name: "Apache-2.0".to_string(),
                    url: Some("https://www.apache.org/licenses/LICENSE-2.0".to_string()),
                }),
                terms_of_service: None,
            },
            servers: self.servers,
            paths,
            components: OpenApiComponents {
                schemas,
                security_schemes,
                responses: None,
                parameters: None,
                examples: None,
            },
            tags,
            security: vec![
                SecurityRequirement {
                    schemes: {
                        let mut m = HashMap::new();
                        m.insert("bearerAuth".to_string(), vec![]);
                        m
                    }
                },
                SecurityRequirement {
                    schemes: {
                        let mut m = HashMap::new();
                        m.insert("apiKeyAuth".to_string(), vec![]);
                        m
                    }
                },
            ],
        }
    }

    fn route_to_operation(&self, route: &ApiRoute) -> OpenApiOperation {
        let mut parameters: Option<Vec<OpenApiParameter>> = None;
        
        if !route.parameters.is_empty() {
            parameters = Some(route.parameters.iter().map(|p| OpenApiParameter {
                name: p.name.clone(),
                location: p.location.clone(),
                description: Some(p.description.clone()),
                required: p.required,
                schema_type: p.schema_type.clone(),
                schema: Some(OpenApiSchema::String(OpenApiStringSchema {
                    format: None,
                    enum_values: None,
                    pattern: None,
                    min_length: None,
                    max_length: None,
                })),
                example: None,
            }).collect());
        }

        let mut request_body = None;
        if let Some(ref req_body) = route.request_body {
            let mut content = HashMap::new();
            content.insert(req_body.content_type.clone(), OpenApiMediaType {
                schema: Some(OpenApiSchema::Ref { $ref: format!("#/components/schemas/{}", req_body.schema) }),
                example: None,
                examples: None,
            });
            
            request_body = Some(OpenApiRequestBody {
                description: None,
                content,
                required: req_body.required,
            });
        }

        let mut responses = HashMap::new();
        for resp in &route.responses {
            let mut resp_content = None;
            if let Some(ref schema) = resp.schema {
                let mut content = HashMap::new();
                content.insert("application/json".to_string(), OpenApiMediaType {
                    schema: Some(OpenApiSchema::Ref { $ref: format!("#/components/schemas/{}", schema) }),
                    example: None,
                    examples: None,
                });
                resp_content = Some(content);
            }

            responses.insert(resp.status.to_string(), OpenApiResponse {
                description: resp.description.clone(),
                headers: None,
                content: resp_content,
                links: None,
            });
        }

        // Default responses
        responses.insert("401".to_string(), OpenApiResponse {
            description: "Unauthorized".to_string(),
            headers: None,
            content: None,
            links: None,
        });
        responses.insert("429".to_string(), OpenApiResponse {
            description: "Too Many Requests".to_string(),
            headers: None,
            content: None,
            links: None,
        });
        responses.insert("500".to_string(), OpenApiResponse {
            description: "Internal Server Error".to_string(),
            headers: None,
            content: None,
            links: None,
        });

        let security = if route.auth_required {
            vec![
                SecurityRequirement {
                    schemes: {
                        let mut m = HashMap::new();
                        m.insert("bearerAuth".to_string(), vec![]);
                        m
                    }
                }
            ]
        } else {
            vec![]
        };

        OpenApiOperation {
            tags: Some(route.tags.clone()),
            summary: Some(route.summary.clone()),
            description: Some(route.description.clone()),
            operation_id: None,
            parameters,
            request_body,
            responses,
            security,
            deprecated: None,
            external_docs: vec![],
        }
    }
}

/// Default routes for the Stellar Operator API
pub fn get_default_routes() -> Vec<ApiRoute> {
    vec![
        ApiRoute {
            path: "/api/v1/nodes".to_string(),
            method: "GET".to_string(),
            summary: "List StellarNodes".to_string(),
            description: "Returns a list of all StellarNodes in the cluster".to_string(),
            tags: vec!["Nodes".to_string()],
            parameters: vec![
                RouteParameter {
                    name: "namespace".to_string(),
                    location: "query".to_string(),
                    required: false,
                    schema_type: "string".to_string(),
                    description: "Filter by namespace".to_string(),
                },
                RouteParameter {
                    name: "label".to_string(),
                    location: "query".to_string(),
                    required: false,
                    schema_type: "string".to_string(),
                    description: "Filter by label selector".to_string(),
                },
            ],
            request_body: None,
            responses: vec![
                RouteResponse {
                    status: 200,
                    description: "Successful response".to_string(),
                    schema: Some("NodeList".to_string()),
                },
            ],
            auth_required: true,
        },
        ApiRoute {
            path: "/api/v1/nodes/{namespace}/{name}".to_string(),
            method: "GET".to_string(),
            summary: "Get StellarNode".to_string(),
            description: "Returns a specific StellarNode by name and namespace".to_string(),
            tags: vec!["Nodes".to_string()],
            parameters: vec![
                RouteParameter {
                    name: "namespace".to_string(),
                    location: "path".to_string(),
                    required: true,
                    schema_type: "string".to_string(),
                    description: "Namespace of the StellarNode".to_string(),
                },
                RouteParameter {
                    name: "name".to_string(),
                    location: "path".to_string(),
                    required: true,
                    schema_type: "string".to_string(),
                    description: "Name of the StellarNode".to_string(),
                },
            ],
            request_body: None,
            responses: vec![
                RouteResponse {
                    status: 200,
                    description: "Successful response".to_string(),
                    schema: Some("StellarNode".to_string()),
                },
            ],
            auth_required: true,
        },
        ApiRoute {
            path: "/api/v1/dashboard/overview".to_string(),
            method: "GET".to_string(),
            summary: "Get Dashboard Overview".to_string(),
            description: "Returns dashboard overview statistics".to_string(),
            tags: vec!["Dashboard".to_string()],
            parameters: vec![],
            request_body: None,
            responses: vec![
                RouteResponse {
                    status: 200,
                    description: "Successful response".to_string(),
                    schema: Some("DashboardOverview".to_string()),
                },
            ],
            auth_required: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openapi_generator() {
        let doc = OpenApiGenerator::new("Test API", "1.0.0")
            .description("Test API Description")
            .add_server("https://api.example.com", Some("Production server".to_string()))
            .add_route(ApiRoute {
                path: "/test".to_string(),
                method: "GET".to_string(),
                summary: "Test endpoint".to_string(),
                description: "Test endpoint description".to_string(),
                tags: vec!["Test".to_string()],
                parameters: vec![],
                request_body: None,
                responses: vec![
                    RouteResponse {
                        status: 200,
                        description: "OK".to_string(),
                        schema: None,
                    }
                ],
                auth_required: false,
            })
            .generate();

        assert_eq!(doc.info.title, "Test API");
        assert_eq!(doc.info.version, "1.0.0");
        assert!(doc.paths.contains_key("/test"));
        assert!(doc.components.security_schemes.contains_key("bearerAuth"));
    }
}