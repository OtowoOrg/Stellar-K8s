import {
  CartesianGrid,
  Legend,
  Line,
  LineChart,
  ReferenceLine,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from 'recharts';

/**
 * Shared time-series chart component (#95).
 *
 * Used by the storage explorer (`frontend/storage/explorer/`) to render
 * Disk Usage %, Read/Write Throughput, and I/O Wait Latency, but kept
 * generic/dependency-free of that app so any other `frontend/*` app in this
 * repo can reuse it for its own metric time series.
 */

export interface ChartDatum {
  /** ISO-8601 timestamp; the x-axis. */
  timestamp: string;
  [seriesKey: string]: number | string | undefined;
}

export interface ChartSeriesDef {
  /** Key into each `ChartDatum` holding this series' y-value. */
  key: string;
  label: string;
  color: string;
  /** Appended to values in the tooltip, e.g. "%" or " MB/s". */
  unit?: string;
}

export interface TrendPoint {
  timestamp: string;
  value: number;
}

export interface MetricsChartProps {
  title: string;
  /** Historical samples, one object per timestamp. */
  data: ChartDatum[];
  series: ChartSeriesDef[];
  /** Optional projected/forecast trend line, rendered dashed and merged onto the same x-axis. */
  trendLine?: TrendPoint[];
  trendLineLabel?: string;
  trendLineColor?: string;
  /** Optional horizontal reference line marking a saturation/warning threshold. */
  thresholdValue?: number;
  thresholdLabel?: string;
  yDomain?: [number | 'auto', number | 'auto'];
  height?: number;
  /** Renders the chart with a warning-styled border/heading when saturation is imminent. */
  warning?: boolean;
}

const TREND_KEY = '__trend';

function formatTimestamp(ts: string): string {
  const d = new Date(ts);
  return d.toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit' });
}

/**
 * Merges historical samples with an (optionally longer/shorter, differently
 * sampled) trend line into a single timestamp-sorted array Recharts can
 * render as overlapping series, so a forecast that extends past the last
 * historical sample still draws correctly.
 */
function mergeForChart(data: ChartDatum[], trendLine?: TrendPoint[]): ChartDatum[] {
  if (!trendLine || trendLine.length === 0) return data;

  const byTimestamp = new Map<string, ChartDatum>();
  for (const d of data) byTimestamp.set(d.timestamp, { ...d });
  for (const t of trendLine) {
    const existing = byTimestamp.get(t.timestamp);
    if (existing) {
      existing[TREND_KEY] = t.value;
    } else {
      byTimestamp.set(t.timestamp, { timestamp: t.timestamp, [TREND_KEY]: t.value });
    }
  }

  return [...byTimestamp.values()].sort(
    (a, b) => new Date(a.timestamp).getTime() - new Date(b.timestamp).getTime(),
  );
}

/**
 * Generic, multi-series time-series chart with an optional projected trend
 * line and saturation threshold marker. Renders smoothly across multi-day
 * historical datasets (Recharts virtualizes nothing, but a `LineChart` over
 * a few thousand points remains smooth; the storage explorer downsamples
 * range queries server-side/at the API layer rather than here).
 */
export function MetricsChart({
  title,
  data,
  series,
  trendLine,
  trendLineLabel = 'Projected trend',
  trendLineColor = '#f59e0b',
  thresholdValue,
  thresholdLabel,
  yDomain = ['auto', 'auto'],
  height = 280,
  warning = false,
}: MetricsChartProps) {
  const merged = mergeForChart(data, trendLine);

  return (
    <div
      className={`metrics-chart${warning ? ' metrics-chart--warning' : ''}`}
      role="figure"
      aria-label={title}
    >
      <div className="metrics-chart__header">
        <h3>{title}</h3>
        {warning && (
          <span className="metrics-chart__badge" role="alert">
            ⚠ Saturation warning
          </span>
        )}
      </div>
      <ResponsiveContainer width="100%" height={height}>
        <LineChart data={merged} margin={{ top: 8, right: 16, left: 0, bottom: 8 }}>
          <CartesianGrid strokeDasharray="3 3" opacity={0.25} />
          <XAxis dataKey="timestamp" tickFormatter={formatTimestamp} minTickGap={40} />
          <YAxis domain={yDomain} width={48} />
          <Tooltip
            labelFormatter={(label) => new Date(label as string).toLocaleString()}
            formatter={(value: number, name: string) => {
              const s = series.find((s) => s.label === name);
              return [`${Number(value).toFixed(2)}${s?.unit ?? ''}`, name];
            }}
          />
          <Legend />
          {thresholdValue !== undefined && (
            <ReferenceLine
              y={thresholdValue}
              stroke="#dc2626"
              strokeDasharray="6 4"
              label={{ value: thresholdLabel ?? `Threshold (${thresholdValue})`, position: 'insideTopRight', fill: '#dc2626', fontSize: 11 }}
            />
          )}
          {series.map((s) => (
            <Line
              key={s.key}
              type="monotone"
              dataKey={s.key}
              name={s.label}
              stroke={s.color}
              dot={false}
              isAnimationActive={false}
              connectNulls
              strokeWidth={2}
            />
          ))}
          {trendLine && trendLine.length > 0 && (
            <Line
              type="monotone"
              dataKey={TREND_KEY}
              name={trendLineLabel}
              stroke={trendLineColor}
              strokeDasharray="6 4"
              dot={false}
              isAnimationActive={false}
              connectNulls
              strokeWidth={2}
            />
          )}
        </LineChart>
      </ResponsiveContainer>
    </div>
  );
}

export default MetricsChart;
