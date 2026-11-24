// Framework-free helpers for the worker-node resource saturation heatmap.
//
// Everything here is synchronous and runs in O(samples + nodes). A 100-node
// cluster reporting a handful of pods each is a few hundred samples per poll,
// which the React component folds into one animation frame without blocking
// the main thread.

export const DEFAULT_THRESHOLDS = { warm: 0.5, hot: 0.75, critical: 0.9 };

// A worker node that stops reporting is shown as "draining" and then dropped
// once it has been silent for longer than this window.
export const DEFAULT_STALE_MS = 15000;

const RESOURCE_ALIASES = {
  cpu: 'cpu',
  cpu_cores: 'cpu',
  cpu_usage: 'cpu',
  cpu_ratio: 'cpu',
  processor: 'cpu',
  compute: 'cpu',
  memory: 'memory',
  mem: 'memory',
  memory_bytes: 'memory',
  memory_usage: 'memory',
  memory_ratio: 'memory',
  ram: 'memory',
};

const SATURATION_STOPS = [
  [0.0, [37, 99, 235]], // idle - blue
  [0.35, [20, 184, 166]], // cool - teal
  [0.55, [163, 191, 63]], // warm - lime
  [0.7, [234, 179, 8]], // warm - amber
  [0.85, [249, 115, 22]], // hot - orange
  [1.0, [220, 38, 38]], // saturated - red
];

export function normalizeRatio(value) {
  const number = Number(value);
  if (!Number.isFinite(number) || number <= 0) return 0;
  if (number <= 1) return number;
  if (number <= 100) return number / 100;
  return 1;
}

export function normalizeResource(value) {
  const key = String(value ?? 'cpu').trim().toLowerCase();
  if (RESOURCE_ALIASES[key]) return RESOURCE_ALIASES[key];
  return key.includes('mem') ? 'memory' : 'cpu';
}

export function classifySaturation(ratio, thresholds = DEFAULT_THRESHOLDS) {
  const value = normalizeRatio(ratio);
  if (value >= thresholds.critical) return 'critical';
  if (value >= thresholds.hot) return 'hot';
  if (value >= thresholds.warm) return 'warm';
  if (value > 0) return 'cool';
  return 'idle';
}

// Cool (idle) to hot (saturated) spectrum, matching the GitHub-contribution
// grid idea: a single hue ramp the eye can scan for outliers.
export function saturationColor(ratio) {
  const value = normalizeRatio(ratio);
  let low = SATURATION_STOPS[0];
  let high = SATURATION_STOPS[SATURATION_STOPS.length - 1];
  for (let index = 0; index < SATURATION_STOPS.length - 1; index += 1) {
    if (value >= SATURATION_STOPS[index][0] && value <= SATURATION_STOPS[index + 1][0]) {
      low = SATURATION_STOPS[index];
      high = SATURATION_STOPS[index + 1];
      break;
    }
  }
  const span = high[0] - low[0] || 1;
  const t = Math.min(Math.max((value - low[0]) / span, 0), 1);
  const hex = [0, 1, 2]
    .map((channel) => Math.round(low[1][channel] + (high[1][channel] - low[1][channel]) * t))
    .map((component) => component.toString(16).padStart(2, '0'))
    .join('');
  return `#${hex}`;
}

function labelBag(raw) {
  if (!raw) return {};
  return raw.metric ?? raw.labels ?? raw;
}

function firstDefined(bag, names) {
  for (const name of names) {
    if (bag[name] !== undefined && bag[name] !== null && bag[name] !== '') return bag[name];
  }
  return undefined;
}

export function normalizeSample(raw) {
  if (!raw) return null;
  const bag = labelBag(raw);
  const node = String(
    firstDefined(bag, ['node', 'instance', 'nodeName', 'kubernetes_node', 'kubernetes_io_hostname']) ??
      raw.node ??
      '',
  ).trim();
  if (!node) return null;
  const zone =
    String(
      firstDefined(bag, ['zone', 'availability_zone', 'topology_kubernetes_io_zone', 'failure_domain_beta_kubernetes_io_zone']) ??
        raw.zone ??
        '',
    ).trim() || 'unknown';
  const pod = String(firstDefined(bag, ['pod', 'pod_name', 'container']) ?? raw.pod ?? node).trim() || node;
  const resource = normalizeResource(firstDefined(bag, ['resource', 'type', 'kind']) ?? raw.resource ?? 'cpu');
  let value = raw.value;
  if (Array.isArray(value)) value = value[1];
  if (value === undefined) value = raw.val ?? (Array.isArray(raw) ? raw[1] : undefined);
  return { node, zone, pod, resource, value: normalizeRatio(value) };
}

// Prometheus text exposition format, limited to the metric we care about.
export function parsePrometheusText(text) {
  const samples = [];
  for (const line of String(text).split('\n')) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const match = trimmed.match(/^([a-zA-Z_:][\w:]*)(?:\{([^}]*)\})?\s+([-+eE0-9.]+|NaN)/);
    if (!match) continue;
    const [, name, rawLabels, rawValue] = match;
    if (!name.includes('resource_usage')) continue;
    const labels = {};
    if (rawLabels) {
      for (const pair of rawLabels.split(',')) {
        const equals = pair.indexOf('=');
        if (equals === -1) continue;
        labels[pair.slice(0, equals).trim()] = pair
          .slice(equals + 1)
          .trim()
          .replace(/^"(.*)"$/, '$1');
      }
    }
    const sample = normalizeSample({ metric: labels, value: rawValue });
    if (sample) samples.push(sample);
  }
  return samples;
}

