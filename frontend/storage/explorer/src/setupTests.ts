import '@testing-library/jest-dom/vitest';

// jsdom doesn't implement ResizeObserver, which Recharts' ResponsiveContainer
// relies on. Without a stub, mounting any MetricsChart under jsdom throws.
class ResizeObserverStub {
  observe() {}
  unobserve() {}
  disconnect() {}
}
// eslint-disable-next-line @typescript-eslint/no-explicit-any
(globalThis as any).ResizeObserver = (globalThis as any).ResizeObserver ?? ResizeObserverStub;
