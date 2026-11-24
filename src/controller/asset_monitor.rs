use std::collections::HashMap;
use std::sync::Mutex;

use once_cell::sync::Lazy;
use tracing::warn;

use crate::controller::metrics::{
    AssetLabels, ASSET_CLAWBACK_EVENTS_TOTAL, ASSET_HOLDERS, ASSET_LARGE_SUPPLY_CHANGES_TOTAL,
    ASSET_LIQUIDITY_STROOPS, ASSET_SUPPLY_CHANGE_PERCENT, ASSET_SUPPLY_STROOPS,
};
use crate::crd::StellarAssetMonitor;
use kube::ResourceExt;

static PREVIOUS_SUPPLY: Lazy<Mutex<HashMap<AssetLabels, i64>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Ledger entry types consumed by the asset monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetLedgerEntryKind {
    TrustLine,
    ContractData,
}

/// Decoded asset state derived from a ledger entry or an associated ledger update.
/// Supply and liquidity values use stroops (10^7 per asset unit).
#[derive(Clone, Debug)]
pub struct AssetLedgerChange {
    pub kind: AssetLedgerEntryKind,
    pub ledger_sequence: u64,
    pub asset_code: String,
    pub issuer: Option<String>,
    pub contract_id: Option<String>,
    pub supply_stroops: i64,
    pub holders: i64,
    pub liquidity_stroops: i64,
    pub clawback: bool,
}

/// Update metrics for a decoded ledger change. Returns false when disabled or unwatched.
/// The upstream ledger adapter should call this once for each relevant ledger update.
pub fn process_ledger_change(monitor: &StellarAssetMonitor, change: &AssetLedgerChange) -> bool {
    let spec = &monitor.spec;
    if !spec.enabled
        || spec.watch_list.is_empty()
        || !spec.large_supply_change_percent.is_finite()
        || spec.large_supply_change_percent <= 0.0
        || !matches!(
            change.kind,
            AssetLedgerEntryKind::TrustLine | AssetLedgerEntryKind::ContractData
        )
    {
        return false;
    }

    let matched = spec.watch_list.iter().any(|asset| {
        asset
            .contract_id
            .as_deref()
            .zip(change.contract_id.as_deref())
            .is_some_and(|(expected, actual)| expected == actual)
            || (asset.asset_code == change.asset_code
                && asset
                    .issuer
                    .as_deref()
                    .zip(change.issuer.as_deref())
                    .is_some_and(|(expected, actual)| expected == actual))
    });
    if !matched {
        return false;
    }

    let labels = AssetLabels {
        namespace: monitor.namespace().unwrap_or_default(),
        monitor: monitor.name_any(),
        network: spec.network.clone(),
        asset_code: change.asset_code.clone(),
        issuer: change.issuer.clone().unwrap_or_default(),
        contract_id: change.contract_id.clone().unwrap_or_default(),
    };

    ASSET_SUPPLY_STROOPS
        .get_or_create(&labels)
        .set(change.supply_stroops);
    ASSET_HOLDERS.get_or_create(&labels).set(change.holders);
    ASSET_LIQUIDITY_STROOPS
        .get_or_create(&labels)
        .set(change.liquidity_stroops);

    let supply_change_percent = match PREVIOUS_SUPPLY.lock() {
        Ok(mut previous) => {
            let delta = previous
                .insert(labels.clone(), change.supply_stroops)
                .map(|old| {
                    if old == 0 {
                        if change.supply_stroops == 0 {
                            0.0
                        } else {
                            100.0
                        }
                    } else {
                        change.supply_stroops.saturating_sub(old) as f64 / old.unsigned_abs() as f64
                            * 100.0
                    }
                })
                .unwrap_or(0.0);
            delta
        }
        Err(_) => {
            warn!("Asset monitor supply cache lock is poisoned");
            return false;
        }
    };

    ASSET_SUPPLY_CHANGE_PERCENT
        .get_or_create(&labels)
        .set(supply_change_percent);
    if supply_change_percent.abs() >= spec.large_supply_change_percent {
        ASSET_LARGE_SUPPLY_CHANGES_TOTAL
            .get_or_create(&labels)
            .inc();
    }

    if change.clawback {
        ASSET_CLAWBACK_EVENTS_TOTAL.get_or_create(&labels).inc();
    }

    tracing::debug!(
        ledger_sequence = change.ledger_sequence,
        asset_code = %change.asset_code,
        "Updated watched asset metrics"
    );

    true
}
