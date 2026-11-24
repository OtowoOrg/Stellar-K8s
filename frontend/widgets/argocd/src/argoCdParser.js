/**
 * argoCdParser.js
 *
 * Pure functions for parsing ArgoCD Application resource trees.
 * Identifies resources in a Terminating state caused by Kubernetes
 * Finalizers that may block a Stellar-K8s StellarNode lifecycle.
 *
 * No external dependencies — fully unit-testable without a browser.
 */

// ── Stellar-K8s specific Finalizers ────────────────────────────────────────
export const STELLAR_FINALIZERS = [
  'stellarnode.k8s.stellar.org/pv-cleanup',
  'stellarnode.k8s.stellar.org/peer-deregister',
  'stellarnode.k8s.stellar.org/config-sync',
  'stellarnode.k8s.stellar.org/network-drain',
  'storage.kubernetes.io/pv-protection',
  'kubernetes.io/pvc-protection',
];

/** @typedef {'Pod'|'PersistentVolumeClaim'|'PersistentVolume'|'StellarNode'|string} ResourceKind */

/**
 * @typedef {Object} ArgoResource
 * @property {string} kind
 * @property {string} name
 * @property {string} namespace
 * @property {string} [group]
 * @property {string} [version]
 * @property {string} [status]          — health status from ArgoCD
 * @property {string} [syncStatus]      — Synced | OutOfSync | Unknown
 * @property {boolean} [requiresPruning]
 * @property {string[]} [finalizers]    — injected from live manifest
 * @property {string} [deletionTimestamp]
 * @property {string} [phase]           — StellarNode lifecycle phase
 * @property {ArgoResource[]} [children]
 */

/**
 * @typedef {Object} TerminatingResource
 * @property {string} kind
 * @property {string} name
 * @property {string} namespace
 * @property {string[]} finalizers       — remaining (blocking) Finalizers
 * @property {string[]} stellarFinalizers — subset matching STELLAR_FINALIZERS
 * @property {string} deletionTimestamp
 * @property {string} phase
 * @property {'Pod'|'PVC'|'PV'|'StellarNode'|'Unknown'} resourceCategory
 * @property {string} resolutionHint     — human-readable fix suggestion
 */

/**
 * @typedef {Object} ParsedAppState
 * @property {string} appName
 * @property {string} syncStatus        — Synced | OutOfSync | Unknown
 * @property {string} healthStatus      — Healthy | Degraded | Progressing | Missing | Unknown
 * @property {TerminatingResource[]} terminatingResources
 * @property {boolean} isStuck          — true if any blocking Finalizers found
 * @property {number} totalResources
 * @property {number} syncedCount
 * @property {number} outOfSyncCount
 */

// ── Category helpers ────────────────────────────────────────────────────────

/**
 * Map a Kubernetes resource kind to a display category.
 * @param {string} kind
 * @returns {'Pod'|'PVC'|'PV'|'StellarNode'|'Unknown'}
 */
export function categorize(kind) {
  if (kind === 'Pod') return 'Pod';
  if (kind === 'PersistentVolumeClaim') return 'PVC';
  if (kind === 'PersistentVolume') return 'PV';
  if (kind === 'StellarNode') return 'StellarNode';
  return 'Unknown';
}

/**
 * Filter the finalizers list to those belonging to Stellar-K8s.
 * @param {string[]} finalizers
 * @returns {string[]}
 */
export function extractStellarFinalizers(finalizers) {
  if (!Array.isArray(finalizers)) return [];
  return finalizers.filter((f) => STELLAR_FINALIZERS.includes(f));
}

/**
 * Determine whether a resource is stuck in Terminating.
 * A resource is considered stuck when it has a deletionTimestamp AND
 * at least one Finalizer remaining.
 * @param {ArgoResource} resource
 * @returns {boolean}
 */
export function isTerminating(resource) {
  return (
    Boolean(resource.deletionTimestamp) &&
    Array.isArray(resource.finalizers) &&
    resource.finalizers.length > 0
  );
}

// ── Resolution hints ────────────────────────────────────────────────────────

/**
 * Generate a contextual resolution hint for a terminating resource.
 * @param {ArgoResource} resource
 * @param {string[]} stellarFinalizers
 * @returns {string}
 */
export function buildResolutionHint(resource, stellarFinalizers) {
  const { kind, name, namespace, phase } = resource;

  if (kind === 'PersistentVolumeClaim') {
    return (
      `PVC "${name}" in "${namespace}" is protected. ` +
      `Ensure all Pods consuming this PVC are fully terminated, ` +
      `then the "kubernetes.io/pvc-protection" finalizer will be removed automatically. ` +
      `If stuck, run: kubectl patch pvc ${name} -n ${namespace} -p '{"metadata":{"finalizers":null}}' --type=merge`
    );
  }

  if (kind === 'PersistentVolume') {
    return (
      `PV "${name}" is in Terminating. Check that its bound PVC has been deleted first. ` +
      `Run: kubectl patch pv ${name} -p '{"metadata":{"finalizers":null}}' --type=merge`
    );
  }

  if (kind === 'Pod') {
    return (
      `Pod "${name}" in "${namespace}" is terminating with Finalizers. ` +
      `This usually indicates the Stellar operator has not completed its peer-deregister hook. ` +
      `Check the operator logs: kubectl logs -n ${namespace} -l app=stellar-operator. ` +
      `If the operator is healthy, force-delete: kubectl delete pod ${name} -n ${namespace} --grace-period=0 --force`
    );
  }

  if (kind === 'StellarNode') {
    const phaseHint =
      phase === 'Draining'
        ? 'The node is in Draining phase — network-drain finalizer must complete first.'
        : phase === 'Deregistering'
        ? 'The node is deregistering from the quorum — wait for peer-deregister to finish.'
        : 'Check the StellarNode controller logs for lifecycle errors.';
    return (
      `StellarNode "${name}" is stuck in Terminating. ${phaseHint} ` +
      `Stellar finalizers present: [${stellarFinalizers.join(', ')}]. ` +
      `Run: kubectl describe stellarnode ${name} -n ${namespace}`
    );
  }

  return (
    `Resource "${kind}/${name}" in "${namespace}" has blocking finalizers: ` +
    `[${(resource.finalizers ?? []).join(', ')}]. ` +
    `Manually patch to remove: kubectl patch ${kind.toLowerCase()} ${name} -n ${namespace} ` +
    `-p '{"metadata":{"finalizers":null}}' --type=merge`
  );
}

