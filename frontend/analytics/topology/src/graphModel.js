export const MAX_NODES = 5000;
export const MAX_EDGES = 20000;

const PHASES = new Set(['PREPARE', 'CONFIRM', 'EXTERNALIZE', 'UNKNOWN']);

function asNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function shortId(id = '') {
  if (id.length <= 12) return id;
  return `${id.slice(0, 4)}...${id.slice(-4)}`;
}

function identityAliases(value) {
  const id = String(value ?? '');
  return id ? [id, shortId(id)] : [];
}

function metric(source, names, fallback = 0) {
  for (const name of names) {
    if (source?.[name] !== undefined) return asNumber(source[name], fallback);
    if (source?.metrics?.[name] !== undefined) return asNumber(source.metrics[name], fallback);
    if (source?.metadata?.[name] !== undefined) return asNumber(source.metadata[name], fallback);
  }
  return fallback;
}

function ledgerTimeMs(source, fallback = 0) {
  const milliseconds = metric(
    source,
    ['ledger_time_ms', 'ledgerTimeMs', 'ledger_close_time_ms', 'avg_ledger_close_ms', 'ledger_close_ms'],
    Number.NaN,
  );
  if (Number.isFinite(milliseconds)) return milliseconds;
  const seconds = metric(source, ['ledger_time_seconds', 'ledgerTimeSeconds', 'ledger_time'], Number.NaN);
  return Number.isFinite(seconds) ? seconds * 1000 : fallback;
}

function normalizeNode(node = {}, fallback = {}) {
  const fullId = String(node.full_id ?? node.fullId ?? node.node_id ?? node.nodeId ?? node.id ?? fallback.id ?? 'unknown');
  const phase = String(node.phase ?? fallback.phase ?? 'UNKNOWN').toUpperCase();
  return {
    id: String(node.id ?? shortId(fullId)),
    fullId,
    name: String(node.node_name ?? node.nodeName ?? fallback.name ?? shortId(fullId)),
    ledgerSequence: node.ledger_sequence ?? node.ledgerSequence ?? fallback.ledgerSequence,
    cluster: String(node.cluster ?? node.namespace ?? fallback.cluster ?? 'default'),
    phase: PHASES.has(phase) ? phase : 'UNKNOWN',
    health: String(node.health ?? fallback.health ?? 'unknown').toLowerCase(),
    critical: Boolean(node.is_critical ?? node.isCritical ?? fallback.critical),
    stalled: Boolean(node.stalled ?? node.is_stalled ?? node.isStalled ?? fallback.stalled),
    threshold: asNumber(
      node.threshold
        ?? node.quorum_set?.threshold
        ?? node.quorum_set?.t
        ?? node.quorumSet?.threshold
        ?? node.quorumSet?.t
        ?? fallback.threshold,
      0,
    ),
    ballotCounter: asNumber(node.ballot_counter ?? node.ballotCounter ?? fallback.ballotCounter, 0),
    tps: metric(node, ['tps', 'peak_tps', 'transactions_per_second'], fallback.tps),
    ledgerTimeMs: ledgerTimeMs(node, fallback.ledgerTimeMs),
    lastSeen: asNumber(node.timestamp ?? fallback.lastSeen, Date.now()),
  };
}

function edgeKey(source, target) {
  return `${source}\u0000${target}`;
}

function nodeId(value) {
  return String(value?.id ?? value?.node_id ?? value?.nodeId ?? value?.full_id ?? value?.fullId ?? value ?? '');
}

function resolveIdentity(identityIndex, rawValue) {
  for (const alias of identityAliases(nodeId(rawValue))) {
    const resolved = identityIndex.get(alias);
    if (resolved) return resolved;
  }
  return undefined;
}

function buildIdentityIndex(nodes) {
  const index = new Map();
  for (const node of nodes) {
    for (const alias of [...identityAliases(node.id), ...identityAliases(node.fullId)]) {
      index.set(alias, node.id);
    }
  }
  return index;
}

function normalizeEdges(edges = [], identityIndex) {
  const result = [];
  const seen = new Set();
  for (const edge of edges) {
    const source = resolveIdentity(identityIndex, edge.source);
    const target = resolveIdentity(identityIndex, edge.target);
    if (!source || !target) continue;
    const key = edgeKey(source, target);
    if (!seen.has(key)) {
      seen.add(key);
      result.push({ source, target });
    }
    if (result.length >= MAX_EDGES) break;
  }
  return result;
}

