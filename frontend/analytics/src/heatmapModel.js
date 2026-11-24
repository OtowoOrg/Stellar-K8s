/**
 * heatmapModel.js
 *
 * Data model for the Resource Saturation Heatmap.
 *
 * Parses raw Prometheus metric samples for `stellar_operator_resource_usage`
 * into a normalized per-node structure. All mutation is pure (no in-place
 * state mutation) so callers can use React state immutably.
 *
 * Saturation levels:
 *   idle        < 40 %
 *   moderate   40 – 69 %
 *   elevated   70 – 84 %
 *   high       85 – 94 %
 *   critical   ≥ 95 %
 */

/** Maximum worker nodes the heatmap will render before the oldest drop off. */
export const MAX_NODES = 100;

/** Poll interval in milliseconds (5 s per the issue spec). */
export const POLL_INTERVAL_MS = 5_000;

/**
 * Saturation thresholds → band name.
 * Ordered from highest to lowest so the first match wins.
 */
const SATURATION_BANDS = [
  { threshold: 95, band: 'critical' },
  { threshold: 85, band: 'high' },
  { threshold: 70, band: 'elevated' },
  { threshold: 40, band: 'moderate' },
  { threshold: 0, band: 'idle' },
];

/**
 * Returns the saturation band name for a given percentage value (0–100).
 * @param {number} pct
 * @returns {'idle'|'moderate'|'elevated'|'high'|'critical'}
 */
export function saturationBand(pct) {
  for (const { threshold, band } of SATURATION_BANDS) {
    if (pct >= threshold) return band;
  }
  return 'idle';
}

/**
 * CSS color (as used in the heatmap cells) for each saturation band.
 * These mirror the project color palette: cool blues/greens → hot reds.
 */
export const BAND_COLORS = {
  idle: '#1e3a4a',
  moderate: '#1e5f4e',
  elevated: '#b38c00',
  high: '#c0521c',
  critical: '#c0192b',
};

/**
 * Normalizes a number value, returning `fallback` if it is not finite.
 * @param {unknown} value
 * @param {number} fallback
 * @returns {number}
 */
function asFloat(value, fallback = 0) {
  const n = parseFloat(value);
  return Number.isFinite(n) ? n : fallback;
}

/**
 * Clamps a number to [0, 100].
 * @param {number} n
 * @returns {number}
 */
function clamp100(n) {
  return Math.min(100, Math.max(0, n));
}

/**
 * Builds the canonical node ID used as the heatmap cell key.
 * Prefers pod labels (namespace/pod), falls back to node name alone.
 *
 * @param {Record<string, string>} labels  Prometheus metric label set
 * @returns {string}
 */
function nodeKey(labels) {
  const ns = labels.namespace ?? labels.exported_namespace ?? '';
  const pod = labels.pod ?? labels.exported_pod ?? '';
  const node = labels.node ?? labels.worker_node ?? labels.instance ?? '';
  if (pod) return ns ? `${ns}/${pod}` : pod;
  return node || 'unknown';
}

/**
 * A single normalized worker-node resource record.
 *
 * @typedef {Object} NodeMetric
 * @property {string}  id            Unique key (namespace/pod or node name)
 * @property {string}  node          Kubernetes node name
 * @property {string}  namespace     Pod namespace (empty string if absent)
 * @property {string}  pod           Pod name (empty string if absent)
 * @property {string}  zone          Availability zone / region label
 * @property {number}  cpuPct        CPU utilization 0–100
 * @property {number}  memPct        Memory utilization 0–100
 * @property {number}  saturationPct Composite saturation (max of cpu/mem) 0–100
 * @property {string}  band          Saturation band name
 * @property {boolean} missing       True if the node was absent from the last poll
 * @property {number}  lastSeen      Unix timestamp (ms) of most recent sample
 */

/**
 * Merges a list of raw Prometheus sample objects into a Map of NodeMetric
 * records, keyed by node ID.
 *
 * Each sample object must have:
 *   - `metric`  : Record<string, string>  — Prometheus label set
 *   - `value`   : [timestamp, valueString] — Prometheus instant-vector value
 *   - `resource`: 'cpu' | 'memory'        — which resource the sample covers
 *                 (injected by the caller when it knows from metric name)
 *
 * @param {Array<{metric: Record<string,string>, value: [number,string], resource: string}>} samples
 * @param {Map<string, NodeMetric>} previous  Previous state (for tombstoning)
 * @param {number} nowMs  Current time in milliseconds (for lastSeen)
 * @returns {Map<string, NodeMetric>}
 */
