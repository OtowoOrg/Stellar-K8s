import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import { PHASES, PHASE_INDEX, phaseIndex, deriveRolloutView, stepStates, replicaStatus, STALL_THRESHOLD_SAMPLES } from './phases.js';
import { diagnoseReplica } from './diagnostics.js';
import { createSimulation, runSimulation, OLD_IMAGE, NEW_IMAGE } from './simulate.js';
import { normalizeApiSnapshot } from './useRolloutStream.js';

/** Fold raw snapshots through deriveRolloutView, returning the last view. */
function deriveAll(snapshots) {
  let view = null;
  for (const snapshot of snapshots) view = deriveRolloutView(snapshot, view);
  return view;
}

function rawReplica(overrides = {}) {
  return {
    ordinal: 0,
    name: 'my-validator-0',
    image: NEW_IMAGE,
    updated: true,
    phase: 'fully-synced',
    phaseProgress: 1,
    containerStatus: 'Ready',
    containerReady: true,
    restartCount: 0,
    phaseDetail: null,
    ...overrides,
  };
}

describe('api snapshot normalization', () => {
  it('passes rollout-shaped payloads through unchanged', () => {
    const payload = { revision: 7, replicas: [rawReplica()] };
    assert.equal(normalizeApiSnapshot(payload), payload);
  });

  it('synthesizes replicas from a StellarNodeStatus node-detail response', () => {
    const payload = {
      name: 'my-validator',
      namespace: 'stellar-system',
      version: 'stellar/core:20.5.0',
      status: { phase: 'Syncing', replicas: 3, ready_replicas: 1, ledger_sequence: 5123400 },
    };
    const snapshot = normalizeApiSnapshot(payload);
    assert.equal(snapshot.nodeName, 'my-validator');
    assert.equal(snapshot.desiredReplicas, 3);
    assert.equal(snapshot.replicas[1].phase, 'history-catchup');
    assert.equal(snapshot.replicas[1].containerReady, false);
    assert.equal(snapshot.replicas[0].containerReady, true);
    assert.deepEqual(snapshot.replicas[2].phaseDetail, { currentLedger: 5123400, targetLedger: 5123400 });
  });

  it('reports idle nodes as fully synced', () => {
    const snapshot = normalizeApiSnapshot({ name: 'idle', status: { phase: 'Running', replicas: 2, ready_replicas: 2 } });
    assert.equal(snapshot.replicas.every((replica) => replica.phase === 'fully-synced' && replica.containerReady), true);
  });
});

describe('phase model', () => {
  it('defines the four Stellar initialization phases in pipeline order', () => {
    assert.deepEqual(
      PHASES.map((phase) => phase.id),
      ['database-schema-migration', 'history-catchup', 'quorum-peering', 'fully-synced'],
    );
    assert.equal(phaseIndex('history-catchup'), 1);
    assert.equal(phaseIndex('nope'), -1);
    assert.equal(PHASE_INDEX['fully-synced'], 3);
  });

  it('clamps phase progress into [0, 1]', () => {
    const view = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 1.7 })],
    });
    assert.equal(view.replicas[0].phaseProgress, 1);
    const view2 = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: -3 })],
    });
    assert.equal(view2.replicas[0].phaseProgress, 0);
  });

  it('falls back to fully-synced for unknown phases', () => {
    const view = deriveRolloutView({ revision: 1, replicas: [rawReplica({ phase: 'mystery' })] });
    assert.equal(view.replicas[0].phase, 'fully-synced');
  });

  it('computes overall progress across the phase pipeline', () => {
    // Catchup at 50% = (1 + 0.5) / 4 = 37.5%
    const view = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 0.5 })],
    });
    assert.equal(view.replicas[0].overallProgress, 0.375);
  });

  it('derives stepper step states from the current phase', () => {
    const replica = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 0.5 })],
    }).replicas[0];
    assert.deepEqual(stepStates(replica), ['done', 'active', 'pending', 'pending']);
  });
});

describe('diagnostics', () => {
  it('returns null for a healthy ready replica', () => {
    const replica = deriveRolloutView({ revision: 1, replicas: [rawReplica()] }).replicas[0];
    assert.equal(diagnoseReplica(replica), null);
  });

  it('explains an actively progressing catchup with ledger numbers', () => {
    const replica = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 0.5, containerReady: false, containerStatus: 'Running', phaseDetail: { currentLedger: 5100000, targetLedger: 5142300 } })],
    }).replicas[0];
    const message = diagnoseReplica(replica);
    assert.match(message, /Catching up: ledger 5,100,000 → 5,142,300 \(50%\)/);
  });

  it('flags a crash-looping container with a log hint', () => {
    const replica = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'quorum-peering', containerReady: false, containerStatus: 'CrashLoopBackOff', restartCount: 3 })],
    }).replicas[0];
    const message = diagnoseReplica(replica);
    assert.match(message, /CrashLoopBackOff/);
    assert.match(message, /kubectl logs my-validator-0 --previous/);
  });

  it('explains the schema migration init gate', () => {
    const replica = deriveRolloutView({
      revision: 1,
      replicas: [rawReplica({ phase: 'database-schema-migration', phaseProgress: 0.4, containerReady: false, containerStatus: 'Init:0/1' })],
    }).replicas[0];
    assert.match(diagnoseReplica(replica), /Database schema migration 40%/);
  });

  it('detects a stalled catchup only after repeated zero-progress samples', () => {
    let view = null;
    const frames = [];
    for (let i = 0; i < STALL_THRESHOLD_SAMPLES + 2; i += 1) {
      frames.push({
        revision: i + 1,
        replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 0.5, containerReady: false, containerStatus: 'Running' })],
      });
    }
    for (const frame of frames) view = deriveRolloutView(frame, view);
    const replica = view.replicas[0];
    assert.equal(replica.stalled, true);
    assert.match(diagnoseReplica(replica), /History Catchup is stalled/);
  });

  it('does not flag progress as a stall', () => {
    let view = null;
    for (let i = 0; i < STALL_THRESHOLD_SAMPLES + 2; i += 1) {
      view = deriveRolloutView(
        { revision: i + 1, replicas: [rawReplica({ phase: 'history-catchup', phaseProgress: 0.3 + i * 0.1, containerReady: false, containerStatus: 'Running' })] },
        view,
      );
    }
    assert.equal(view.replicas[0].stalled, false);
  });
});