export function normalizeSnapshot(snapshot = {}) {
  const sourceNodes = Array.isArray(snapshot.nodes) ? snapshot.nodes : [];
  const nodes = sourceNodes.slice(0, MAX_NODES).map((node) => normalizeNode(node));
  const identityIndex = buildIdentityIndex(nodes);
  return {
    nodes,
    edges: normalizeEdges(Array.isArray(snapshot.edges) ? snapshot.edges : [], identityIndex),
    timestamp: snapshot.timestamp ?? new Date().toISOString(),
    healthy: snapshot.healthy !== false,
  };
}

export function applyMessage(state, message = {}) {
  const rawId = nodeId(message);
  if (!rawId) return state;
  const existingId = resolveIdentity(state.identityIndex, rawId);
  const existing = existingId ? state.nodesById.get(existingId) : undefined;
  const node = normalizeNode(message, existing);
  const key = existing?.id ?? node.id;
  node.id = key;
  node.fullId = node.fullId || existing?.fullId || rawId;
  state.nodesById.set(key, node);
  for (const alias of [...identityAliases(key), ...identityAliases(node.fullId)]) {
    state.identityIndex.set(alias, key);
  }

  const rawQuorum = message.quorum_set ?? message.quorumSet ?? {};
  const members = [
    ...(Array.isArray(rawQuorum.validators) ? rawQuorum.validators : []),
    ...(Array.isArray(rawQuorum.v) ? rawQuorum.v : []),
    ...(Array.isArray(rawQuorum.inner_sets) ? rawQuorum.inner_sets.flatMap((set) => set.validators ?? set.v ?? []) : []),
    ...(Array.isArray(rawQuorum.innerSets) ? rawQuorum.innerSets.flatMap((set) => set.validators ?? set.v ?? []) : []),
  ];
  for (const member of members) {
    addOrPendEdge(state, node.id, nodeId(member));
  }
  resolvePendingEdges(state);

  while (state.nodesById.size > MAX_NODES) {
    const first = state.nodesById.keys().next().value;
    const removed = state.nodesById.get(first);
    state.nodesById.delete(first);
    for (const alias of [...identityAliases(removed?.id), ...identityAliases(removed?.fullId)]) {
      if (state.identityIndex.get(alias) === first) state.identityIndex.delete(alias);
    }
    state.edges = state.edges.filter((edge) => edge.source !== first && edge.target !== first);
    state.edgeKeys = new Set(state.edges.map((edge) => edgeKey(edge.source, edge.target)));
  }
  state.timestamp = message.timestamp ?? new Date().toISOString();
  state.healthy = true;
  return state;
}

export function createStreamState(snapshot = {}) {
  const normalized = normalizeSnapshot(snapshot);
  return {
    nodesById: new Map(normalized.nodes.map((node) => [node.id, node])),
    identityIndex: buildIdentityIndex(normalized.nodes),
    edges: normalized.edges,
    edgeKeys: new Set(normalized.edges.map((edge) => edgeKey(edge.source, edge.target))),
    pendingEdges: [],
    pendingEdgeKeys: new Set(),
    timestamp: normalized.timestamp,
    healthy: normalized.healthy,
  };
}

export function materialize(state) {
  return {
    nodes: [...state.nodesById.values()],
    edges: [...state.edges],
    timestamp: state.timestamp,
    healthy: state.healthy,
  };
}

function addOrPendEdge(state, sourceRaw, targetRaw) {
  const source = resolveIdentity(state.identityIndex, sourceRaw);
  const target = resolveIdentity(state.identityIndex, targetRaw);
  if (source && target) {
    const key = edgeKey(source, target);
    if (!state.edgeKeys.has(key)) {
      state.edgeKeys.add(key);
      state.edges.push({ source, target });
      if (state.edges.length > MAX_EDGES) {
        const removed = state.edges.shift();
        state.edgeKeys.delete(edgeKey(removed.source, removed.target));
      }
    }
    return;
  }
  const pendingKey = edgeKey(sourceRaw, targetRaw);
  if (sourceRaw && targetRaw && state.pendingEdges.length < MAX_EDGES && !state.pendingEdgeKeys.has(pendingKey)) {
    state.pendingEdges.push({ sourceRaw, targetRaw });
    state.pendingEdgeKeys.add(pendingKey);
  }
}

function resolvePendingEdges(state) {
  if (!state.pendingEdges.length) return;
  const pending = state.pendingEdges;
  state.pendingEdges = [];
  state.pendingEdgeKeys.clear();
  for (const edge of pending) addOrPendEdge(state, edge.sourceRaw, edge.targetRaw);
}

export function ingest(state, payload) {
  if (Array.isArray(payload?.nodes)) return createStreamState(payload);
  return applyMessage(state, payload);
}

export function statusForNode(node) {
  if (node.stalled || node.phase === 'UNKNOWN') return 'falling-behind';
  if (node.health === 'degraded' || node.phase === 'PREPARE' || node.phase === 'CONFIRM') return 'degraded';
  return 'synced';
}
