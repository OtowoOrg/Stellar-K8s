import { memo, useEffect, useState } from 'react';
import { computeFeeTiers, feeBumpProjection, formatFee } from '../fees/feeModel.js';
import { getFeeFeedState, subscribeFeeFeed } from '../fees/feeFeed.js';

const LEVEL_LABELS = {
  normal: 'Normal',
  elevated: 'Elevated',
  high: 'High',
  surge: 'Surge',
};

function FeeTierPanel() {
  const [feed, setFeed] = useState(() => getFeeFeedState());
  useEffect(() => subscribeFeeFeed(setFeed), []);
  const tiers = computeFeeTiers(feed.samples);
  const projection = feeBumpProjection(tiers);

  return (
    <aside className="fee-panel" aria-label="Recommended priority fee tiers">
      <div className="fee-panel-head">
        <div>
          <span className="eyebrow">PRIORITY FEES</span>
          <h2>Recommended fee tiers</h2>
        </div>
        <span className={`status-dot ${feed.connection}`} />
      </div>
      <dl className="fee-tiers">
        <div className="fee-tier-summary">
          <dt>Live base fee</dt>
          <dd>{tiers.baseFee.toLocaleString()} stroops</dd>
        </div>
        <div className="fee-tier-summary">
          <dt>Congestion</dt>
          <dd className={`tone-${congestionTone(tiers.congestion.level)}`}>
            {LEVEL_LABELS[tiers.congestion.level] ?? tiers.congestion.level}
            <span className="muted"> ×{tiers.congestion.factor.toFixed(2)}</span>
          </dd>
        </div>
      </dl>
      <div className="fee-tier-rows">
        {projection.map((item) => (
          <div className="fee-tier-row" key={item.tier}>
            <span className={`fee-tier-name tone-${tierTone(item.tier)}`}>{item.tier}</span>
            <span className="fee-tier-mult muted">×{item.multiplier} base</span>
            <strong>{item.maxFee.toLocaleString()}</strong>
            <span className="muted">{formatFeeXlm(item.xlm)}</span>
          </div>
        ))}
      </div>
      <p className="inspector-note">
        Recommend a fee-bump <strong>maxFee</strong> of {tiers.medium.toLocaleString()} stroops while congestion is{' '}
        {LEVEL_LABELS[tiers.congestion.level]?.toLowerCase()}.
      </p>
    </aside>
  );
}

function tierTone(tier) {
  if (tier === 'high') return 'red';
  if (tier === 'medium') return 'amber';
  return 'green';
}

function congestionTone(level) {
  if (level === 'surge') return 'red';
  if (level === 'high' || level === 'elevated') return 'amber';
  return 'green';
}

function formatFeeXlm(xlm) {
  if (!xlm) return '0 XLM';
  return `${Number(xlm.toFixed(7)).toString()} XLM`;
}

export default memo(FeeTierPanel);