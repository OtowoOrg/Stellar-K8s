/**
 * DR API client — Issue #92
 *


export type DrPhaseKey =
  | 'snapshot_restoration'
  | 'pod_recreation'
  | 'catchup_sync'
  | 'traffic_redirection';

export type DrPhaseStatus =
  | 'pending'
  | 'running'
  | 'passed'
  | 'failed'
  | 'skipped';

export interface DrPhaseState {

  dry_run?: boolean;
}

export interface DrTriggerResponse {
  }

  /**
   * POST /api/dr/trigger
   *
   * Enqueues a dry-run (default) or live failover drill for the given node.

  }

  /**
   * GET /api/dr/status/:drillId
   *

  }

  /**
   * POST /api/dr/reset
   *

  wsUrl(wsBase = 'ws://localhost:8080'): string {
    return `${wsBase}/api/dr/stream`;
  }
}


export default drClient;
