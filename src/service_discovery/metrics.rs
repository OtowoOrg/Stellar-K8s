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
//! Prometheus metrics for service discovery events and topology changes

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct DiscoveryMetrics {
    pub services_registered: Arc<AtomicU64>,
    pub services_deregistered: Arc<AtomicU64>,
    pub topology_changes: Arc<AtomicU64>,
    pub health_checks_performed: Arc<AtomicU64>,
    pub routing_decisions_made: Arc<AtomicU64>,
    pub stale_pruned: Arc<AtomicU64>,
}

impl DiscoveryMetrics {
    pub fn record_registration(&self) {
        self.services_registered.fetch_add(1, Ordering::Relaxed);
        self.topology_changes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_deregistration(&self) {
        self.services_deregistered.fetch_add(1, Ordering::Relaxed);
        self.topology_changes.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_health_check(&self) {
        self.health_checks_performed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_routing_decision(&self) {
        self.routing_decisions_made.fetch_add(1, Ordering::Relaxed);
    }

    pub fn to_prometheus(&self) -> String {
        format!(
            "# TYPE stellar_sd_services_registered_total counter\n\
             stellar_sd_services_registered_total {}\n\
             # TYPE stellar_sd_topology_changes_total counter\n\
             stellar_sd_topology_changes_total {}\n\
             # TYPE stellar_sd_health_checks_total counter\n\
             stellar_sd_health_checks_total {}\n\
             # TYPE stellar_sd_routing_decisions_total counter\n\
             stellar_sd_routing_decisions_total {}\n",
            self.services_registered.load(Ordering::Relaxed),
            self.topology_changes.load(Ordering::Relaxed),
            self.health_checks_performed.load(Ordering::Relaxed),
            self.routing_decisions_made.load(Ordering::Relaxed),
        )
    }
}
