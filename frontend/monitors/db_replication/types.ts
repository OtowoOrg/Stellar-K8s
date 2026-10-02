export type ReplicationState = 'streaming' | 'catchup' | 'disconnected' | 'unknown';

export interface ReplicaMetrics {
  /** Postgres application_name / replica identifier. */
  name: string;
  /** Host address of the replica. */
  host: string;
  /** Current replication state reported by pg_stat_replication. */
  state: ReplicationState;
  /** Byte lag of the replica relative to the primary (write ahead). */
  byteLag: number;
  /** Replay delay in seconds. */
  replayLatencySeconds: number;
  /** Timestamp (ms) of the last metric sample. */
  lastUpdated: number;
}

export interface PrimaryMetrics {
  /** Host address of the primary writer. */
  host: string;
  /** Number of attached replicas. */
  replicaCount: number;
  /** Timestamp (ms) of the last metric sample. */
  lastUpdated: number;
}

export interface ReplicationSnapshot {
  primary: PrimaryMetrics;
  replicas: ReplicaMetrics[];
}

/** Lag threshold in seconds above which the UI must alert. */
export const LAG_THRESHOLD_SECONDS = 5;

export function isLagging(replica: ReplicaMetrics): boolean {
  return replica.replayLatencySeconds > LAG_THRESHOLD_SECONDS;
}
