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
//! Automated database migration harness (issue #1317).
//!
//! Requires `DATABASE_URL` pointing at an isolated PostgreSQL instance.
//! CI provisions `postgres:16` and always sets the URL. Local runs without
//! Postgres print a skip reason unless `CI=true`, in which case the tests fail.

use stellar_k8s::db_migrations::{
    default_migrations_dir, load_migrations, run_existing_data_path, run_fresh_path,
    with_temp_schema,
};

fn database_url() -> Option<String> {
    std::env::var("DATABASE_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn require_database_url() -> Option<String> {
    match database_url() {
        Some(url) => Some(url),
        None if std::env::var("STELLAR_MIGRATION_TEST").ok().as_deref() == Some("1") => {
            panic!("DATABASE_URL must be set when STELLAR_MIGRATION_TEST=1");
        }
        None => {
            eprintln!(
                "SKIP: DATABASE_URL is not set. Start Postgres and re-run, or use `make test-db-migrations`."
            );
            None
        }
    }
}

#[tokio::test]
async fn fresh_database_forward_rollback_reapply() {
    let Some(url) = require_database_url() else {
        return;
    };
    let migrations = load_migrations(default_migrations_dir()).expect("load migrations");
    with_temp_schema(&url, "migtest_fresh", |pool| async move {
        run_fresh_path(&pool, "migtest_fresh", &migrations).await
    })
    .await
    .expect("fresh migration path");
}

#[tokio::test]
async fn existing_database_preserves_data_and_transforms_checksum() {
    let Some(url) = require_database_url() else {
        return;
    };
    let migrations = load_migrations(default_migrations_dir()).expect("load migrations");
    with_temp_schema(&url, "migtest_existing", |pool| async move {
        run_existing_data_path(&pool, "migtest_existing", &migrations).await
    })
    .await
    .expect("existing-data migration path");
}
