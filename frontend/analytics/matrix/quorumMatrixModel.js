export const MAX_MATRIX_NODES = 5000;
export const MAX_MATRIX_CELLS = 250000;

const PHASES = new Set(['PREPARE', 'CONFIRM', 'EXTERNALIZE', 'UNKNOWN']);

function asNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function clamp01(value) {
  return Math.min(1, Math.max(0, value));
}

function nodeId(value) {
  return String(value?.id ?? value?.node_id ?? value?.nodeId ?? value?.full_id ?? value?.fullId ?? value ?? '');
}

function shortId(id = '') {
  if (id.length <= 12) return id;
  return `${id.slice(0, 4)}…${id.slice(-4)}`;
}

function metric(source, names, fallback = 0) {
  for (const name of names) {
    if (source?.[name] !== undefined) return asNumber(source[name], fallback);
    if (source?.metrics?.[name] !== undefined) return asNumber(source.metrics[name], fallback);
    if (source?.metadata?.[name] !== undefined) return asNumber(source.metadata[name], fallback);
  }
  return fallback;
}

function normalizeMatrixNode(node = {}, index = 0) {
  const fullId = String(node.full_id ?? node.fullId ?? node.node_id ?? node.nodeId ?? node.id ?? `node-${index}`);
  const phase = String(node.phase ?? 'UNKNOWN').toUpperCase();
  const tps = asNumber(metric(node, ['tps', 'peak_tps', 'transactions_per_second'], 0));
  const ledgerTimeMs = asNumber(metric(node, ['ledger_time_ms', 'ledgerTimeMs', 'ledger_close_time_ms'], 0));
  return {
    id: String(node.id ?? shortId(fullId)),
    fullId,
    name: String(node.node_name ?? node.nodeName ?? shortId(fullId)),
    cluster: String(node.cluster ?? node.namespace ?? 'default'),
    phase: PHASES.has(phase) ? phase : 'UNKNOWN',
    stalled: Boolean(node.stalled ?? node.is_stalled ?? node.isStalled),
    critical: Boolean(node.is_critical ?? node.isCritical),
    threshold: asNumber(node.threshold ?? node.quorum_set?.threshold ?? node.quorumSet?.t, 0),
    tps,
    ledgerTimeMs,
    health: String(node.health ?? 'unknown').toLowerCase(),
    publicKey: String(node.public_key ?? node.publicKey ?? fullId),
    order: index,
  };
}

export function agreementForPair(source, target) {
  if (!source || !target) return 'unknown';
  if (source.stalled || target.stalled) return 'diverged';
  if (source.phase === 'UNKNOWN' || target.phase === 'UNKNOWN') return 'unknown';
  if (source.phase === 'EXTERNALIZE' && target.phase === 'EXTERNALIZE') return 'agreeing';
  if (source.phase === 'EXTERNALIZE' || target.phase === 'EXTERNALIZE') return 'lagging';
  return 'confirming';
}

export function trustWeight(source, target) {
  if (!source || !target) return 0;
  let weight = 0.5;
  if (source.phase === 'EXTERNALIZE') weight += 0.2;
  if (target.phase === 'EXTERNALIZE') weight += 0.2;
  if (source.critical) weight += 0.05;
  if (target.critical) weight += 0.05;
  if (source.stalled || target.stalled) weight = 0.1;
  return Math.round(clamp01(weight) * 1000) / 1000;
}

export function latencyMs(source, target) {
  if (!source || !target) return 0;
  const left = source.ledgerTimeMs || 0;
  const right = target.ledgerTimeMs || 0;
  return Math.abs(left - right) + Math.min(left, right) * 0.5;
}

export function buildQuorumMatrix(snapshot = {}) {
  const rawNodes = Array.isArray(snapshot.nodes) ? snapshot.nodes : [];
  const nodes = rawNodes.slice(0, MAX_MATRIX_NODES).map(normalizeMatrixNode);
  const indexById = new Map(nodes.map((node, index) => [node.id, index]));

  const cells = [];
  const seen = new Set();
  const rawEdges = Array.isArray(snapshot.edges) ? snapshot.edges : [];
  for (const edge of rawEdges) {
    if (cells.length >= MAX_MATRIX_CELLS) break;
    const sourceIndex = indexById.get(nodeId(edge.source));
    const targetIndex = indexById.get(nodeId(edge.target));
    if (sourceIndex === undefined || targetIndex === undefined || sourceIndex === targetIndex) continue;
    const key = sourceIndex * nodes.length + targetIndex;
    if (seen.has(key)) continue;
    seen.add(key);
    cells.push(buildCell(nodes, sourceIndex, targetIndex));
  }

  return {
    nodes,
    cells,
    size: nodes.length,
    timestamp: snapshot.timestamp ?? new Date().toISOString(),
    healthy: snapshot.healthy !== false,
  };
}

