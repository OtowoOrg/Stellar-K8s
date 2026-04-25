use comfy_table::Table;
use csv::WriterBuilder;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::{Api, Client, ResourceExt};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use std::time::Duration;

use stellar_k8s::crd::StellarNode;
use stellar_k8s::error::{Error, Result};

pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

pub struct SqlExecutor {
    client: Client,
    namespace: String,
}

impl SqlExecutor {
    pub fn new(client: Client, namespace: String) -> Self {
        Self { client, namespace }
    }

    pub async fn execute(&self, node_name: &str, query: &str, format: OutputFormat) -> Result<()> {
        let node_api: Api<StellarNode> = Api::namespaced(self.client.clone(), &self.namespace);
        let node = node_api.get(node_name).await.map_err(Error::KubeError)?;

        let db_uri = self.get_db_uri(&node).await?;

        // Setup port-forwarding
        let (local_port, _pf_handle) = self.setup_port_forward(&node).await?;

        // Adjust URI to use localhost and the forwarded port
        let local_uri = self.patch_uri_for_localhost(&db_uri, local_port)?;

        // Connect and execute
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .acquire_timeout(Duration::from_secs(10))
            .connect(&local_uri)
            .await
            .map_err(Error::SqlxError)?;

        // Enforce read-only at the session level
        sqlx::query("SET SESSION CHARACTERISTICS AS TRANSACTION READ ONLY")
            .execute(&pool)
            .await
            .map_err(Error::SqlxError)?;

        let rows = sqlx::query(query)
            .fetch_all(&pool)
            .await
            .map_err(Error::SqlxError)?;

        if rows.is_empty() {
            println!("Query returned no rows.");
            return Ok(());
        }

        match format {
            OutputFormat::Table => self.print_table(&rows),
            OutputFormat::Json => self.print_json(&rows),
            OutputFormat::Csv => self.print_csv(&rows),
        }

        Ok(())
    }

    async fn get_db_uri(&self, node: &StellarNode) -> Result<String> {
        let secret_api: Api<Secret> = Api::namespaced(self.client.clone(), &self.namespace);

        if let Some(_managed) = &node.spec.managed_database {
            let secret_name = format!("{}-app", node.name_any());
            let secret = secret_api
                .get(&secret_name)
                .await
                .map_err(Error::KubeError)?;

            let data = secret
                .data
                .as_ref()
                .ok_or_else(|| Error::ConfigError(format!("Secret {} has no data", secret_name)))?;

            let uri_bytes = data.get("uri").ok_or_else(|| {
                Error::ConfigError(format!("Secret {} missing 'uri' key", secret_name))
            })?;

            String::from_utf8(uri_bytes.0.clone())
                .map_err(|e| Error::ConfigError(format!("Invalid UTF-8 in URI: {}", e)))
        } else if let Some(ext) = &node.spec.database {
            let secret_name = &ext.secret_key_ref.name;
            let key = &ext.secret_key_ref.key;

            let secret = secret_api
                .get(secret_name)
                .await
                .map_err(Error::KubeError)?;
            let data = secret
                .data
                .as_ref()
                .ok_or_else(|| Error::ConfigError(format!("Secret {} has no data", secret_name)))?;

            let uri_bytes = data.get(key).ok_or_else(|| {
                Error::ConfigError(format!("Secret {} missing key '{}'", secret_name, key))
            })?;

            String::from_utf8(uri_bytes.0.clone())
                .map_err(|e| Error::ConfigError(format!("Invalid UTF-8 in URI: {}", e)))
        } else {
            Err(Error::ConfigError(
                "No database configured for node".to_string(),
            ))
        }
    }

