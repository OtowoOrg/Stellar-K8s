import { useEffect, useState } from 'react';
import { startFeeFeed, stopFeeFeed } from '../fees/feeFeed.js';
import FeeChart from './FeeChart.jsx';
import FeeTierPanel from './FeeTierPanel.jsx';
import FeeCalculator from './FeeCalculator.jsx';

export default function FeeExplorer() {
  const [source, setSource] = useState('mock');

  useEffect(() => {
    startFeeFeed(source);
    return () => stopFeeFeed();
  }, [source]);

  return (
    <section className="fee-explorer" aria-label="Network congestion and dynamic fee estimator explorer">
      <header className="fee-explorer-head">
        <div>
          <span className="eyebrow">STELLAR / FEES</span>
          <h2>Network congestion &amp; fee estimator</h2>
          <p>Ledger base fee trends, live priority tiers, and invocation cost projection.</p>
        </div>
        <label className="select-wrap">
          <span>Fee telemetry</span>
          <select value={source} onChange={(event) => setSource(event.target.value)}>
            <option value="mock">Mock fee stream</option>
            <option value="live">Live operator stream</option>
          </select>
        </label>
      </header>
      <div className="fee-grid">
        <div className="fee-chart-card">
          <FeeChart />
        </div>
        <FeeTierPanel />
        <FeeCalculator />
      </div>
    </section>
  );
}