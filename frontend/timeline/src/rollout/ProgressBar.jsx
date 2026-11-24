import { clamp01 } from './phases.js';

/**
 * Custom progress bar for internal node catch-up status. Accessible via the
 * progressbar role; tone drives the fill colour (default, amber, green, red).
 */
export default function ProgressBar({ value, label, detail = null, tone = 'default' }) {
  const percent = Math.round(clamp01(value) * 100);
  return (
    <div className={`progress-bar progress-${tone}`}>
      <div className="progress-head">
        <span className="progress-label">{label}</span>
        {detail ? <span className="muted">{detail}</span> : null}
      </div>
      <div
        className="progress-track"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={percent}
        aria-label={label}
      >
        <div className="progress-fill" style={{ width: `${percent}%` }} />
      </div>
    </div>
  );
}
