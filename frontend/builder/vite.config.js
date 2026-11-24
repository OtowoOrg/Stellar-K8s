import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  root: 'alerts',
  server: {
    port: 5175,
    strictPort: false,
    proxy: {
      '/api': {
        target: 'http://localhost:9090',
      },
    },
  },
  build: {
    outDir: '../dist',
    emptyOutDir: true,
  },
});
