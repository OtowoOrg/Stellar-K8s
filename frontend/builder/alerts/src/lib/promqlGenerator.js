/**
 * PromQL generation for Stellar-K8s alert conditions.
 *
 * A condition is built from one or more comparisons joined by AND/OR,
 * each comparison referencing a known Stellar-K8s metric.
 */

export const STELLAR_METRICS = Object.freeze({
  stellar_fork_detector_consecutive_diverging_ledgers: {
    label: 'Consecutive diverging ledgers',
    unit: 'ledgers',
    type: 'gauge',
  },
  stellar_fork_detector_responding_anchors: {
    label: 'Responding anchor nodes',
    unit: 'count',
    type: 'gauge',
  },
  stellar_fork_detector_sync_confidence: {
    label: 'Sync confidence',
    unit: 'permille',
    type: 'gauge',
  },
  stellar_watcher_last_poll_timestamp_seconds: {
    label: 'Last poll timestamp',
    unit: 'seconds',
    type: 'gauge',
  },
  stellar_watcher_ledger_hash: {
    label: 'Watcher ledger hash',
    unit: 'hash',
    type: 'gauge',
  },
  stellar_watcher_poll_errors_total: {
    label: 'Watcher poll errors (total)',
    unit: 'count',
    type: 'counter',
  },
  stellar_cve_critical_alerts_total: {
    label: 'Critical CVE alerts (total)',
    unit: 'count',
    type: 'counter',
  },
});

export const OPERATORS = Object.freeze(['>', '>=', '<', '<=', '==', '!=']);

const VALID_METRIC_NAMES = new Set(Object.keys(STELLAR_METRICS));

/**
 * @typedef {Object} Comparison
 * @property {string} metric - must be a key in STELLAR_METRICS
 * @property {boolean} [useIncrease] - wrap counter in increase(metric[rangeWindow])
 * @property {string} [rangeWindow] - e.g. "1h", required if useIncrease is true
 * @property {string} operator - one of OPERATORS
 * @property {number} threshold
 */

/**
 * @typedef {Object} AlertCondition
 * @property {Comparison[]} comparisons
 * @property {'and'|'or'} [joiner] - how comparisons combine, defaults to 'and'
 * @property {string} [forDuration] - Prometheus `for:` duration, e.g. "5m"
 */

function validateComparison(comparison) {
  const { metric, operator, threshold, useIncrease, rangeWindow } = comparison;

  if (!VALID_METRIC_NAMES.has(metric)) {
    throw new Error(`Unknown Stellar-K8s metric: "${metric}"`);
  }
  if (!OPERATORS.includes(operator)) {
    throw new Error(`Unsupported operator: "${operator}". Must be one of ${OPERATORS.join(', ')}`);
  }
  if (typeof threshold !== 'number' || Number.isNaN(threshold)) {
    throw new Error(`Threshold must be a finite number, got: ${threshold}`);
  }
  if (useIncrease) {
    if (STELLAR_METRICS[metric].type !== 'counter') {
      throw new Error(`increase() can only be applied to counter metrics; "${metric}" is a ${STELLAR_METRICS[metric].type}`);
    }
    if (!rangeWindow || !/^\d+[smhd]$/.test(rangeWindow)) {
      throw new Error(`rangeWindow must be a valid Prometheus duration (e.g. "1h") when useIncrease is set, got: "${rangeWindow}"`);
    }
  }
}

/**
 * Build the PromQL expression string for a single comparison.
 * @param {Comparison} comparison
 * @returns {string}
 */
export function buildComparisonExpr(comparison) {
  validateComparison(comparison);
  const { metric, operator, threshold, useIncrease, rangeWindow } = comparison;
  const lhs = useIncrease ? `increase(${metric}[${rangeWindow}])` : metric;
  return `${lhs} ${operator} ${threshold}`;
}

/**
 * Build the full multi-line PromQL expression for an alert condition,
 * joining comparisons with AND/OR the way Prometheus expects
 * (each joined clause on its own line, matching this repo's existing
 * multi-line `expr: |` style).
 *
 * @param {AlertCondition} condition
 * @returns {string}
 */
export function buildAlertExpr(condition) {
  if (!condition.comparisons || condition.comparisons.length === 0) {
    throw new Error('An alert condition requires at least one comparison');
  }

  const joiner = condition.joiner === 'or' ? 'or' : 'and';
  const clauses = condition.comparisons.map(buildComparisonExpr);

  if (clauses.length === 1) {
    return clauses[0];
  }

  return clauses.join(`\n${joiner}\n`);
}

/**
 * Validate a `for:` duration string against Prometheus's accepted format.
 * @param {string} duration
 * @returns {boolean}
 */
export function isValidForDuration(duration) {
  return /^\d+[smhd]$/.test(duration);
}
