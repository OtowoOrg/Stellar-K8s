import { memo, useEffect, useRef, useState } from 'react';
import { TIMEFRAMES, bucketFees, estimateCongestion, stroopsToXlm } from '../fees/feeModel.js';
import { getFeeFeedState, subscribeFeeFeed } from '../fees/feeFeed.js';

const LEVEL_COLORS = {
  normal: '#39d98a',
  elevated: '#f5b942',
  high: '#f5b942',
  surge: '#f05d5e',
};
const GRID = '#273340';
const MUTED = '#7f92a3';
const SPARK_BAND = '#39d98a22';
const SPARK_LINE = '#a9f0ce';
const MARGIN = { top: 12, right: 14, bottom: 28, left: 46 };
const TARGET_BUCKETS = 72;
const DEFAULT_FLOOR = 100;

function timeLabel(timestamp) {
  return new Date(timestamp).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function FeeChart() {
  const hostRef = useRef(null);
  const [width, setWidth] = useState(720);
  const [timeframe, setTimeframe] = useState('24h');
  const [data, setData] = useState(() => bucketFees(getFeeFeedState().samples, '24h'));
  const [level, setLevel] = useState(() => estimateCongestion(getFeeFeedState().samples).level);
  const [factor, setFactor] = useState(() => estimateCongestion(getFeeFeedState().samples).factor);
  const [latest, setLatest] = useState(getFeeFeedState().latest);
  const [connection, setConnection] = useState(getFeeFeedState().connection);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const observer = new ResizeObserver(() => {
      const next = host.clientWidth || 720;
      setWidth((current) => (Math.abs(current - next) < 1 ? current : next));
    });
    observer.observe(host);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    return subscribeFeeFeed((feed) => {
      setLatest(feed.latest);
      setConnection(feed.connection);
      setLevel(estimateCongestion(feed.samples).level);
      setFactor(estimateCongestion(feed.samples).factor);
      setData((current) => (current.timeframe === timeframe ? bucketFees(feed.samples, timeframe) : current));
    });
  }, [timeframe]);

  const height = 300;
  const innerWidth = Math.max(120, width - MARGIN.left - MARGIN.right);
  const innerHeight = height - MARGIN.top - MARGIN.bottom;
  const buckets = data.buckets;
  const populated = buckets.filter((bucket) => bucket.count > 0);
  const values = populated.map((bucket) => bucket.avg);
  const minValue = values.length ? Math.min(...values) : 0;
  const maxValue = values.length ? Math.max(...values) : DEFAULT_FLOOR;
  const span = Math.max(maxValue - minValue, 1);
  const pad = span * 0.12;
  const low = Math.max(0, Math.floor((minValue - pad) / 100) * 100);
  const high = Math.ceil((maxValue + pad) / 100) * 100;
  const range = Math.max(high - low, 1);
  const x = (index) => (populated.length > 1 ? MARGIN.left + (index / (populated.length - 1)) * innerWidth : MARGIN.left + innerWidth / 2);
  const y = (value) => MARGIN.top + innerHeight - ((value - low) / range) * innerHeight;
  const accent = LEVEL_COLORS[level] ?? LEVEL_COLORS.normal;

  const linePoints = populated.map((bucket, index) => `${x(index)},${y(bucket.avg)}`).join(' ');
  const areaPath = populated.length
    ? `M ${x(0)},${y(low)} L ${linePoints.split(' ').join(' L ')} L ${x(populated.length - 1)},${y(low)} Z`
    : '';
  const spikeBudget = populated.length
    ? populated.slice(0, 24)
    : [];
  const spikeRatio = spikeBudget.length
    ? populated.map((bucket) => bucket.avg / Math.max(1, spikeBudget.reduce((total, item) => total + item.avg, 0) / spikeBudget.length))
    : [];

  const gridLines = [0, 0.25, 0.5, 0.75, 1].map((ratio) => ({ y: MARGIN.top + innerHeight - ratio * innerHeight, value: Math.round(low + ratio * range) }));
  const xTicks = [0, 0.5, 1].map((ratio) => {
    const index = Math.round((populated.length - 1) * ratio);
    return { label: populated[index] ? timeLabel(populated[index].ts) : '', x: x(index) };
  });

  return (
    <div className="fee-chart" ref={hostRef}>
      <div className="fee-chart-head">
        <div>
          <span className={`status-dot ${connection}`} />
          <strong>Average ledger base fee</strong>
          <span className="muted">{timeframe} window</span>
        </div>
        <div className="fee-timeframe" role="group" aria-label="Chart timeframe">
          {Object.keys(TIMEFRAMES).map((key) => (
            <button key={key} type="button" className={key === timeframe ? 'active' : ''} onClick={() => setTimeframe(key)}>
              {TIMEFRAMES[key].label}
            </button>
          ))}
        </div>
      </div>
      <div className="fee-chart-body">
        <svg width={width} height={height} role="img" aria-label="Time series of average ledger base fees">
          <defs>
            <clipPath id="fee-plot-clip">
              <rect x={MARGIN.left} y={MARGIN.top} width={innerWidth} height={innerHeight} rx={4} />
            </clipPath>
          </defs>
          {gridLines.map((line) => (
            <g key={line.value}>
              <line x1={MARGIN.left} y1={line.y} x2={width - MARGIN.right} y2={line.y} stroke={GRID} strokeWidth={1} strokeDasharray="3 5" />
              <text x={MARGIN.left - 6} y={line.y + 3} textAnchor="end" fill={MUTED} className="fee-axis" fontSize="10">{line.value}</text>
            </g>
          ))}
          {areaPath ? <path d={areaPath} fill={SPARK_BAND} clipPath="url(#fee-plot-clip)" /> : null}
          <path d={linePoints ? `M ${linePoints}` : ''} fill="none" stroke={accent} strokeWidth={2} clipPath="url(#fee-plot-clip)" />
          {populated.map((bucket, index) =>
            spikeRatio[index] >= 2.5 ? <circle key={bucket.ts} cx={x(index)} cy={y(bucket.avg)} r={3} fill="#f05d5e" /> : null,
          )}
          {latest ? (
            <g>
              <line x1={MARGIN.left} y1={y(latest.baseFee)} x2={width - MARGIN.right} y2={y(latest.baseFee)} stroke={accent} strokeWidth={1} strokeDasharray="2 4" opacity={0.8} />
              <circle cx={MARGIN.left + innerWidth} cy={y(latest.baseFee)} r={3.5} fill={accent} />
              <text x={width - MARGIN.right - 4} y={y(latest.baseFee) - 6} textAnchor="end" fill={accent} className="fee-axis" fontSize="10">{latest.baseFee.toLocaleString()}</text>
            </g>
          ) : null}
          {xTicks.map((tick) => (
            <text key={tick.label} x={tick.x} y={height - 8} textAnchor="middle" fill={MUTED} className="fee-axis" fontSize="10">{tick.label}</text>
          ))}
          {!populated.length ? (
            <text x={MARGIN.left + innerWidth / 2} y={MARGIN.top + innerHeight / 2} textAnchor="middle" fill={MUTED} className="fee-axis" fontSize="12">
              Waiting for fee telemetry…
            </text>
          ) : null}
          <text x={width - MARGIN.right} y={MARGIN.top + 10} textAnchor="end" fill={accent} className="fee-axis" fontSize="10">
            {level.toUpperCase()} · factor {factor.toFixed(2)}
          </text>
        </svg>
      </div>
      <div className="fee-chart-foot muted">
        <span>Current live base fee: {latest ? `${latest.baseFee.toLocaleString()} stroops (${stroopsToXlm(latest.baseFee).toFixed(7)} XLM)` : 'no live sample'}</span>
        <span className="fee-spark-note">● spike buckets above steady-state ratio</span>
      </div>
    </div>
  );
}

export default memo(FeeChart);