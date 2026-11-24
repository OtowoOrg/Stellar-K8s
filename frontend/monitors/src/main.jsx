import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import IngressCertDashboard from './ingress/IngressCertDashboard.jsx';
import './styles.css';

/**
 * Main application entry point for the Ingress TLS Certificate Monitor.
 *
 * In production this component would:
 *  - Fetch /api/v1/ingress/certs on mount (polling or WebSocket)
 *  - Pass the raw IngressCertRecord[] via the `records` prop
 *
 * For local development / demo, `useMock={true}` loads the fixture set.
 */
function App() {
  /**
   * In a real integration, replace useMock with fetch state:
   *
   *   const [records, setRecords] = useState([]);
   *   useEffect(() => {
   *     fetch('/api/v1/ingress/certs')
   *       .then(r => r.json())
   *       .then(data => setRecords(data.items ?? data));
   *   }, []);
   *
   *   return <IngressCertDashboard records={records} onRenew={handleRenew} />;
   */
  async function handleRenew(certRow) {
    // POST to cert-manager annotation patch endpoint.
    // In a real app this calls the backend; for mock mode it resolves immediately.
    await new Promise((resolve) => setTimeout(resolve, 800));
    console.info('[cert-monitor] Force renewal triggered for:', certRow.host);
  }

  return (
    <IngressCertDashboard
      useMock
      onRenew={handleRenew}
      lastSyncAt={new Date().toISOString()}
    />
  );
}

createRoot(document.getElementById('root')).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
