import test from 'node:test';
import assert from 'node:assert/strict';
import { renderToStaticMarkup } from 'react-dom/server';
import ResourceHeatmap, { buildHeatmapLayout } from './ResourceHeatmap.jsx';

const noopFetch = () => new Promise(() => {});

function vector(result) {
  return { status: 'success', data: { resultType: 'vector', result } };
}

function sampleCluster(nodeCount, hotNode) {
  const result = [];
  for (let index = 0; index < nodeCount; index += 1) {
    const zone = `az-${'abc'[index % 3]}`;
    const cpu = index === hotNode ? 0.96 : 0.2 + (index % 5) * 0.05;
    result.push({ metric: { node: `worker-${index}`, zone, pod: `pod-${index}`, resource: 'cpu' }, value: [1, String(cpu)] });
    result.push({ metric: { node: `worker-${index}`, zone, pod: `pod-${index}`, resource: 'memory' }, value: [1, '0.3'] });
  }
  return vector(result);
}

test('buildHeatmapLayout wraps cells into non-overlapping rows and stacks zones', () => {
  const zones = [
    { zone: 'az-a', peak: 0.9, mean: 0.5, cells: Array.from({ length: 25 }, (_, i) => ({ id: `a${i}` })) },
    { zone: 'az-b', peak: 0.4, mean: 0.3, cells: Array.from({ length: 3 }, (_, i) => ({ id: `b${i}` })) },
  ];
  const layout = buildHeatmapLayout(zones, 20);
  assert.equal(layout.zones.length, 2);
  // 25 cells at 20 columns -> two rows in the first band.
  assert.equal(layout.zones[0].cells[0].x, 0);
  assert.equal(layout.zones[0].cells[20].x, 0);
  assert.ok(layout.zones[0].cells[20].y > layout.zones[0].cells[0].y);
  // Second band starts below the first.
  assert.ok(layout.zones[1].y >= layout.zones[0].y + layout.zones[0].height);
  assert.ok(layout.height > 0 && layout.width > 0);
});

test('renders one grid cell per worker node with zone bands', () => {
  const markup = renderToStaticMarkup(
    <ResourceHeatmap initialSamples={sampleCluster(100, 7)} fetchImpl={noopFetch} pollIntervalMs={60000} />,
  );
  const cells = markup.match(/data-node="worker-\d+"/g) ?? [];
  assert.equal(cells.length, 100);
  assert.match(markup, /az-a - peak/);
  assert.match(markup, /Resource saturation for 100 worker nodes across 3 availability zones/);
});

test('color-codes a near-saturation node as critical and idle nodes as cool', () => {
  const markup = renderToStaticMarkup(
    <ResourceHeatmap initialSamples={sampleCluster(12, 3)} fetchImpl={noopFetch} pollIntervalMs={60000} />,
  );
  assert.match(markup, /data-node="worker-3"[^>]*data-level="critical"/);
  assert.doesNotMatch(markup, /data-node="worker-0"[^>]*data-level="critical"/);
});

test('shows a waiting message when no telemetry has arrived', () => {
  const markup = renderToStaticMarkup(
    <ResourceHeatmap fetchImpl={noopFetch} pollIntervalMs={60000} endpoint="/api/v1/query" />,
  );
  assert.match(markup, /Waiting for worker-node telemetry from \/api\/v1\/query/);
  assert.doesNotMatch(markup, /<rect/);
});

test('renders a cluster that has lost a worker node without gaps', () => {
  // A scrape that only reports 5 of the previous 6 nodes yields exactly 5 cells.
  const markup = renderToStaticMarkup(
    <ResourceHeatmap initialSamples={sampleCluster(5, 1)} fetchImpl={noopFetch} pollIntervalMs={60000} />,
  );
  const cells = markup.match(/data-node="worker-\d+"/g) ?? [];
  assert.equal(cells.length, 5);
  assert.doesNotMatch(markup, /data-node="worker-5"/);
});

test('never emits more grid cells than worker nodes at 100-node scale', () => {
  const markup = renderToStaticMarkup(
    <ResourceHeatmap initialSamples={sampleCluster(100, 42)} fetchImpl={noopFetch} pollIntervalMs={60000} />,
  );
  const rects = markup.match(/<rect/g) ?? [];
  assert.equal(rects.length, 100);
});
