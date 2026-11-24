/**
 * Quorum Validator — validates a TopologyState for Stellar quorum redundancy.
 *
 * The Stellar Consensus Protocol (SCP) requires validators to be spread across
 * independent failure domains (availability zones) so that no single zone
 * outage can prevent quorum. This module encodes those rules as a set of
 * structured errors and warnings that the configurator UI surfaces to the user
 * before exporting Kubernetes manifests.
 *
 * Usage:
 *   import { validateTopology } from './quorum_validator';
 *   const result = validateTopology(state);
 *   if (!result.valid) { ... }
 */

import type {
  TopologyState,
  PlacedStellarNode,
  ValidationError,
  ValidationWarning,
  ValidationResult,
} from './types';

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/**
 * Collects all PlacedStellarNode entries whose nodeType is 'Validator'.
 */
function getValidatorNodes(state: TopologyState): PlacedStellarNode[] {
  return state.placedNodes.filter((n) => n.nodeType === 'Validator');
}

/**
 * Builds a map from zoneId → list of Validator nodes placed in that zone.
 * Only zones that appear in `state.zones` are considered.
 */
function buildValidatorsByZone(
  validators: PlacedStellarNode[],
  state: TopologyState,
): Map<string, PlacedStellarNode[]> {
  const zoneIds = new Set(state.zones.map((z) => z.id));
  const map = new Map<string, PlacedStellarNode[]>();

  for (const v of validators) {
    if (!zoneIds.has(v.availabilityZoneId)) continue;
    const list = map.get(v.availabilityZoneId) ?? [];
    list.push(v);
    map.set(v.availabilityZoneId, list);
  }

  return map;
}

/**
 * Returns the zone name for a given zone ID, falling back to the raw ID if
 * the zone is not found (defensive guard for partial states).
 */
function zoneName(state: TopologyState, zoneId: string): string {
  return state.zones.find((z) => z.id === zoneId)?.name ?? zoneId;
}

/**
 * Returns the total replica count for a list of Validator nodes.
 * Each PlacedStellarNode's `replicas` field is summed.
 */
function totalReplicas(validators: PlacedStellarNode[]): number {
  return validators.reduce((sum, v) => sum + (v.replicas ?? 1), 0);
}

// ---------------------------------------------------------------------------
// Validation rules
// ---------------------------------------------------------------------------

/**
 * RULE 1 — ERROR: INSUFFICIENT_ZONES
 *
 * Stellar quorum is only meaningful when validators are distributed across at
 * least 3 independent failure domains. If fewer than 3 zones contain at least
 * one Validator the topology cannot survive a single-zone failure.
 */
