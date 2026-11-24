import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import ArgoCdFinalizerWidget from './ArgoCdFinalizerWidget.jsx';
import './styles.css';

/**
 * Read configuration from query parameters so the widget can be embedded
 * inside any ArgoCD-connected dashboard without a rebuild.
 *
 * Usage examples:
 *   ?mode=mock                  — use built-in demo data (default when no base URL)
 *   ?base=https://argo.example.com&token=<JWT>   — live ArgoCD API
 *   ?poll=5000                  — poll every 5 seconds
 */
const query = new URLSearchParams(window.location.search);
const argoCdBaseUrl  = query.get('base')  ?? '';
const token          = query.get('token') ?? '';
const pollIntervalMs = Number(query.get('poll')) || 10_000;
const mode           = (query.get('mode') === 'live' && argoCdBaseUrl) ? 'live' : 'mock';

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <ArgoCdFinalizerWidget
      argoCdBaseUrl={argoCdBaseUrl}
      token={token}
      pollIntervalMs={pollIntervalMs}
      mode={mode}
    />
  </StrictMode>,
);
