/**
 * usePrometheusPoller.js
 *
 * React hook that polls the Prometheus HTTP API for
 * `stellar_operator_resource_usage` metrics at a configurable interval,
 * and returns a live Map of normalized NodeMetric records.
 *
 * The poll runs in the background via setInterval.  React state is updated
 * only once per interval tick using a single setState call (never concurrent
 * partial updates), so re-renders are bounded to one per poll cycle.
 *
 * On each poll the previous Map is passed to `applyPrometheusResponse` so
 * tombstoned (disappeared) nodes are preserved for one extra cycle and
 * rendered with the `missing` flag.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import { applyPrometheusResponse, materializeNodes, POLL_INTERVAL_MS } from '../../heatmapModel.js';

/**
 * @typedef {'idle'|'polling'|'error'|'offline'} PollerStatus
 */

/**
 * @param {object}  options
 * @param {string}  [options.prometheusUrl='/api/prometheus']
 *   Base URL of the Prometheus HTTP API.  The hook will query
 *   `{prometheusUrl}/api/v1/query` with the resource usage PromQL expression.
 * @param {number}  [options.intervalMs]  Override the default 5-second interval.
 * @param {boolean} [options.paused]       Suspend polling while true.
 * @param {string}  [options.query]
 *   Override the default PromQL query.  Must return samples whose labels
 *   include a `resource` label set to either "cpu" or "memory", with
 *   values in the range [0, 1] (ratio, not percent).
 *
 * @returns {{ nodes: import('../../heatmapModel.js').NodeMetric[], status: PollerStatus, lastPollAt: Date|null, error: string|null }}
 */
export function usePrometheusPoller({
  prometheusUrl = '/api/prometheus',
  intervalMs = POLL_INTERVAL_MS,
  paused = false,
  query = 'stellar_operator_resource_usage',
} = {}) {
  const [nodes, setNodes] = useState(/** @type {import('../../heatmapModel.js').NodeMetric[]} */ ([]));
  const [status, setStatus] = useState(/** @type {PollerStatus} */ ('idle'));
  const [lastPollAt, setLastPollAt] = useState(/** @type {Date|null} */ (null));
  const [error, setError] = useState(/** @type {string|null} */ (null));

  // Keep a ref to the current node Map so we can pass it to applyPrometheusResponse
  // without it being a dependency of the polling effect.
  const nodeMapRef = useRef(/** @type {Map<string, import('../../heatmapModel.js').NodeMetric>} */ (new Map()));
  const pausedRef = useRef(paused);
  useEffect(() => { pausedRef.current = paused; }, [paused]);

  const poll = useCallback(async () => {
    if (pausedRef.current) return;
    setStatus('polling');
    try {
      const url = `${prometheusUrl}/api/v1/query?query=${encodeURIComponent(query)}`;
      const response = await fetch(url, { signal: AbortSignal.timeout(4_000) });
      if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const body = await response.json();
      const now = Date.now();
      nodeMapRef.current = applyPrometheusResponse(body, nodeMapRef.current, now);
      setNodes(materializeNodes(nodeMapRef.current));
      setLastPollAt(new Date(now));
      setStatus('idle');
      setError(null);
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      setStatus(message === 'Failed to fetch' ? 'offline' : 'error');
      setError(message);
    }
  }, [prometheusUrl, query]);

  useEffect(() => {
    // Kick off an immediate first poll then schedule subsequent ones.
    poll();
    const timer = setInterval(poll, intervalMs);
    return () => clearInterval(timer);
  }, [poll, intervalMs]);

  return { nodes, status, lastPollAt, error };
}