    async fn setup_port_forward(
        &self,
        node: &StellarNode,
    ) -> Result<(u16, tokio::task::JoinHandle<()>)> {
        let pod_api: Api<Pod> = Api::namespaced(self.client.clone(), &self.namespace);

        // Find a suitable pod for port-forwarding
        let pod_name: String = if node.spec.managed_database.is_some() {
            // For CNPG, try to find a replica first
            let lp = kube::api::ListParams::default().labels(&format!(
                "cnpg.io/cluster={},cnpg.io/role=replica",
                node.name_any()
            ));
            let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;

            if let Some(pod) = pods.items.first() {
                pod.name_any()
            } else {
                // Fallback to any pod in the cluster
                let lp = kube::api::ListParams::default()
                    .labels(&format!("cnpg.io/cluster={}", node.name_any()));
                let pods = pod_api.list(&lp).await.map_err(Error::KubeError)?;
                pods.items
                    .first()
                    .ok_or_else(|| Error::ConfigError("No database pods found".to_string()))?
                    .name_any()
            }
        } else {
            // For external DB, we don't know where to port-forward unless it's in the cluster.
            return Err(Error::ConfigError(
                "SQL command currently only supports managed_database nodes".to_string(),
            ));
        };

        // Manual TCP proxy to the portforward stream
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        listener.set_nonblocking(true)?;
        let tokio_listener = tokio::net::TcpListener::from_std(listener)?;

        let pod_api_clone = pod_api.clone();
        let pod_name_clone = pod_name.clone();

        let pf_handle = tokio::spawn(async move {
            while let Ok((mut client_stream, _)) = tokio_listener.accept().await {
                if let Ok(mut pf) = pod_api_clone.portforward(&pod_name_clone, &[5432]).await {
                    if let Some(mut server_stream) = pf.take_stream(5432) {
                        tokio::spawn(async move {
                            let _ = tokio::io::copy_bidirectional(
                                &mut client_stream,
                                &mut server_stream,
                            )
                            .await;
                        });
                    }
                }
            }
        });

        // Wait a bit for the tunnel to establish
        tokio::time::sleep(Duration::from_millis(200)).await;

        Ok((port, pf_handle))
    }

    fn patch_uri_for_localhost(&self, uri: &str, port: u16) -> Result<String> {
        // Simple URI patching: replace host:port with 127.0.0.1:port
        // postgresql://user:pass@host:5432/db
        let parts: Vec<&str> = uri.split('@').collect();
        if parts.len() != 2 {
            return Err(Error::ConfigError("Invalid DB URI format".to_string()));
        }

        let host_part = parts[1];
        let slash_index = host_part.find('/').ok_or_else(|| {
            Error::ConfigError("Invalid DB URI format (missing / after host)".to_string())
        })?;

        let _host_and_port = &host_part[..slash_index];
        let db_name = &host_part[slash_index..];

        Ok(format!("{}@127.0.0.1:{}{}", parts[0], port, db_name))
    }

    fn print_table(&self, rows: &[sqlx::postgres::PgRow]) {
        use sqlx::{Column, Row};
        let mut table = Table::new();

        if rows.is_empty() {
            return;
        }

        let columns = rows[0].columns();
        table.set_header(columns.iter().map(|c| c.name()));

        for row in rows {
            let mut table_row = Vec::new();
            for i in 0..columns.len() {
                // Try to get as string, fallback to debug formatting
                let val: String = row
                    .try_get_unchecked::<'_, String, _>(i)
                    .unwrap_or_else(|_| "binary data".to_string());
                table_row.push(val);
            }
            table.add_row(table_row);
        }

        println!("{}", table);
    }

    fn print_json(&self, rows: &[sqlx::postgres::PgRow]) {
        use sqlx::{Column, Row};
        let mut results = Vec::new();

        for row in rows {
            let mut obj = serde_json::Map::new();
            for col in row.columns() {
                let val: serde_json::Value = row
                    .try_get_unchecked::<'_, String, _>(col.ordinal())
                    .map(|s| json!(s))
                    .unwrap_or_else(|_| json!("binary data"));
                obj.insert(col.name().to_string(), val);
            }
            results.push(serde_json::Value::Object(obj));
        }

        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    }

    fn print_csv(&self, rows: &[sqlx::postgres::PgRow]) {
        use sqlx::{Column, Row};
        let mut wtr = WriterBuilder::new().from_writer(std::io::stdout());

        if rows.is_empty() {
            return;
        }

        let columns = rows[0].columns();
        let _ = wtr.write_record(columns.iter().map(|c| c.name()));

        for row in rows {
            let mut record = Vec::new();
            for i in 0..columns.len() {
                let val: String = row
                    .try_get_unchecked::<'_, String, _>(i)
                    .unwrap_or_else(|_| "binary data".to_string());
                record.push(val);
            }
            let _ = wtr.write_record(record);
        }
        let _ = wtr.flush();
    }
}