function buildCell(nodes, sourceIndex, targetIndex) {
  const source = nodes[sourceIndex];
  const target = nodes[targetIndex];
  return {
    sourceIndex,
    targetIndex,
    agreement: agreementForPair(source, target),
    trust: trustWeight(source, target),
    latencyMs: latencyMs(source, target),
  };
}

export function cellColor(cell) {
  switch (cell?.agreement) {
    case 'agreeing': return [0.22, 0.85, 0.54];
    case 'confirming': return [0.96, 0.73, 0.26];
    case 'lagging': return [0.35, 0.62, 0.95];
    case 'diverged': return [0.94, 0.36, 0.37];
    default: return [0.42, 0.47, 0.55];
  }
}

export function matrixStats(matrix) {
  const counts = { agreeing: 0, confirming: 0, lagging: 0, diverged: 0, unknown: 0 };
  let trustSum = 0;
  let latencySum = 0;
  for (const cell of matrix.cells) {
    counts[cell.agreement] = (counts[cell.agreement] ?? 0) + 1;
    trustSum += cell.trust;
    latencySum += cell.latencyMs;
  }
  const cellCount = matrix.cells.length || 1;
  return {
    counts,
    cells: matrix.cells.length,
    avgTrust: trustSum / cellCount,
    avgLatencyMs: latencySum / cellCount,
  };
}

// Lookup maps are cached per matrix instance so hover picking stays O(1) even
// for 10,000+ cell matrices; stale entries are released with the matrix.
const lookupCache = new WeakMap();

function lookupFor(matrix) {
  let lookup = lookupCache.get(matrix);
  if (!lookup) {
    lookup = new Map();
    for (const cell of matrix.cells) lookup.set(cell.sourceIndex * matrix.size + cell.targetIndex, cell);
    lookupCache.set(matrix, lookup);
  }
  return lookup;
}

export function cellAt(matrix, sourceIndex, targetIndex) {
  if (!matrix || sourceIndex < 0 || targetIndex < 0 || sourceIndex >= matrix.size || targetIndex >= matrix.size) return null;
  return lookupFor(matrix).get(sourceIndex * matrix.size + targetIndex) ?? null;
}

export function cellForPosition(matrix, sourceIndex, targetIndex) {
  return cellAt(matrix, sourceIndex, targetIndex);
}

// Latency deltas above the floor start dimming a cell; at the ceiling the cell
// reaches its minimum opacity. Trust weights scale brightness linearly so a
// low-trust link reads dimmer at the same agreement status.
export const SHADE_LATENCY_FLOOR_MS = 4;
export const SHADE_LATENCY_CEILING_MS = 16;
const SHADE_MAX_OPACITY = 0.95;
const SHADE_MIN_OPACITY = 0.4;
const SHADE_MIN_BRIGHTNESS = 0.55;

export function cellShade(cell) {
  const base = cellColor(cell);
  const trust = clamp01(asNumber(cell?.trust, 0));
  const brightness = SHADE_MIN_BRIGHTNESS + (1 - SHADE_MIN_BRIGHTNESS) * trust;
  const latency = Math.max(0, asNumber(cell?.latencyMs, 0));
  const span = SHADE_LATENCY_CEILING_MS - SHADE_LATENCY_FLOOR_MS;
  const latencyPenalty = clamp01((latency - SHADE_LATENCY_FLOOR_MS) / span);
  return {
    color: base.map((channel) => Math.round(channel * brightness * 1000) / 1000),
    opacity: SHADE_MAX_OPACITY - (SHADE_MAX_OPACITY - SHADE_MIN_OPACITY) * latencyPenalty,
  };
}

export function emptyMatrix() {
  return { nodes: [], cells: [], size: 0, timestamp: null, healthy: true };
}