// ── Flat resource extractor ─────────────────────────────────────────────────

/**
 * Recursively flatten an ArgoCD resource tree into a flat array.
 * @param {ArgoResource[]} nodes
 * @returns {ArgoResource[]}
 */
export function flattenResourceTree(nodes) {
  const result = [];
  const stack = [...(nodes ?? [])];
  while (stack.length > 0) {
    const node = stack.pop();
    result.push(node);
    if (Array.isArray(node.children)) {
      stack.push(...node.children);
    }
  }
  return result;
}

// ── Main parse entry-point ──────────────────────────────────────────────────

/**
 * Parse an ArgoCD Application API response into a structured summary.
 *
 * Handles both the full Application object returned by
 * `GET /api/v1/applications/{name}` and the condensed resource summary
 * from `GET /api/v1/applications/{name}/resource-tree`.
 *
 * @param {object} appResponse — raw ArgoCD API JSON response
 * @returns {ParsedAppState}
 */
export function parseAppState(appResponse) {
  const appName = appResponse?.metadata?.name ?? appResponse?.name ?? 'unknown';
  const syncStatus = appResponse?.status?.sync?.status ?? appResponse?.syncStatus ?? 'Unknown';
  const healthStatus = appResponse?.status?.health?.status ?? appResponse?.healthStatus ?? 'Unknown';

  // Support both /applications/{name} (has status.resources) and
  // /applications/{name}/resource-tree (has nodes at top level)
  const rawResources =
    appResponse?.status?.resources ??
    appResponse?.nodes ??
    appResponse?.items ??
    [];

  const flat = flattenResourceTree(rawResources);
  const totalResources = flat.length;
  const syncedCount = flat.filter((r) => r.syncStatus === 'Synced').length;
  const outOfSyncCount = flat.filter((r) => r.syncStatus === 'OutOfSync').length;

  const terminatingResources = flat
    .filter(isTerminating)
    .map((resource) => {
      const finalizers = resource.finalizers ?? [];
      const stellarFinalizers = extractStellarFinalizers(finalizers);
      return /** @type {TerminatingResource} */ ({
        kind: resource.kind,
        name: resource.name,
        namespace: resource.namespace ?? '',
        finalizers,
        stellarFinalizers,
        deletionTimestamp: resource.deletionTimestamp,
        phase: resource.phase ?? '',
        resourceCategory: categorize(resource.kind),
        resolutionHint: buildResolutionHint(resource, stellarFinalizers),
      });
    });

  return {
    appName,
    syncStatus,
    healthStatus,
    terminatingResources,
    isStuck: terminatingResources.length > 0,
    totalResources,
    syncedCount,
    outOfSyncCount,
  };
}

// ── Polling helpers ─────────────────────────────────────────────────────────

/**
 * Lightweight ArgoCD API client with efficient polling.
 * Reuses a single AbortController per poll cycle.
 */
export class ArgoCdPoller {
  /**
   * @param {object} opts
   * @param {string} opts.baseUrl          — ArgoCD API base URL (no trailing slash)
   * @param {string} [opts.token]          — Bearer token for ArgoCD API
   * @param {number} [opts.intervalMs]     — polling interval in ms (default 10 000)
   * @param {(apps: ParsedAppState[]) => void} opts.onUpdate — callback fired on each poll
   * @param {(error: Error) => void} [opts.onError]
   */
  constructor({ baseUrl, token, intervalMs = 10_000, onUpdate, onError }) {
    this._baseUrl = baseUrl.replace(/\/$/, '');
    this._token = token ?? '';
    this._intervalMs = intervalMs;
    this._onUpdate = onUpdate;
    this._onError = onError ?? (() => {});
    this._timerId = null;
    this._abortController = null;
  }

  _headers() {
    const h = { 'Content-Type': 'application/json' };
    if (this._token) h['Authorization'] = `Bearer ${this._token}`;
    return h;
  }

  async _fetchAll() {
    this._abortController = new AbortController();
    const signal = this._abortController.signal;

    const listRes = await fetch(`${this._baseUrl}/api/v1/applications`, {
      headers: this._headers(),
      signal,
    });
    if (!listRes.ok) throw new Error(`ArgoCD API error: ${listRes.status} ${listRes.statusText}`);
    const listJson = await listRes.json();

    // items may be absent when there are no apps
    const apps = listJson?.items ?? [];
    return apps.map(parseAppState);
  }

  async _poll() {
    try {
      const states = await this._fetchAll();
      this._onUpdate(states);
    } catch (err) {
      if (err?.name !== 'AbortError') this._onError(err);
    }
  }

  /** Start polling immediately, then at the configured interval. */
  start() {
    this._poll();
    this._timerId = setInterval(() => this._poll(), this._intervalMs);
  }

  /** Stop polling and cancel any in-flight request. */
  stop() {
    clearInterval(this._timerId);
    this._timerId = null;
    this._abortController?.abort();
  }
}
