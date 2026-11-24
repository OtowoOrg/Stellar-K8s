/**
 * Shared types for the Persistent Volume storage explorer (#95).
 */

/** A single timestamped sample of PVC storage/I-O metrics. */
export interface StorageMetricSample {
  /** ISO-8601 timestamp of the sample. */
  timestamp: string;
  /** Disk usage as a percentage of PVC capacity (0-100). */
  diskUsagePercent: number;
  /** Read throughput in MiB/s. */
  readThroughputMBps: number;
  /** Write throughput in MiB/s. */
  writeThroughputMBps: number;
  /** I/O wait latency in milliseconds. */
  ioWaitMs: number;
}

/** Identifies a PVC belonging to a StellarNode-managed volume. */
export interface PvcRef {
  namespace: string;
  name: string;
  /** Total provisioned capacity in bytes, used to translate usage% into an absolute date/size projection. */
  capacityBytes: number;
}

/** Response shape for `GET /api/v1/storage/pvcs/:namespace/:name/metrics`. */
export interface StorageMetricsResponse {
  pvc: PvcRef;
  samples: StorageMetricSample[];
}

/** Historical time range presets offered by the explorer. */
export type MetricsRange = '24h' | '7d' | '14d' | '30d';

/** Lifecycle state of a triggered storage I/O benchmark job. */
export type BenchmarkJobState = 'pending' | 'running' | 'succeeded' | 'failed';

/** Result payload once a benchmark job completes. */
export interface BenchmarkResult {
  readIops: number;
  writeIops: number;
  readThroughputMBps: number;
  writeThroughputMBps: number;
  avgLatencyMs: number;
}

/** Response shape for `POST /api/v1/storage/pvcs/:namespace/:name/benchmark` and its status poll. */
export interface BenchmarkJob {
  jobId: string;
  state: BenchmarkJobState;
  startedAt: string;
  finishedAt?: string;
  result?: BenchmarkResult;
  error?: string;
}

/** A linear projection of when a metric will cross a saturation threshold. */
export interface SaturationProjection {
  /** Slope of the fitted line, in percentage points per day. */
  growthPercentPerDay: number;
  /** ISO-8601 date the trend crosses `thresholdPercent`, or null if it never will (flat/negative growth). */
  projectedSaturationDate: string | null;
  /** Days from the last sample until saturation, or null if never. */
  daysUntilSaturation: number | null;
  /** Whether saturation is projected within the configured warning window. */
  isWarning: boolean;
  /** The fitted trend line, sampled at each input timestamp plus a forward projection window. */
  trendLine: Array<{ timestamp: string; value: number }>;
}
