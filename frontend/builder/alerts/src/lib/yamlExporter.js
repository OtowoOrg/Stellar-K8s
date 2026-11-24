/**
 * Exports AlertCondition objects as Prometheus Operator PrometheusRule
 * Custom Resource YAML, matching the structure used in
 * monitoring/fork-detector-alerts.yaml.
 */

import { buildAlertExpr, isValidForDuration } from './promqlGenerator.js';

/**
 * @typedef {Object} AlertRule
 * @property {string} name - PascalCase alert name, e.g. "StellarForkDetected"
 * @property {import('./promqlGenerator.js').AlertCondition} condition
 * @property {string} forDuration - Prometheus `for:` duration, e.g. "5m"
 * @property {'critical'|'warning'|'info'} severity
 * @property {string} team
 * @property {string} component
 * @property {string} summary - annotation summary (may use {{ $labels.x }} templating)
 * @property {string} description - annotation description
 */

const VALID_SEVERITIES = new Set(['critical', 'warning', 'info']);

function validateRule(rule) {
  if (!rule.name || !/^[A-Z][A-Za-z0-9]*$/.test(rule.name)) {
    throw new Error(`Alert name must be PascalCase (e.g. "StellarForkDetected"), got: "${rule.name}"`);
  }
  if (!isValidForDuration(rule.forDuration)) {
    throw new Error(`Invalid "for" duration: "${rule.forDuration}"`);
  }
  if (!VALID_SEVERITIES.has(rule.severity)) {
    throw new Error(`Severity must be one of ${[...VALID_SEVERITIES].join(', ')}, got: "${rule.severity}"`);
  }
  if (!rule.team) {
    throw new Error('Rule requires a "team" label');
  }
  if (!rule.component) {
    throw new Error('Rule requires a "component" label');
  }
  if (!rule.summary) {
    throw new Error('Rule requires a "summary" annotation');
  }
}

/** Indent every line of a multi-line string by `spaces` spaces. */
function indent(text, spaces) {
  const pad = ' '.repeat(spaces);
  return text
    .split('\n')
    .map((line) => (line.length ? pad + line : line))
    .join('\n');
}

/**
 * Render a single alert rule as a YAML block, matching this repo's
 * existing multi-line `expr: |` and `description: |` style.
 * @param {AlertRule} rule
 * @returns {string}
 */
export function renderAlertRuleYaml(rule) {
  validateRule(rule);
  const expr = buildAlertExpr(rule.condition);

  const lines = [
    `- alert: ${rule.name}`,
    `  expr: |`,
    indent(expr, 4),
    `  for: ${rule.forDuration}`,
    `  labels:`,
    `    severity: ${rule.severity}`,
    `    team: ${rule.team}`,
    `    component: ${rule.component}`,
    `  annotations:`,
    `    summary: "${rule.summary.replace(/"/g, '\\"')}"`,
  ];

  if (rule.description) {
    lines.push(`    description: |`, indent(rule.description.trim(), 6));
  }

  return lines.join('\n');
}

/**
 * @typedef {Object} PrometheusRuleOptions
 * @property {string} name - metadata.name for the PrometheusRule resource
 * @property {string} namespace - defaults to "monitoring"
 * @property {string} groupName - the alert group name under spec.groups
 * @property {Record<string,string>} [extraLabels] - additional metadata.labels
 * @property {AlertRule[]} rules
 */

/**
 * Render a full PrometheusRule Custom Resource, matching the structure of
 * monitoring/fork-detector-alerts.yaml.
 * @param {PrometheusRuleOptions} options
 * @returns {string}
 */
export function renderPrometheusRule(options) {
  const {
    name,
    namespace = 'monitoring',
    groupName,
    extraLabels = {},
    rules,
  } = options;

  if (!name) throw new Error('PrometheusRule requires metadata.name');
  if (!groupName) throw new Error('PrometheusRule requires a group name');
  if (!rules || rules.length === 0) throw new Error('PrometheusRule requires at least one rule');

  const labelLines = [
    `    app: stellar-operator`,
    ...Object.entries(extraLabels).map(([k, v]) => `    ${k}: ${v}`),
  ];

  const ruleBlocks = rules.map((rule) => indent(renderAlertRuleYaml(rule), 8)).join('\n');

  return [
    `---`,
    `apiVersion: monitoring.coreos.com/v1`,
    `kind: PrometheusRule`,
    `metadata:`,
    `  name: ${name}`,
    `  namespace: ${namespace}`,
    `  labels:`,
    ...labelLines,
    `spec:`,
    `  groups:`,
    `    - name: ${groupName}`,
    `      rules:`,
    ruleBlocks,
    ``,
  ].join('\n');
}
