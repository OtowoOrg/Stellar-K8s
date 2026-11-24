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
//! Webhook certificate coverage gate (#1980): every admission webhook
//! configuration the chart ships must get its CA bundle from cert-manager's
//! cainjector (automated renewal + trust-bundle distribution), must not
//! declare `failurePolicy` twice, and the serving cert must be paired with the
//! cert-health sidecar.

use std::fs;
use std::path::Path;

fn chart_templates() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("charts/stellar-operator/templates");
    let mut out = Vec::new();
    let mut stack = vec![dir];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).expect("read templates") {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "yaml") {
                let text = fs::read_to_string(&path).unwrap();
                out.push((path.display().to_string(), text));
            }
        }
    }
    out
}

/// Split a template into YAML documents on `---` separator lines.
fn documents(text: &str) -> Vec<String> {
    let mut docs = vec![String::new()];
    for line in text.lines() {
        if line.trim() == "---" {
            docs.push(String::new());
        } else {
            docs.last_mut().unwrap().push_str(line);
            docs.last_mut().unwrap().push('\n');
        }
    }
    docs
}

fn webhook_config_docs() -> Vec<(String, String)> {
    let mut found = Vec::new();
    for (path, text) in chart_templates() {
        for doc in documents(&text) {
            let is_cfg = doc.lines().any(|l| {
                let l = l.trim();
                l == "kind: ValidatingWebhookConfiguration"
                    || l == "kind: MutatingWebhookConfiguration"
            });
            if is_cfg {
                found.push((path.clone(), doc));
            }
        }
    }
    found
}

#[test]
fn chart_ships_at_least_one_webhook_configuration() {
    assert!(!webhook_config_docs().is_empty());
}

#[test]
fn every_webhook_configuration_is_covered_by_automated_renewal() {
    for (path, doc) in webhook_config_docs() {
        assert!(
            doc.contains("cert-manager.io/inject-ca-from:"),
            "{path}: webhook configuration lacks cert-manager inject-ca-from, so its CA bundle would not renew"
        );
    }
}

#[test]
fn webhook_configurations_do_not_duplicate_failure_policy() {
    for (path, doc) in webhook_config_docs() {
        // Count per `webhooks:` entry (entries start with "  - name:").
        for entry in doc.split("\n  - name:").skip(1) {
            let n = entry
                .lines()
                .filter(|l| l.trim_start().starts_with("failurePolicy:"))
                .count();
            assert_eq!(
                n, 1,
                "{path}: each webhook must set failurePolicy exactly once, found {n}"
            );
        }
    }
}

#[test]
fn serving_certificate_is_paired_with_the_cert_health_sidecar() {
    let webhook = chart_templates()
        .into_iter()
        .find(|(p, _)| p.ends_with("templates/webhook.yaml"))
        .expect("webhook.yaml")
        .1;
    assert!(
        webhook.contains("name: cert-health"),
        "cert-health sidecar missing"
    );
    assert!(webhook.contains("/stellar-cert-health"));
    // Readiness is gated on the sidecar so a bad cert never receives traffic.
    assert!(webhook.contains("port: cert-health"));
}
