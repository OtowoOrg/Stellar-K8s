import js from '@eslint/js';
import react from 'eslint-plugin-react';

const browserGlobals = {
  ResizeObserver: 'readonly',
  URLSearchParams: 'readonly',
  WebSocket: 'readonly',
  cancelAnimationFrame: 'readonly',
  clearInterval: 'readonly',
  document: 'readonly',
  performance: 'readonly',
  requestAnimationFrame: 'readonly',
  setInterval: 'readonly',
  window: 'readonly',
};

const nodeGlobals = {
  console: 'readonly',
  process: 'readonly',
  setInterval: 'readonly',
};

export default [
  {
    ignores: ['dist/**', 'node_modules/**'],
  },
  js.configs.recommended,
  {
    files: ['src/**/*.js', 'src/**/*.jsx'],
    plugins: {
      react,
    },
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: 'module',
      globals: browserGlobals,
      parserOptions: {
        ecmaFeatures: { jsx: true },
      },
    },
    rules: {
      'react/jsx-uses-vars': 'error',
    },
  },
  {
    files: ['src/**/*.test.js', 'scripts/**/*.mjs', 'vite.config.js', 'eslint.config.js'],
    languageOptions: {
      ecmaVersion: 2024,
      sourceType: 'module',
      globals: { ...browserGlobals, ...nodeGlobals },
    },
  },
];