function checkInsufficientZones(
  validators: PlacedStellarNode[],
  validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationError | null {
  if (validators.length === 0) return null; // No validators → skip

  const zonesWithValidators = validatorsByZone.size;
  if (zonesWithValidators >= 3) return null;

  return {
    code: 'INSUFFICIENT_ZONES',
    message:
      `Validators must be spread across at least 3 availability zones for quorum ` +
      `fault tolerance. Currently ${zonesWithValidators} zone(s) contain validators.`,
    zoneIds: [...validatorsByZone.keys()],
  };
}

/**
 * RULE 2 — ERROR: ZONE_MISSING_VALIDATOR
 *
 * When validators exist, every zone that has assigned worker nodes should also
 * host at least one Validator. A zone with workers but no validator represents
 * an under-utilised failure domain and creates an uneven quorum distribution.
 */
function checkZoneMissingValidator(
  validators: PlacedStellarNode[],
  validatorsByZone: Map<string, PlacedStellarNode[]>,
  state: TopologyState,
): ValidationError | null {
  if (validators.length === 0) return null;

  // Zones that have at least one assigned worker node
  const zonesWithWorkers = state.zones.filter((z) => z.workerNodeIds.length > 0);
  const missingZoneIds = zonesWithWorkers
    .filter((z) => !validatorsByZone.has(z.id))
    .map((z) => z.id);

  if (missingZoneIds.length === 0) return null;

  const names = missingZoneIds.map((id) => zoneName(state, id)).join(', ');
  return {
    code: 'ZONE_MISSING_VALIDATOR',
    message:
      `The following zone(s) have worker nodes assigned but no Validator placed: ` +
      `${names}. Each zone with capacity should host at least one Validator.`,
    zoneIds: missingZoneIds,
  };
}

/**
 * RULE 3 — ERROR: QUORUM_BELOW_THRESHOLD
 *
 * Stellar SCP requires a minimum of 3 validator replicas to form a quorum
 * slice. Fewer than 3 total replicas means consensus is impossible.
 */
function checkQuorumBelowThreshold(
  validators: PlacedStellarNode[],
  _validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationError | null {
  if (validators.length === 0) return null;

  const total = totalReplicas(validators);
  if (total >= 3) return null;

  return {
    code: 'QUORUM_BELOW_THRESHOLD',
    message:
      `Total Validator replica count is ${total}. At least 3 replicas are required ` +
      `for quorum to be achievable. Increase replica counts or add more Validator nodes.`,
    zoneIds: [],
  };
}

/**
 * RULE 4 — ERROR: SINGLE_ZONE_VALIDATORS
 *
 * When all Validators reside in a single availability zone, a zone failure
 * takes down the entire validator set, eliminating fault tolerance entirely.
 */
function checkSingleZoneValidators(
  validators: PlacedStellarNode[],
  validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationError | null {
  if (validators.length === 0) return null;
  if (validatorsByZone.size !== 1) return null;

  const singleZoneId = [...validatorsByZone.keys()][0];
  return {
    code: 'SINGLE_ZONE_VALIDATORS',
    message:
      `All Validators are placed in a single availability zone (${singleZoneId}). ` +
      `A zone failure will eliminate all validators. Distribute validators across ` +
      `multiple zones to achieve fault tolerance.`,
    zoneIds: [singleZoneId],
  };
}

/**
 * RULE 5 — WARNING: UNEVEN_DISTRIBUTION
 *
 * Validators should be distributed roughly evenly across zones. If the
 * difference between the maximum and minimum replica counts per zone exceeds 2,
 * quorum can become asymmetric — some zones carry disproportionate load and
 * their failure has an outsized impact.
 */
function checkUnevenDistribution(
  validators: PlacedStellarNode[],
  validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationWarning | null {
  if (validators.length === 0 || validatorsByZone.size < 2) return null;

  const replicasPerZone = [...validatorsByZone.values()].map(totalReplicas);
  const max = Math.max(...replicasPerZone);
  const min = Math.min(...replicasPerZone);

  if (max - min <= 2) return null;

  return {
    code: 'UNEVEN_DISTRIBUTION',
    message:
      `Validator replica counts across zones differ by ${max - min} ` +
      `(max: ${max}, min: ${min}). An imbalance greater than 2 can create ` +
      `asymmetric quorum slices. Consider evening out the distribution.`,
    zoneIds: [...validatorsByZone.keys()],
  };
}

/**
 * RULE 6 — WARNING: NO_HISTORY_ARCHIVE
 *
 * Stellar history archives are the mechanism by which new nodes catch up to
 * the network. If no Validator has `enableHistoryArchive: true`, new peers
 * cannot bootstrap from this deployment.
 */
function checkNoHistoryArchive(
  validators: PlacedStellarNode[],
  _validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationWarning | null {
  if (validators.length === 0) return null;

  const hasArchive = validators.some(
    (v) => v.validatorConfig?.enableHistoryArchive === true,
  );
  if (hasArchive) return null;

  return {
    code: 'NO_HISTORY_ARCHIVE',
    message:
      'None of the Validator nodes have `enableHistoryArchive` set to true. ' +
      'Without a history archive, new nodes cannot bootstrap from this deployment. ' +
      'Enable history archiving on at least one Validator.',
    zoneIds: [],
  };
}

/**
 * RULE 7 — WARNING: MISSING_QUORUM_SET
 *
 * Without an explicit quorum set the operator falls back to the network
 * default, which may not reflect the intended trust topology for a production
 * deployment. Configuring an explicit quorum set is strongly recommended.
 */
function checkMissingQuorumSet(
  validators: PlacedStellarNode[],
  _validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationWarning | null {
  if (validators.length === 0) return null;

  const hasQuorumSet = validators.some(
    (v) => v.validatorConfig?.quorumSet != null &&
           v.validatorConfig.quorumSet.trim().length > 0,
  );
  if (hasQuorumSet) return null;

  return {
    code: 'MISSING_QUORUM_SET',
    message:
      'No Validator node has a `quorumSet` configured. Without an explicit quorum ' +
      'set the operator uses the network default, which may not match your intended ' +
      'trust topology. Configure a quorum set on at least one Validator.',
    zoneIds: [],
  };
}

/**
 * RULE 8 — WARNING: SEED_SECRET_MISSING
 *
 * The validator signing key is sourced from a Kubernetes Secret referenced by
 * `seedSecretRef`. If none of the Validator nodes has this field set the
 * operator cannot start a signing validator — they will run in watcher mode
 * only, which does not participate in SCP.
 */
function checkSeedSecretMissing(
  validators: PlacedStellarNode[],
  _validatorsByZone: Map<string, PlacedStellarNode[]>,
  _state: TopologyState,
): ValidationWarning | null {
  if (validators.length === 0) return null;

  const hasSeedSecret = validators.some(
    (v) =>
      v.validatorConfig?.seedSecretRef != null &&
      v.validatorConfig.seedSecretRef.trim().length > 0,
  );
  if (hasSeedSecret) return null;

  return {
    code: 'SEED_SECRET_MISSING',
    message:
      'No Validator node has a `seedSecretRef` configured. Without a signing key ' +
      'secret, validators cannot participate in SCP consensus — they will run in ' +
      'watcher mode only. Set `validatorConfig.seedSecretRef` on each Validator.',
    zoneIds: [],
  };
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/**
 * Validates a topology state for Stellar quorum redundancy and best-practice
 * configuration.
 *
 * The function inspects all `PlacedStellarNode` entries of type `'Validator'`
 * and runs eight rules (4 errors, 4 warnings). The returned `ValidationResult`
 * is suitable for display in the configurator's validation panel and for
 * gate-keeping manifest export.
 *
 * Rules evaluated (in order):
 *
 * **Errors** (block manifest export):
 * 1. `INSUFFICIENT_ZONES`      — Validators must span ≥ 3 zones.
 * 2. `ZONE_MISSING_VALIDATOR`  — Every zone with workers needs ≥ 1 Validator.
 * 3. `QUORUM_BELOW_THRESHOLD`  — Total replicas < 3 prevents quorum.
 * 4. `SINGLE_ZONE_VALIDATORS`  — All validators in one zone = zero fault tolerance.
 *
 * **Warnings** (informational, export still allowed):
 * 5. `UNEVEN_DISTRIBUTION`     — Zone replica counts differ by > 2.
 * 6. `NO_HISTORY_ARCHIVE`      — No Validator publishes a history archive.
 * 7. `MISSING_QUORUM_SET`      — No Validator has an explicit quorum set.
 * 8. `SEED_SECRET_MISSING`     — No Validator has a seed secret reference.
 *
 * @param state - The current TopologyState from the topology store.
 * @returns A ValidationResult with `valid`, `errors`, and `warnings` fields.
 */
export function validateTopology(state: TopologyState): ValidationResult {
  const validators = getValidatorNodes(state);
  const validatorsByZone = buildValidatorsByZone(validators, state);

  // Collect errors — all rules receive the same pre-computed inputs
  const rawErrors: Array<ValidationError | null> = [
    checkInsufficientZones(validators, validatorsByZone, state),
    checkZoneMissingValidator(validators, validatorsByZone, state),
    checkQuorumBelowThreshold(validators, validatorsByZone, state),
    checkSingleZoneValidators(validators, validatorsByZone, state),
  ];

  // Collect warnings
  const rawWarnings: Array<ValidationWarning | null> = [
    checkUnevenDistribution(validators, validatorsByZone, state),
    checkNoHistoryArchive(validators, validatorsByZone, state),
    checkMissingQuorumSet(validators, validatorsByZone, state),
    checkSeedSecretMissing(validators, validatorsByZone, state),
  ];

  const errors = rawErrors.filter((e): e is ValidationError => e !== null);
  const warnings = rawWarnings.filter((w): w is ValidationWarning => w !== null);

  return {
    valid: errors.length === 0,
    errors,
    warnings,
  };
}
