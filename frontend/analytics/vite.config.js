import { defineConfig } from 'vite';
import path from 'node:path';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  build: {
    rollupOptions: {
      // perf.html mounts the matrix against the deterministic mock topology
      // for browser profiling; see scripts/browser-perf.mjs.
      input: {
        main: path.resolve(__dirname, 'index.html'),
        perf: path.resolve(__dirname, 'perf.html'),
      },
    },
  },
  resolve: {
    alias: {
      // Shared WebGL components live outside the analytics package root.
      '@webgl': path.resolve(__dirname, '../components/webgl'),
      // Deps are installed under frontend/analytics/node_modules only.
      three: path.resolve(__dirname, 'node_modules/three'),
    },
  },
  server: {
    port: 5174,
    strictPort: false,
    proxy: {
      '/api/prometheus': {
        target: 'http://localhost:9091',
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api\/prometheus/, ''),
      },
      '/api': {
        target: 'http://localhost:9090',
        ws: true,
      },
      '/ws': {
        target: 'ws://localhost:9090',
        ws: true,
      },
      // Local `npm run mock:prometheus` for the saturation heatmap.
      '/mock-prom': {
        target: 'http://localhost:9091',
        rewrite: (path) => path.replace(/^\/mock-prom/, ''),
      },
    },
  },
  build: {
    rollOptions: {
      external: ['d3'],
    },
  },
});
