//! Fuzzing entry point.

use super::prelude::*;
use super::reconcile::reconcile;
use super::state::ControllerState;

/// Public entry point for state-machine fuzzing.
/// Only compiled when the `reconciler-fuzz` feature is enabled.
#[cfg(feature = "reconciler-fuzz")]
pub async fn reconcile_for_fuzz(
    obj: Arc<StellarNode>,
    ctx: Arc<ControllerState>,
) -> Result<Action> {
    reconcile(obj, ctx).await
}