export function mergeSamples(samples, previous = new Map(), nowMs = Date.now()) {
  const next = new Map();

  for (const sample of samples) {
    const labels = sample.metric ?? {};
    const id = nodeKey(labels);
    const pct = clamp100(asFloat(sample.value?.[1]) * 100);
    const existing = next.get(id) ?? {
      id,
      node: labels.node ?? labels.worker_node ?? labels.instance ?? id,
      namespace: labels.namespace ?? labels.exported_namespace ?? '',
      pod: labels.pod ?? labels.exported_pod ?? '',
      zone: labels.zone ?? labels.availability_zone ?? labels.topology_zone ?? '',
      cpuPct: 0,
      memPct: 0,
      saturationPct: 0,
      band: 'idle',
      missing: false,
      lastSeen: nowMs,
    };

    if (sample.resource === 'cpu') {
      existing.cpuPct = pct;
    } else if (sample.resource === 'memory') {
      existing.memPct = pct;
    }

    existing.lastSeen = nowMs;
    next.set(id, existing);
  }

  // Compute composite saturation after all samples for a node are merged.
  for (const record of next.values()) {
    record.saturationPct = clamp100(Math.max(record.cpuPct, record.memPct));
    record.band = saturationBand(record.saturationPct);
    record.missing = false;
  }

  // Tombstone nodes that were present before but absent now.
  for (const [id, prev] of previous.entries()) {
    if (!next.has(id)) {
      next.set(id, { ...prev, missing: true });
    }
  }

  // Enforce the MAX_NODES cap – drop oldest entries by lastSeen.
  if (next.size > MAX_NODES) {
    const sorted = [...next.entries()].sort((a, b) => b[1].lastSeen - a[1].lastSeen);
    return new Map(sorted.slice(0, MAX_NODES));
  }

  return next;
}

/**
 * Parses a Prometheus HTTP API response body into a flat array of annotated
 * sample objects ready for `mergeSamples`.
 *
 * Understands two metric names:
 *   - `stellar_operator_resource_usage{resource="cpu",...}`
 *   - `stellar_operator_resource_usage{resource="memory",...}`
 *
 * If the `resource` label is absent the metric name suffix is used as a
 * fallback (`_cpu_` → cpu, `_memory_` / `_mem_` → memory).
 *
 * @param {unknown} responseBody  Parsed JSON from /api/v1/query or /api/v1/query_range
 * @returns {Array<{metric: Record<string,string>, value: [number,string], resource: string}>}
 */
export function parsePrometheusResponse(responseBody) {
  if (!responseBody || responseBody.status !== 'success') return [];
  const resultType = responseBody.data?.resultType;
  const result = responseBody.data?.result;
  if (!Array.isArray(result)) return [];

  const samples = [];

  for (const item of result) {
    const labels = item.metric ?? {};
    // Determine resource type from label first, then metric name.
    let resource = labels.resource ?? '';
    if (!resource) {
      const metricName = labels.__name__ ?? '';
      if (/_cpu_|_cpu$/.test(metricName)) resource = 'cpu';
      else if (/_mem(?:ory)?_|_mem(?:ory)?$/.test(metricName)) resource = 'memory';
      else resource = 'cpu'; // default
    }
    resource = resource.toLowerCase();
    if (resource !== 'cpu' && resource !== 'memory') continue;

    if (resultType === 'vector' && Array.isArray(item.value)) {
      samples.push({ metric: labels, value: item.value, resource });
    } else if (resultType === 'matrix' && Array.isArray(item.values) && item.values.length > 0) {
      // Use the latest value from a range query.
      const latest = item.values[item.values.length - 1];
      samples.push({ metric: labels, value: latest, resource });
    }
  }

  return samples;
}

/**
 * Applies a freshly fetched Prometheus response onto the previous node map.
 *
 * Convenience wrapper around `parsePrometheusResponse` + `mergeSamples`.
 *
 * @param {unknown} responseBody
 * @param {Map<string, NodeMetric>} previous
 * @param {number} [nowMs]
 * @returns {Map<string, NodeMetric>}
 */
export function applyPrometheusResponse(responseBody, previous = new Map(), nowMs = Date.now()) {
  const samples = parsePrometheusResponse(responseBody);
  return mergeSamples(samples, previous, nowMs);
}

/**
 * Converts the internal Map into a sorted, stable array for rendering.
 * Nodes are grouped by zone, then sorted by saturation descending so the
 * hottest cells always appear first within each group.
 *
 * @param {Map<string, NodeMetric>} nodeMap
 * @returns {NodeMetric[]}
 */
export function materializeNodes(nodeMap) {
  const nodes = [...nodeMap.values()];
  nodes.sort((a, b) => {
    // Zone group first.
    const zoneCmp = (a.zone || '\uffff').localeCompare(b.zone || '\uffff');
    if (zoneCmp !== 0) return zoneCmp;
    // Within zone: hottest first.
    return b.saturationPct - a.saturationPct;
  });
  return nodes;
}
