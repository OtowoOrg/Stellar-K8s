import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildComparisonExpr,
  buildAlertExpr,
  isValidForDuration,
  STELLAR_METRICS,
  OPERATORS,
} from './promqlGenerator.js';

test('buildComparisonExpr: simple gauge comparison', () => {
  const expr = buildComparisonExpr({
    metric: 'stellar_fork_detector_consecutive_diverging_ledgers',
    operator: '>=',
    threshold: 3,
  });
  assert.equal(expr, 'stellar_fork_detector_consecutive_diverging_ledgers >= 3');
});

test('buildComparisonExpr: counter with increase()', () => {
  const expr = buildComparisonExpr({
    metric: 'stellar_watcher_poll_errors_total',
    operator: '>',
    threshold: 0,
    useIncrease: true,
    rangeWindow: '1h',
  });
  assert.equal(expr, 'increase(stellar_watcher_poll_errors_total[1h]) > 0');
});

test('buildComparisonExpr: rejects unknown metric', () => {
  assert.throws(
    () => buildComparisonExpr({ metric: 'not_a_real_metric', operator: '>', threshold: 1 }),
    /Unknown Stellar-K8s metric/,
  );
});

test('buildComparisonExpr: rejects unsupported operator', () => {
  assert.throws(
    () =>
      buildComparisonExpr({
        metric: 'stellar_fork_detector_sync_confidence',
        operator: '~=',
        threshold: 500,
      }),
    /Unsupported operator/,
  );
});

test('buildComparisonExpr: rejects non-numeric threshold', () => {
  assert.throws(
    () =>
      buildComparisonExpr({
        metric: 'stellar_fork_detector_sync_confidence',
        operator: '<',
        threshold: NaN,
      }),
    /Threshold must be a finite number/,
  );
});

test('buildComparisonExpr: rejects increase() on a gauge metric', () => {
  assert.throws(
    () =>
      buildComparisonExpr({
        metric: 'stellar_fork_detector_sync_confidence',
        operator: '<',
        threshold: 500,
        useIncrease: true,
        rangeWindow: '1h',
      }),
    /increase\(\) can only be applied to counter metrics/,
  );
});

test('buildComparisonExpr: rejects invalid rangeWindow', () => {
  assert.throws(
    () =>
      buildComparisonExpr({
        metric: 'stellar_watcher_poll_errors_total',
        operator: '>',
        threshold: 0,
        useIncrease: true,
        rangeWindow: 'one hour',
      }),
    /rangeWindow must be a valid Prometheus duration/,
  );
});

test('buildAlertExpr: single comparison returns bare expr', () => {
  const expr = buildAlertExpr({
    comparisons: [
      { metric: 'stellar_fork_detector_sync_confidence', operator: '<', threshold: 500 },
    ],
  });
  assert.equal(expr, 'stellar_fork_detector_sync_confidence < 500');
});

test('buildAlertExpr: two comparisons joined with AND (default)', () => {
  const expr = buildAlertExpr({
    comparisons: [
      { metric: 'stellar_fork_detector_consecutive_diverging_ledgers', operator: '>=', threshold: 3 },
      { metric: 'stellar_fork_detector_responding_anchors', operator: '<', threshold: 8 },
    ],
  });
  assert.equal(
    expr,
    'stellar_fork_detector_consecutive_diverging_ledgers >= 3\nand\nstellar_fork_detector_responding_anchors < 8',
  );
});

test('buildAlertExpr: two comparisons joined with OR when specified', () => {
  const expr = buildAlertExpr({
    comparisons: [
      { metric: 'stellar_fork_detector_responding_anchors', operator: '==', threshold: 0 },
      { metric: 'stellar_watcher_poll_errors_total', operator: '>', threshold: 5, useIncrease: true, rangeWindow: '5m' },
    ],
    joiner: 'or',
  });
  assert.equal(
    expr,
    'stellar_fork_detector_responding_anchors == 0\nor\nincrease(stellar_watcher_poll_errors_total[5m]) > 5',
  );
});

test('buildAlertExpr: three comparisons all joined the same way', () => {
  const expr = buildAlertExpr({
    comparisons: [
      { metric: 'stellar_fork_detector_consecutive_diverging_ledgers', operator: '>=', threshold: 3 },
      { metric: 'stellar_fork_detector_responding_anchors', operator: '<', threshold: 8 },
      { metric: 'stellar_fork_detector_sync_confidence', operator: '<', threshold: 500 },
    ],
  });
  assert.equal(
    expr,
    [
      'stellar_fork_detector_consecutive_diverging_ledgers >= 3',
      'stellar_fork_detector_responding_anchors < 8',
      'stellar_fork_detector_sync_confidence < 500',
    ].join('\nand\n'),
  );
});

test('buildAlertExpr: throws on empty comparisons array', () => {
  assert.throws(() => buildAlertExpr({ comparisons: [] }), /requires at least one comparison/);
});

test('isValidForDuration: accepts valid Prometheus durations', () => {
  assert.equal(isValidForDuration('0m'), true);
  assert.equal(isValidForDuration('5m'), true);
  assert.equal(isValidForDuration('1h'), true);
  assert.equal(isValidForDuration('2d'), true);
});

test('isValidForDuration: rejects invalid durations', () => {
  assert.equal(isValidForDuration('five minutes'), false);
  assert.equal(isValidForDuration('5'), false);
  assert.equal(isValidForDuration(''), false);
});

test('STELLAR_METRICS: contains all 7 canonical metrics from monitoring/*.yaml', () => {
  const expected = [
    'stellar_cve_critical_alerts_total',
    'stellar_fork_detector_consecutive_diverging_ledgers',
    'stellar_fork_detector_responding_anchors',
    'stellar_fork_detector_sync_confidence',
    'stellar_watcher_last_poll_timestamp_seconds',
    'stellar_watcher_ledger_hash',
    'stellar_watcher_poll_errors_total',
  ];
  assert.deepEqual(Object.keys(STELLAR_METRICS).sort(), expected.sort());
});

test('OPERATORS: contains all standard comparison operators', () => {
  assert.deepEqual(OPERATORS, ['>', '>=', '<', '<=', '==', '!=']);
});
