import { memo, useEffect, useState } from 'react';
import { calculateInvocation, computeFeeTiers, formatFee } from '../fees/feeModel.js';
import { getFeeFeedState, subscribeFeeFeed } from '../fees/feeFeed.js';

const TIER_OPTIONS = [
  { value: 'low', label: 'Low' },
  { value: 'medium', label: 'Medium' },
  { value: 'high', label: 'High' },
];

function Field({ label, children }) {
  return (
    <label className="fee-field">
      <span>{label}</span>
      {children}
    </label>
  );
}

function toNumber(event, fallback = 0) {
  const value = Number(event.target.value);
  return Number.isFinite(value) && value >= 0 ? value : fallback;
}

function FeeCalculator() {
  const [feed, setFeed] = useState(() => getFeeFeedState());
  useEffect(() => subscribeFeeFeed(setFeed), []);
  const baseFee = computeFeeTiers(feed.samples).baseFee;

  const [operations, setOperations] = useState(1);
  const [instructionsM, setInstructionsM] = useState(1);
  const [readKb, setReadKb] = useState(4);
  const [writeKb, setWriteKb] = useState(2);
  const [events, setEvents] = useState(5);
  const [tier, setTier] = useState('medium');

  const result = calculateInvocation({
    operations,
    instructions: instructionsM * 1_000_000,
    readBytes: readKb * 1024,
    writeBytes: writeKb * 1024,
    events,
    tier,
    baseFee,
  });

  return (
    <section className="fee-panel fee-calculator" aria-label="Fee calculator">
      <div className="fee-panel-head">
        <div>
          <span className="eyebrow">INVOCATION COST</span>
          <h2>Fee calculator</h2>
        </div>
      </div>
      <div className="fee-calc-inputs">
        <Field label={`Classic operations (${operations})`}>
          <input type="range" min="0" max="50" step="1" value={operations} onChange={(event) => setOperations(toNumber(event))} />
        </Field>
        <Field label={`Soroban CPU instructions (${instructionsM} M)`}>
          <input type="range" min="0" max="50" step="1" value={instructionsM} onChange={(event) => setInstructionsM(toNumber(event))} />
        </Field>
        <Field label={`Ledger read bytes (${readKb} KB)`}>
          <input type="range" min="0" max="200" step="1" value={readKb} onChange={(event) => setReadKb(toNumber(event))} />
        </Field>
        <Field label={`Ledger write bytes (${writeKb} KB)`}>
          <input type="range" min="0" max="100" step="1" value={writeKb} onChange={(event) => setWriteKb(toNumber(event))} />
        </Field>
        <Field label={`Events emitted (${events})`}>
          <input type="range" min="0" max="50" step="1" value={events} onChange={(event) => setEvents(toNumber(event))} />
        </Field>
        <Field label="Priority tier">
          <select value={tier} onChange={(event) => setTier(event.target.value)}>
            {TIER_OPTIONS.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
        </Field>
      </div>
      <dl className="fee-calc-results">
        <div className="detail-row"><dt>Inclusion fee</dt><dd>{result.inclusionFee.toLocaleString()} stroops</dd></div>
        <div className="detail-row"><dt>Resource fee</dt><dd>{result.resourceFee.toLocaleString()} stroops</dd></div>
        <div className="detail-row"><dt>Subtotal (base)</dt><dd>{result.subtotal.toLocaleString()} stroops</dd></div>
        <div className="detail-row"><dt>Tier multiplier</dt><dd>×{result.multiplier}</dd></div>
        <div className="detail-row fee-calc-total"><dt>Recommended maxFee</dt><dd>{formatFee(result.maxFee)}</dd></div>
      </dl>
      <p className="muted fee-calc-note">
        Estimates adapt to the live base fee ({baseFee.toLocaleString()} stroops) read from the congestion feed.
      </p>
    </section>
  );
}

export default memo(FeeCalculator);