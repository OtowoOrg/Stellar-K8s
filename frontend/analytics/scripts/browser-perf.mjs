#!/usr/bin/env node
// Browser profiling harness for the WebGL quorum matrix (#86 review evidence).
//
// Serves the production bundle, then drives a real headless Chromium through
// two phases while recording requestAnimationFrame frame deltas, long tasks,
// JS heap, per-second render timing, and (optionally) a WebM screencast of the
// interactive session:
//   1. full matrix  — 120 validators / 10,000 interconnect cells (issue target)
//   2. small matrix — 24 validators / ~528 cells, validating the 60 fps
//      pipeline independent of rasterizer throughput
//
// The container this evidence was captured in has no GPU, so Chromium renders
// WebGL through SwiftShader (CPU rasterization). The report records the actual
// renderer string so numbers are never mistaken for hardware-GPU results.
//
// Usage (from frontend/analytics):
//   npm run build && npm run matrix:browser:perf
// Options:
//   --video          also save an interactive navigation screencast (WebM)
//   --sweep <s>      hover sweep duration for the full phase (default 10)

import { spawn } from 'node:child_process';
import { mkdir, readdir, rename, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium } from 'playwright-core';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, '..');
const resultsDir = path.join(root, 'results');
const saveVideo = process.argv.includes('--video');
const fullSweepSeconds = (() => {
  const index = process.argv.indexOf('--sweep');
  return index >= 0 ? Number(process.argv[index + 1]) || 10 : 10;
})();
const smallSweepSeconds = Math.max(2, Math.round(fullSweepSeconds / 2));

const PORT = 4173;

async function waitForServer(url, timeoutMs = 20000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch { /* not up yet */ }
    await new Promise((resolve) => setTimeout(resolve, 150));
  }
  throw new Error(`dev server did not become ready at ${url}`);
}

