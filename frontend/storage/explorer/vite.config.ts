import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

// Mirrors frontend/analytics/vite.config.js's dev-proxy convention: the
// storage explorer talks to the operator's REST API (src/rest_api,
// default port 9090) for /api routes. See src/api/storageMetrics.ts for
// the documented /api/v1/storage/* contract this proxies to once the
// corresponding backend handlers exist.
export default defineConfig({
  plugins: [react()],
  server: {
    port: 5175,
    strictPort: false,
    // frontend/components/metrics_chart.tsx lives outside this app's root
    // (frontend/storage/explorer/); allow the dev server to serve it.
    fs: {
      allow: ['../../'],
    },
    proxy: {
      '/api': {
        target: 'http://localhost:9090',
      },
    },
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/setupTests.ts'],
  },
});
