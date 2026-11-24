/**
 * Real-Time Quorum & Topology Validation Engine for Stellar-K8s Node Topology.
 */

export interface NodeItem {
  id: string;
  name: string;
  nodeType: 'Validator' | 'Horizon' | 'SorobanRpc';
  zone: string;
}

export interface ZoneItem {
  id: string;
  name: string;
}

export interface ValidationSettings {
  maxSkew: number;
}

export interface QuorumValidationResult {
  isValid: boolean;
  quorumSafe: boolean;
  totalNodes: number;
  totalValidators: number;
  activeZonesCount: number;
  zoneValidatorCounts: Record<string, number>;
  zoneTotalCounts: Record<string, number>;
  skew: number;
  errors: string[];
  warnings: string[];
}

/**
 * Validates topology placement for Quorum redundancy and Kubernetes topology spread rules.
 */
export function validateTopologyQuorum(
  zones: ZoneItem[],
  nodes: NodeItem[],
  settings: ValidationSettings = { maxSkew: 1 }
): QuorumValidationResult {
  const errors: string[] = [];
  const warnings: string[] = [];

  const zoneIds = zones.map(z => z.id);
  const zoneValidatorCounts: Record<string, number> = {};
  const zoneTotalCounts: Record<string, number> = {};

  zoneIds.forEach(zId => {
    zoneValidatorCounts[zId] = 0;
    zoneTotalCounts[zId] = 0;
  });

  let unassignedCount = 0;
  let totalValidators = 0;

  nodes.forEach(node => {
    if (!node.zone || !zoneIds.includes(node.zone)) {
      unassignedCount++;
      return;
    }

    zoneTotalCounts[node.zone] = (zoneTotalCounts[node.zone] || 0) + 1;
    if (node.nodeType === 'Validator') {
      zoneValidatorCounts[node.zone] = (zoneValidatorCounts[node.zone] || 0) + 1;
      totalValidators++;
    }
  });

  const activeZones = zoneIds.filter(zId => zoneTotalCounts[zId] > 0);
  const activeZonesCount = activeZones.length;

  if (unassignedCount > 0) {
    warnings.push(`${unassignedCount} node(s) remain unassigned to any availability zone.`);
  }

  // Rule 1: Minimum 3 Availability Zones requirement for quorum redundancy
  if (activeZonesCount < 3) {
    errors.push(
      `Quorum risk: Topology spans only ${activeZonesCount} zone(s). Minimum 3 availability zones required for fault-tolerant quorum.`
    );
  }

  // Rule 2: Majority / Single-zone failure threshold check
  let quorumSafe = activeZonesCount >= 3;
  zoneIds.forEach(zId => {
    const validatorsInZone = zoneValidatorCounts[zId] || 0;
    if (totalValidators > 0 && validatorsInZone >= totalValidators / 2) {
      const percentage = Math.round((validatorsInZone / totalValidators) * 100);
      errors.push(
        `Quorum hazard: Zone '${zId}' contains ${validatorsInZone}/${totalValidators} (${percentage}%) of validator nodes. A single zone failure will lose quorum.`
      );
      quorumSafe = false;
    }
  });

  // Rule 3: Skew calculation across active zones
  const countsArray = activeZones.map(zId => zoneTotalCounts[zId]);
  let skew = 0;
  if (countsArray.length > 0) {
    const maxCount = Math.max(...countsArray);
    const minCount = Math.min(...countsArray);
    skew = maxCount - minCount;

    const maxAllowedSkew = settings.maxSkew ?? 1;
    if (skew > maxAllowedSkew) {
      warnings.push(
        `Topology skew warning: Node distribution skew is ${skew}, which exceeds maxSkew constraint (${maxAllowedSkew}).`
      );
    }
  }

  const isValid = errors.length === 0;

  return {
    isValid,
    quorumSafe,
    totalNodes: nodes.length,
    totalValidators,
    activeZonesCount,
    zoneValidatorCounts,
    zoneTotalCounts,
    skew,
    errors,
    warnings,
  };
}