describe('3-pod rolling update simulation (Pod #1 stuck in History Catchup)', () => {
  it('starts with every replica synced on the old image', () => {
    const simulation = createSimulation();
    const view = deriveRolloutView(simulation.snapshot());
    assert.equal(view.desiredReplicas, 3);
    assert.deepEqual(
      view.replicas.map((replica) => [replica.phase, replica.containerReady, replica.image]),
      [
        ['fully-synced', true, OLD_IMAGE],
        ['fully-synced', true, OLD_IMAGE],
        ['fully-synced', true, OLD_IMAGE],
      ],
    );
    assert.equal(view.bottleneck, null);
  });

  it('rolls the highest ordinal through the full pipeline first', () => {
    const { frames } = runSimulation({}, 20);
    const view = deriveAll(frames);
    const pod2 = view.replicas[2];
    assert.equal(pod2.image, NEW_IMAGE);
    assert.equal(pod2.updated, true);
    assert.equal(pod2.phase, 'fully-synced');
    assert.equal(pod2.containerReady, true);
    // pod-1 and pod-0 have not rolled yet
    assert.equal(view.replicas[0].updated, false);
    assert.equal(view.replicas[1].updated, false);
  });

  it('freezes Pod #1 in History Catchup, isolating it as the bottleneck', () => {
    const { frames } = runSimulation({}, 40);
    const view = deriveAll(frames);
    const pod1 = view.replicas[1];

    // Pod #1 reached catchup and froze at the freeze point.
    assert.equal(pod1.phase, 'history-catchup');
    assert.equal(pod1.phaseProgress, 0.53);
    assert.equal(pod1.containerReady, false);
    assert.equal(pod1.stalled, true);
    assert.equal(pod1.bottleneck, true);
    assert.equal(replicaStatus(pod1), 'bottleneck');
    assert.match(diagnoseReplica(pod1), /stalled/);
    assert.match(diagnoseReplica(pod1), /history archive/);

    // Pod #0 is gated behind it and never rolls.
    const pod0 = view.replicas[0];
    assert.equal(pod0.updated, false);
    assert.equal(pod0.blocked, true);
    assert.equal(pod0.blockedBy?.name, 'my-validator-1');
    assert.equal(replicaStatus(pod0), 'gated');
    assert.match(diagnoseReplica(pod0), /Waiting for my-validator-1/);

    // The whole rollout waits on exactly one bottleneck.
    assert.equal(view.bottleneck?.name, 'my-validator-1');
    assert.equal(view.replicas.filter((replica) => replica.bottleneck).length, 1);

    // Stepper isolates the blocked step for the stuck pod.
    assert.deepEqual(stepStates(pod1), ['done', 'blocked', 'pending', 'pending']);
  });

  it('releases the gate once the stuck replica resumes', () => {
    const { frames, simulation } = runSimulation({}, 40);
    const stuckView = deriveAll(frames);
    assert.equal(stuckView.bottleneck?.name, 'my-validator-1');

    simulation.unstick();
    const resumeFrames = [];
    for (let i = 0; i < 40; i += 1) {
      simulation.tick();
      resumeFrames.push(simulation.snapshot());
    }
    const finalView = deriveAll([...frames, ...resumeFrames]);

    assert.deepEqual(
      finalView.replicas.map((replica) => [replica.ordinal, replica.phase, replica.containerReady, replica.image]),
      [
        [0, 'fully-synced', true, NEW_IMAGE],
        [1, 'fully-synced', true, NEW_IMAGE],
        [2, 'fully-synced', true, NEW_IMAGE],
      ],
    );
    assert.equal(finalView.bottleneck, null);
    assert.equal(finalView.replicas.filter((replica) => replica.blocked).length, 0);
    assert.equal(finalView.replicas.every((replica) => diagnoseReplica(replica) === null), true);
  });

  it('completes without a stuck pod when stuckOrdinal is disabled', () => {
    const { frames } = runSimulation({ stuckOrdinal: -1 }, 80);
    const view = deriveAll(frames);
    assert.equal(view.replicas.every((replica) => replica.containerReady), true);
    assert.equal(view.bottleneck, null);
  });
});