// Accepts the Prometheus HTTP API vector response, a bare `result` array, a
// plain array of samples, or the raw `/metrics` text body.
export function parsePrometheusResponse(payload) {
  if (payload == null) return [];
  if (typeof payload === 'string') return parsePrometheusText(payload);
  if (Array.isArray(payload)) return payload.map(normalizeSample).filter(Boolean);
  const result = payload?.data?.result ?? payload?.result;
  if (Array.isArray(result)) return result.map(normalizeSample).filter(Boolean);
  return [];
}

export function createHeatmapState() {
  return { nodes: new Map(), lastSampleAt: 0, lastError: null };
}

export function markError(state, error, now = Date.now()) {
  const message = error && error.message ? String(error.message) : String(error ?? 'unknown error');
  state.lastError = { message, at: now };
  return state;
}

export function ingestSamples(state, samples, now = Date.now(), staleAfterMs = DEFAULT_STALE_MS) {
  const list = Array.isArray(samples) ? samples : [];
  const perNode = new Map();

  for (const raw of list) {
    const sample = raw && raw.node && raw.resource ? raw : normalizeSample(raw);
    if (!sample || !sample.node) continue;
    let entry = perNode.get(sample.node);
    if (!entry) {
      entry = { zone: sample.zone || 'unknown', pods: new Map() };
      perNode.set(sample.node, entry);
    }
    if ((!entry.zone || entry.zone === 'unknown') && sample.zone) entry.zone = sample.zone;
    const pod = entry.pods.get(sample.pod) ?? { cpu: 0, memory: 0 };
    const ratio = normalizeRatio(sample.value);
    if (sample.resource === 'memory') pod.memory = Math.max(pod.memory, ratio);
    else pod.cpu = Math.max(pod.cpu, ratio);
    entry.pods.set(sample.pod, pod);
  }

  for (const [id, entry] of perNode) {
    const existing = state.nodes.get(id);
    let cpu = 0;
    let memory = 0;
    for (const pod of entry.pods.values()) {
      cpu = Math.max(cpu, pod.cpu);
      memory = Math.max(memory, pod.memory);
    }
    state.nodes.set(id, {
      id,
      zone: entry.zone || existing?.zone || 'unknown',
      cpu,
      memory,
      podCount: entry.pods.size,
      pods: [...entry.pods.entries()].map(([name, usage]) => ({ name, cpu: usage.cpu, memory: usage.memory })),
      firstSeen: existing?.firstSeen ?? now,
      lastSeen: now,
      missingSince: null,
    });
  }

  // A worker node that dropped out of the scrape is flagged, then evicted once
  // it has been gone longer than the stale window.
  for (const [id, node] of state.nodes) {
    if (perNode.has(id)) continue;
    if (node.missingSince == null) node.missingSince = now;
    if (now - node.missingSince > staleAfterMs) state.nodes.delete(id);
  }

  state.lastSampleAt = now;
  state.lastError = null;
  return state;
}

export function materializeHeatmap(state, options = {}) {
  const thresholds = options.thresholds ?? DEFAULT_THRESHOLDS;
  const now = options.now ?? Date.now();
  const staleAfterMs = options.staleAfterMs ?? DEFAULT_STALE_MS;

  const cells = [];
  for (const node of state.nodes.values()) {
    const cpu = normalizeRatio(node.cpu);
    const memory = normalizeRatio(node.memory);
    const saturation = Math.max(cpu, memory);
    let cellState = 'active';
    if (node.missingSince != null) cellState = 'draining';
    else if (now - node.lastSeen > staleAfterMs) cellState = 'stale';
    cells.push({
      id: node.id,
      zone: node.zone || 'unknown',
      cpu,
      memory,
      saturation,
      podCount: node.podCount ?? 0,
      level: classifySaturation(saturation, thresholds),
      color: saturationColor(saturation),
      state: cellState,
      lastSeen: node.lastSeen,
    });
  }

  cells.sort((a, b) => {
    if (a.zone !== b.zone) return a.zone < b.zone ? -1 : 1;
    if (a.saturation !== b.saturation) return b.saturation - a.saturation;
    return a.id < b.id ? -1 : 1;
  });

  const zoneMap = new Map();
  for (const cell of cells) {
    let bucket = zoneMap.get(cell.zone);
    if (!bucket) {
      bucket = { zone: cell.zone, cells: [], peak: 0, total: 0 };
      zoneMap.set(cell.zone, bucket);
    }
    bucket.cells.push(cell);
    bucket.peak = Math.max(bucket.peak, cell.saturation);
    bucket.total += cell.saturation;
  }

  const zones = [...zoneMap.values()].map((bucket) => ({
    zone: bucket.zone,
    cells: bucket.cells,
    peak: bucket.peak,
    mean: bucket.cells.length ? bucket.total / bucket.cells.length : 0,
  }));

  const byLevel = { idle: 0, cool: 0, warm: 0, hot: 0, critical: 0 };
  let total = 0;
  let hottest = null;
  for (const cell of cells) {
    byLevel[cell.level] += 1;
    total += cell.saturation;
    if (!hottest || cell.saturation > hottest.saturation) hottest = cell;
  }

  return {
    cells,
    zones,
    summary: {
      nodeCount: cells.length,
      meanSaturation: cells.length ? total / cells.length : 0,
      hottest: hottest ? { id: hottest.id, zone: hottest.zone, saturation: hottest.saturation } : null,
      byLevel,
      generatedAt: now,
      lastSampleAt: state.lastSampleAt,
      lastError: state.lastError,
    },
  };
}
