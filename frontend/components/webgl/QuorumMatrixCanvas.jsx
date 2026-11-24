import { useEffect, useRef, useState } from 'react';
import { QuorumMatrixRenderer } from './QuorumMatrixRenderer.js';
import { cellForPosition } from '../../analytics/matrix/quorumMatrixModel.js';

function pickCell(matrix, renderer, clientX, clientY) {
  const rect = renderer.canvas.getBoundingClientRect();
  const width = rect.width || 1;
  const height = rect.height || 1;
  const stride = renderer.cellSize + renderer.gap;
  const half = (matrix.size * stride) / 2;
  const aspect = width / height;
  const ndcX = ((clientX - rect.left) / width) * 2 - 1;
  const ndcY = -((clientY - rect.top) / height) * 2 + 1;
  const worldX = ndcX * half * aspect;
  const worldY = ndcY * half;
  const column = Math.round(worldX / stride + (matrix.size - 1) / 2);
  const row = Math.round((matrix.size - 1) / 2 - worldY / stride);
  return cellForPosition(matrix, row, column);
}

export default function QuorumMatrixCanvas({ matrix, onHoverCell, onFrameTiming, cellSize = 1, gap = 0.08 }) {
  const canvasRef = useRef(null);
  const rendererRef = useRef(null);
  const matrixRef = useRef(matrix);
  const [fps, setFps] = useState(0);
  const frameTimingRef = useRef(onFrameTiming);

  useEffect(() => { frameTimingRef.current = onFrameTiming; }, [onFrameTiming]);

  useEffect(() => { matrixRef.current = matrix; }, [matrix]);

  // The renderer owns a WebGL context, so its lifecycle must live inside this
  // effect: React StrictMode double-invokes effects in development, and the
  // cleanup below guarantees the first context is released before a second
  // renderer is constructed against the same canvas.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;
    const renderer = new QuorumMatrixRenderer({ canvas, cellSize, gap });
    rendererRef.current = renderer;

    const parent = canvas.parentElement;
    const resize = () => renderer.resize(parent.clientWidth || 800, parent.clientHeight || 600);
    const observer = new ResizeObserver(resize);
    observer.observe(parent);
    resize();

    let raf = 0;
    let frames = 0;
    let renderJsMs = 0;
    let last = performance.now();
    const loop = () => {
      // Cheap when the matrix and highlight are unchanged: buffers are not
      // re-uploaded, only the same frame is presented to keep the fps meter.
      const renderStart = performance.now();
      renderer.render(matrixRef.current);
      renderJsMs += performance.now() - renderStart;
      frames += 1;
      const now = performance.now();
      if (now - last >= 1000) {
        const measuredFps = Math.round((frames * 1000) / (now - last));
        setFps(measuredFps);
        // JS-side render cost excludes browser rasterization time, so it
        // isolates main-thread load from GPU/software rasterizer throughput.
        frameTimingRef.current?.({ fps: measuredFps, avgRenderJsMs: frames ? renderJsMs / frames : 0, frames });
        frames = 0;
        renderJsMs = 0;
        last = now;
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);

    return () => {
      cancelAnimationFrame(raf);
      observer.disconnect();
      renderer.dispose();
      rendererRef.current = null;
    };
  }, [cellSize, gap]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;
    let pickFrame = 0;
    let pendingEvent = null;
    // Pointer events fire far more often than the display refreshes; coalesce
    // them into one picking pass per animation frame.
    const runPick = () => {
      pickFrame = 0;
      const rendererInstance = rendererRef.current;
      if (!rendererInstance || !pendingEvent) return;
      const cell = pickCell(matrixRef.current, rendererInstance, pendingEvent.clientX, pendingEvent.clientY);
      pendingEvent = null;
      rendererInstance.highlight = cell;
      onHoverCell?.(cell);
    };
    const onMove = (event) => {
      pendingEvent = event;
      if (!pickFrame) pickFrame = requestAnimationFrame(runPick);
    };
    const onLeave = () => {
      if (pickFrame) {
        cancelAnimationFrame(pickFrame);
        pickFrame = 0;
      }
      pendingEvent = null;
      const rendererInstance = rendererRef.current;
      if (rendererInstance) rendererInstance.highlight = null;
      onHoverCell?.(null);
    };
    canvas.addEventListener('pointermove', onMove);
    canvas.addEventListener('pointerleave', onLeave);
    return () => {
      if (pickFrame) cancelAnimationFrame(pickFrame);
      canvas.removeEventListener('pointermove', onMove);
      canvas.removeEventListener('pointerleave', onLeave);
    };
  }, [onHoverCell]);

  return (
    <div className="matrix-host" role="img" aria-label="Interactive quorum matrix">
      <canvas ref={canvasRef} />
      <span className="matrix-fps" aria-live="off">{fps} fps</span>
    </div>
  );
}
