import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: '.',
  plugins: [react()],
  resolve: {
    alias: {
      /**
       * The event service lives at frontend/services/event_stream.ts.
       * Components import from '../../services/event_stream' (relative from
       * src/events/).  This alias makes that import resolve correctly.
       */
      '../../services/event_stream': path.resolve(
        __dirname,
        '../services/event_stream.ts',
      ),
    },
  },
  server: {
    port: 5175,
    strictPort: false,
    proxy: {
      '/api': {
        target: 'http://localhost:9090',
        ws: true,
      },
      '/ws': {
        target: 'ws://localhost:9090',
        ws: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    sourcemap: true,
    rollupOptions: {
      input: {
        main: path.resolve(__dirname, 'index.html'),
      },
    },
  },
});