// Collects requestAnimationFrame deltas in-page until told to stop.
function startFrameRecorder(page) {
  return page.evaluate(() => {
    window.__sweepDeltas = [];
    window.__sweepRecording = true;
    let last = performance.now();
    const tick = (now) => {
      if (!window.__sweepRecording) return;
      window.__sweepDeltas.push(now - last);
      last = now;
      requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

function summarize(deltas) {
  if (!deltas.length) return { samples: 0 };
  const sorted = [...deltas].sort((a, b) => a - b);
  const p = (q) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * q))];
  const avg = deltas.reduce((sum, value) => sum + value, 0) / deltas.length;
  return {
    avgFps: Math.round(1000 / avg),
    p50FrameMs: Number(p(0.5).toFixed(2)),
    p95FrameMs: Number(p(0.95).toFixed(2)),
    p99FrameMs: Number(p(0.99).toFixed(2)),
    maxFrameMs: Number(sorted[sorted.length - 1].toFixed(2)),
    samples: deltas.length,
  };
}

// One measurement pass: idle deltas, then a continuous hover sweep with frame
// deltas accumulating in-page, plus the per-second render timing hook.
async function measurePhase(page, { nodes, edges, sweepSeconds }) {
  await page.evaluate(() => {
    window.__longTasks = 0;
    window.__longTaskMs = 0;
  });
  await page.goto(`http://127.0.0.1:${PORT}/perf.html?nodes=${nodes}&edges=${edges}`, { waitUntil: 'networkidle' });
  const cellCount = await page.evaluate(() => {
    const heading = document.querySelector('.brand-block h1');
    return Number((heading?.textContent.match(/([\d,]+) cells/) ?? [])[1]?.replaceAll(',', '')) || 0;
  });
  if (cellCount < 1) throw new Error(`perf harness rendered no cells (nodes=${nodes})`);

  await page.waitForTimeout(1000);
  const idleFrames = await page.evaluate(() => new Promise((resolve) => {
    const deltas = [];
    let last = performance.now();
    let count = 0;
    const tick = (now) => {
      deltas.push(now - last);
      last = now;
      count += 1;
      if (count < 120) requestAnimationFrame(tick);
      else resolve(deltas);
    };
    requestAnimationFrame(tick);
  }));

  const canvas = page.locator('.matrix-host canvas');
  const box = await canvas.boundingBox();
  if (!box) throw new Error('matrix canvas not visible');
  await startFrameRecorder(page);
  const sweepStart = Date.now();
  let pass = 0;
  while (Date.now() - sweepStart < sweepSeconds * 1000) {
    const y = pass % 2 === 0 ? box.y + box.height / 2 : box.y + (box.height * 2) / 3;
    for (let step = 0; step <= 40; step += 1) {
      await page.mouse.move(box.x + (box.width * step) / 40, y, { steps: 2 });
      await page.waitForTimeout(16);
    }
    for (let step = 0; step <= 40; step += 1) {
      await page.mouse.move(box.x + box.width - (box.width * step) / 40, y, { steps: 2 });
      await page.waitForTimeout(16);
    }
    pass += 1;
  }
  await page.evaluate(() => { window.__sweepRecording = false; });
  const sweepFrames = await page.evaluate(() => window.__sweepDeltas);
  const timing = await page.evaluate(() => window.__matrixTiming ?? null);
  const longTasks = await page.evaluate(() => ({ count: window.__longTasks, ms: Math.round(window.__longTaskMs) }));

  return {
    dataset: { validators: nodes, interconnectCells: cellCount },
    idle: summarize(idleFrames),
    hoverSweep: summarize(sweepFrames),
    renderTiming: timing,
    longTasks,
  };
}

const server = spawn(
  process.execPath,
  [path.join(root, 'node_modules/vite/bin/vite.js'), 'preview', '--host', '127.0.0.1', '--port', String(PORT), '--strictPort'],
  { cwd: root, stdio: 'ignore' },
);

let videoPath = null;
try {
  const baseUrl = `http://127.0.0.1:${PORT}`;
  await waitForServer(`${baseUrl}/perf.html`);

  await mkdir(resultsDir, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 1,
    recordVideo: saveVideo ? { dir: resultsDir, size: { width: 1280, height: 800 } } : undefined,
  });
  const page = await context.newPage();
  await page.addInitScript(() => {
    window.__longTasks = 0;
    window.__longTaskMs = 0;
    new PerformanceObserver((list) => {
      for (const entry of list.getEntries()) {
        if (entry.duration > 50) {
          window.__longTasks += 1;
          window.__longTaskMs += entry.duration;
        }
      }
    }).observe({ entryTypes: ['longtask'] });
  });

  const full = await measurePhase(page, { nodes: 120, edges: 10000, sweepSeconds: fullSweepSeconds });
  const small = await measurePhase(page, { nodes: 24, edges: 528, sweepSeconds: smallSweepSeconds });

  const memory = await page.evaluate(() => performance.memory ? {
    usedJsHeapMB: Math.round(performance.memory.usedJSHeapSize / 1048576),
    jsHeapLimitMB: Math.round(performance.memory.jsHeapSizeLimit / 1048576),
  } : null);

  const glInfo = await page.evaluate(() => {
    const gl = document.querySelector('.matrix-host canvas').getContext('webgl2')
      || document.querySelector('.matrix-host canvas').getContext('webgl');
    if (!gl) return null;
    const ext = gl.getExtension('WEBGL_debug_renderer_info');
    return {
      renderer: ext ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER),
      vendor: ext ? gl.getParameter(ext.UNMASKED_VENDOR_WEBGL) : gl.getParameter(gl.VENDOR),
    };
  });

  const report = {
    generatedAt: new Date().toISOString(),
    environment: {
      browser: 'Chromium 151 (playwright-core headless shell)',
      viewport: '1280x800',
      deviceScaleFactor: 1,
      webgl: glInfo,
      note: 'GPU-less CI container: WebGL rasterization runs on SwiftShader (CPU).',
    },
    full,
    small,
    memory,
  };

  const reportPath = path.join(resultsDir, 'matrix-browser-perf.json');
  await writeFile(reportPath, `${JSON.stringify(report, null, 2)}\n`);

  if (saveVideo) {
    // Closing the context flushes the recording to disk.
    await context.close();
    const videos = (await readdir(resultsDir)).filter((name) => name.endsWith('.webm'));
    if (videos.length) {
      videoPath = path.join(resultsDir, 'matrix-10k-navigation.webm');
      await rename(path.join(resultsDir, videos.at(-1)), videoPath);
    }
  }

  const line = (label, phase) => `${label.padEnd(22)} ${phase.idle.avgFps} fps idle, ${phase.hoverSweep.avgFps} fps sweep, `
    + `p95 ${phase.hoverSweep.p95FrameMs} ms, render JS ${phase.renderTiming?.avgRenderJsMs?.toFixed(2) ?? 'n/a'} ms/frame`;
  console.log('Browser profiling complete');
  console.log(`  cells:                ${full.dataset.interconnectCells} full / ${small.dataset.interconnectCells} small`);
  if (glInfo) console.log(`  webgl renderer:       ${glInfo.renderer}`);
  console.log(`  ${line('full matrix:', full)}`);
  console.log(`  ${line('small matrix:', small)}`);
  console.log(`  long tasks > 50 ms:   full ${full.longTasks.count}, small ${small.longTasks.count}`);
  if (memory) console.log(`  JS heap:              ${memory.usedJsHeapMB} MB used / ${memory.jsHeapLimitMB} MB limit`);
  console.log(`  report:               ${path.relative(root, reportPath)}`);
  if (videoPath) console.log(`  screencast:           ${path.relative(root, videoPath)}`);

  await browser.close();
} finally {
  server.kill('SIGTERM');
}
