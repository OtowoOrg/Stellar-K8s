import { useMemo, useState, useCallback } from 'react';
import { STELLAR_METRICS, OPERATORS, buildAlertExpr, isValidForDuration } from '../lib/promqlGenerator.js';
import { renderPrometheusRule } from '../lib/yamlExporter.js';
import PromqlPreview from './PromqlPreview.jsx';

const METRIC_NAMES = Object.keys(STELLAR_METRICS);

function emptyComparison() {
  return {
    id: crypto.randomUUID(),
    metric: METRIC_NAMES[0],
    operator: '>',
    threshold: 3,
    useIncrease: false,
    rangeWindow: '5m',
  };
}

function emptyRuleForm() {
  return {
    name: '',
    forDuration: '5m',
    severity: 'warning',
    team: 'stellar-infra',
    component: '',
    summary: '',
    description: '',
    joiner: 'and',
    comparisons: [emptyComparison()],
  };
}

export default function AlertBuilder() {
  const [form, setForm] = useState(emptyRuleForm);
  const [testState, setTestState] = useState({ status: 'idle', message: '' });

  const updateField = useCallback((field, value) => {
    setForm((prev) => ({ ...prev, [field]: value }));
  }, []);

  const updateComparison = useCallback((id, patch) => {
    setForm((prev) => ({
      ...prev,
      comparisons: prev.comparisons.map((c) => (c.id === id ? { ...c, ...patch } : c)),
    }));
  }, []);

  const addComparison = useCallback(() => {
    setForm((prev) => ({ ...prev, comparisons: [...prev.comparisons, emptyComparison()] }));
  }, []);

  const removeComparison = useCallback((id) => {
    setForm((prev) => ({
      ...prev,
      comparisons: prev.comparisons.length > 1 ? prev.comparisons.filter((c) => c.id !== id) : prev.comparisons,
    }));
  }, []);

  const { expr, exprError } = useMemo(() => {
    try {
      return { expr: buildAlertExpr({ comparisons: form.comparisons, joiner: form.joiner }), exprError: null };
    } catch (err) {
      return { expr: '', exprError: err.message };
    }
  }, [form.comparisons, form.joiner]);

  const { yaml, yamlError } = useMemo(() => {
    if (exprError || !form.name || !form.component || !form.summary) {
      return { yaml: '', yamlError: null };
    }
    try {
      const rendered = renderPrometheusRule({
        name: `stellar-custom-${form.name.toLowerCase()}`,
        groupName: 'stellar.custom.alerts',
        extraLabels: { release: 'kube-prometheus-stack' },
        rules: [
          {
            name: form.name,
            condition: { comparisons: form.comparisons, joiner: form.joiner },
            forDuration: form.forDuration,
            severity: form.severity,
            team: form.team,
            component: form.component,
            summary: form.summary,
            description: form.description,
          },
        ],
      });
      return { yaml: rendered, yamlError: null };
    } catch (err) {
      return { yaml: '', yamlError: err.message };
    }
  }, [form, exprError]);

  const canTest = Boolean(yaml && !yamlError && !exprError);

  const runPrometheusTest = useCallback(async () => {
    if (!canTest) return;
    setTestState({ status: 'loading', message: '' });
    try {
      const res = await fetch('/api/v1/alerts/test', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ expr }),
      });

      const contentType = res.headers.get('content-type') || '';
      if (!contentType.includes('application/json')) {
        setTestState({
          status: 'error',
          message: `Could not reach the Prometheus test endpoint (HTTP ${res.status}). Is the backend running and PROMETHEUS_URL configured?`,
        });
        return;
      }

      const data = await res.json();
      if (!res.ok) {
        setTestState({ status: 'error', message: data.message || `Request failed (${res.status})` });
        return;
      }
      setTestState({
        status: 'success',
        message: data.currentlyFiring
          ? `Valid PromQL. Condition is CURRENTLY TRUE (${data.sampleCount} series matched).`
          : `Valid PromQL. Condition is not currently true (${data.sampleCount} series evaluated).`,
      });
    } catch (err) {
      setTestState({ status: 'error', message: `Network error: ${err.message}` });
    }
  }, [canTest, expr]);

  return (
    <main className="app-shell">
      <header className="topbar">
        <div className="brand-block">
          <span className="eyebrow">STELLAR / OBSERVABILITY</span>
          <h1>Alert rule builder</h1>
          <p>Construct, preview, and export Prometheus alert rules for Stellar-K8s.</p>
        </div>
      </header>

      <section className="workspace">
        <div className="graph-panel">
          <div className="panel-heading">
            <strong>Rule metadata</strong>
          </div>

          <div className="form-grid">
            <label className="select-wrap">
              <span>Alert name (PascalCase)</span>
              <input
                type="text"
                value={form.name}
                onChange={(e) => updateField('name', e.target.value)}
                placeholder="StellarLedgerCloseDelay"
              />
            </label>

            <label className="select-wrap">
              <span>Severity</span>
              <select value={form.severity} onChange={(e) => updateField('severity', e.target.value)}>
                <option value="critical">critical</option>
                <option value="warning">warning</option>
                <option value="info">info</option>
              </select>
            </label>

            <label className="select-wrap">
              <span>Team</span>
              <input type="text" value={form.team} onChange={(e) => updateField('team', e.target.value)} />
            </label>

            <label className="select-wrap">
              <span>Component</span>
              <input
                type="text"
                value={form.component}
                onChange={(e) => updateField('component', e.target.value)}
                placeholder="fork-detector"
              />
            </label>

            <label className="select-wrap">
              <span>For duration</span>
              <input
                type="text"
                value={form.forDuration}
                onChange={(e) => updateField('forDuration', e.target.value)}
                placeholder="5m"
              />
              {!isValidForDuration(form.forDuration) && (
                <span className="field-error">Must match Prometheus duration format, e.g. "5m", "1h"</span>
              )}
            </label>

            <label className="select-wrap">
              <span>Comparisons joined by</span>
              <select value={form.joiner} onChange={(e) => updateField('joiner', e.target.value)}>
                <option value="and">AND</option>
                <option value="or">OR</option>
              </select>
            </label>
          </div>

          <label className="select-wrap">
            <span>Summary</span>
            <input
              type="text"
              value={form.summary}
              onChange={(e) => updateField('summary', e.target.value)}
              placeholder="Ledger close delay exceeded threshold"
            />
          </label>

          <label className="select-wrap">
            <span>Description</span>
            <textarea
              value={form.description}
              onChange={(e) => updateField('description', e.target.value)}
              rows={3}
              placeholder="Detailed runbook-style description shown in the alert annotation."
            />
          </label>

          <div className="panel-heading">
            <strong>Conditions</strong>
            <button type="button" className="tool-button" onClick={addComparison}>
              + Add condition
            </button>
          </div>

          {form.comparisons.map((comparison, index) => (
            <ComparisonRow
              key={comparison.id}
              comparison={comparison}
              index={index}
              onChange={(patch) => updateComparison(comparison.id, patch)}
              onRemove={() => removeComparison(comparison.id)}
              removable={form.comparisons.length > 1}
            />
          ))}
        </div>

        <aside className="inspector" aria-live="polite">
          <span className="eyebrow">LIVE PREVIEW</span>
          <PromqlPreview expr={expr} error={exprError} />

          <div className="panel-heading">
            <strong>Exported YAML</strong>
          </div>
          {yamlError && <div className="field-error">{yamlError}</div>}
          <pre className="yaml-preview">{yaml || '// Fill in all required fields to generate YAML'}</pre>

          <button
            type="button"
            className="tool-button"
            disabled={!canTest || testState.status === 'loading'}
            onClick={runPrometheusTest}
          >
            {testState.status === 'loading' ? 'Testing…' : 'Test against Prometheus'}
          </button>
          {testState.status === 'success' && <div className="test-success">{testState.message}</div>}
          {testState.status === 'error' && <div className="field-error">{testState.message}</div>}
        </aside>
      </section>
    </main>
  );
}

