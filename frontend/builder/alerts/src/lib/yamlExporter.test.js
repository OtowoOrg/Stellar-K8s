import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderAlertRuleYaml, renderPrometheusRule } from './yamlExporter.js';

const baseRule = () => ({
  name: 'StellarForkDetected',
  condition: {
    comparisons: [
      { metric: 'stellar_fork_detector_consecutive_diverging_ledgers', operator: '>=', threshold: 3 },
    ],
  },
  forDuration: '0m',
  severity: 'critical',
  team: 'stellar-infra',
  component: 'fork-detector',
  summary: 'Potential network fork detected',
  description: 'The local node has diverged from the anchor majority.',
});

test('renderAlertRuleYaml: produces expected structure for single comparison', () => {
  const yaml = renderAlertRuleYaml(baseRule());
  assert.match(yaml, /^- alert: StellarForkDetected$/m);
  assert.match(yaml, /^ {2}expr: \|$/m);
  assert.match(yaml, /^ {4}stellar_fork_detector_consecutive_diverging_ledgers >= 3$/m);
  assert.match(yaml, /^ {2}for: 0m$/m);
  assert.match(yaml, /^ {4}severity: critical$/m);
  assert.match(yaml, /^ {4}team: stellar-infra$/m);
  assert.match(yaml, /^ {4}component: fork-detector$/m);
  assert.match(yaml, /^ {4}summary: "Potential network fork detected"$/m);
});

test('renderAlertRuleYaml: includes description block when provided', () => {
  const yaml = renderAlertRuleYaml(baseRule());
  assert.match(yaml, /^ {4}description: \|$/m);
  assert.match(yaml, /^ {6}The local node has diverged/m);
});

test('renderAlertRuleYaml: omits description block when not provided', () => {
  const rule = baseRule();
  delete rule.description;
  const yaml = renderAlertRuleYaml(rule);
  assert.doesNotMatch(yaml, /description:/);
});

test('renderAlertRuleYaml: escapes double quotes in summary', () => {
  const rule = baseRule();
  rule.summary = 'Node "abc" diverged';
  const yaml = renderAlertRuleYaml(rule);
  assert.match(yaml, /summary: "Node \\"abc\\" diverged"/);
});

test('renderAlertRuleYaml: rejects non-PascalCase name', () => {
  const rule = baseRule();
  rule.name = 'stellar-fork-detected';
  assert.throws(() => renderAlertRuleYaml(rule), /must be PascalCase/);
});

test('renderAlertRuleYaml: rejects invalid severity', () => {
  const rule = baseRule();
  rule.severity = 'urgent';
  assert.throws(() => renderAlertRuleYaml(rule), /Severity must be one of/);
});

test('renderAlertRuleYaml: rejects invalid for duration', () => {
  const rule = baseRule();
  rule.forDuration = 'soon';
  assert.throws(() => renderAlertRuleYaml(rule), /Invalid "for" duration/);
});

test('renderAlertRuleYaml: multi-comparison expr is indented correctly under expr: |', () => {
  const rule = baseRule();
  rule.condition = {
    comparisons: [
      { metric: 'stellar_fork_detector_consecutive_diverging_ledgers', operator: '>=', threshold: 3 },
      { metric: 'stellar_fork_detector_responding_anchors', operator: '<', threshold: 8 },
    ],
  };
  const yaml = renderAlertRuleYaml(rule);
  assert.match(yaml, /^ {4}stellar_fork_detector_consecutive_diverging_ledgers >= 3$/m);
  assert.match(yaml, /^ {4}and$/m);
  assert.match(yaml, /^ {4}stellar_fork_detector_responding_anchors < 8$/m);
});

test('renderPrometheusRule: produces full valid-shaped CR with one rule', () => {
  const yaml = renderPrometheusRule({
    name: 'stellar-custom-alerts',
    groupName: 'stellar.custom.alerts',
    extraLabels: { release: 'kube-prometheus-stack' },
    rules: [baseRule()],
  });

  assert.match(yaml, /^---$/m);
  assert.match(yaml, /^apiVersion: monitoring\.coreos\.com\/v1$/m);
  assert.match(yaml, /^kind: PrometheusRule$/m);
  assert.match(yaml, /^ {2}name: stellar-custom-alerts$/m);
  assert.match(yaml, /^ {2}namespace: monitoring$/m);
  assert.match(yaml, /^ {4}app: stellar-operator$/m);
  assert.match(yaml, /^ {4}release: kube-prometheus-stack$/m);
  assert.match(yaml, /^ {4}- name: stellar\.custom\.alerts$/m);
  assert.match(yaml, / {8}- alert: StellarForkDetected$/m);
});

test('renderPrometheusRule: defaults namespace to "monitoring"', () => {
  const yaml = renderPrometheusRule({
    name: 'stellar-custom-alerts',
    groupName: 'stellar.custom.alerts',
    rules: [baseRule()],
  });
  assert.match(yaml, /^ {2}namespace: monitoring$/m);
});

test('renderPrometheusRule: supports multiple rules under one group', () => {
  const secondRule = baseRule();
  secondRule.name = 'StellarForkLowSyncConfidence';
  secondRule.condition = {
    comparisons: [
      { metric: 'stellar_fork_detector_sync_confidence', operator: '<', threshold: 500 },
    ],
  };
  secondRule.forDuration = '2m';
  secondRule.severity = 'warning';

  const yaml = renderPrometheusRule({
    name: 'stellar-custom-alerts',
    groupName: 'stellar.custom.alerts',
    rules: [baseRule(), secondRule],
  });

  assert.match(yaml, /- alert: StellarForkDetected/);
  assert.match(yaml, /- alert: StellarForkLowSyncConfidence/);
});

test('renderPrometheusRule: throws on missing name', () => {
  assert.throws(
    () => renderPrometheusRule({ groupName: 'g', rules: [baseRule()] }),
    /requires metadata\.name/,
  );
});

test('renderPrometheusRule: throws on empty rules array', () => {
  assert.throws(
    () => renderPrometheusRule({ name: 'x', groupName: 'g', rules: [] }),
    /requires at least one rule/,
  );
});