function ComparisonRow({ comparison, index, onChange, onRemove, removable }) {
  const metricInfo = STELLAR_METRICS[comparison.metric];
  const isCounter = metricInfo?.type === 'counter';

  return (
    <div className="comparison-row">
      <span className="muted">#{index + 1}</span>

      <select value={comparison.metric} onChange={(e) => onChange({ metric: e.target.value })}>
        {Object.entries(STELLAR_METRICS).map(([name, info]) => (
          <option key={name} value={name}>
            {info.label}
          </option>
        ))}
      </select>

      {isCounter && (
        <label className="inline-check">
          <input
            type="checkbox"
            checked={comparison.useIncrease}
            onChange={(e) => onChange({ useIncrease: e.target.checked })}
          />
          <span>increase() over</span>
          <input
            type="text"
            className="range-input"
            value={comparison.rangeWindow}
            onChange={(e) => onChange({ rangeWindow: e.target.value })}
            disabled={!comparison.useIncrease}
            placeholder="1h"
          />
        </label>
      )}

      <select value={comparison.operator} onChange={(e) => onChange({ operator: e.target.value })}>
        {OPERATORS.map((op) => (
          <option key={op} value={op}>
            {op}
          </option>
        ))}
      </select>

      <input
        type="number"
        value={comparison.threshold}
        onChange={(e) => onChange({ threshold: Number(e.target.value) })}
        className="threshold-input"
      />
      <span className="muted">{metricInfo?.unit}</span>

      {removable && (
        <button type="button" className="tool-button" onClick={onRemove} aria-label="Remove condition">
          ×
        </button>
      )}
    </div>
  );
}
