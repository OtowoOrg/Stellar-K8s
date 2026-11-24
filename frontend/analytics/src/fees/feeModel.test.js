import test from 'node:test';
import assert from 'node:assert/strict';
import {
  TIMEFRAMES,
  bucketFees,
  calculateInvocation,
  ceilTo100,
  computeFeeTiers,
  createFeeState,
  estimateCongestion,
  feeBumpProjection,
  formatFee,
  ingestFeeSample,
  parseFeeSample,
  stroopsToXlm,
} from './feeModel.js';

function sample(fee, index = 0) {
  return {
    timestamp: Date.now() - 300 * 60_000 + index * 60_000,
    baseFee: fee,
    ledgerCloseMs: 4.2,
    ledgerSequence: index,
    tps: 900,
    inferred: false,
  };
}

function steadyHistory(count, fee = 100) {
  return Array.from({ length: count }, (_, index) => sample(fee, index));
}

test('parses fee samples from snake_case, camelCase, metrics and metadata', () => {
  assert.equal(parseFeeSample({ base_fee: 250 }).baseFee, 250);
  assert.equal(parseFeeSample({ baseFee: '300' }).baseFee, 300);
  assert.equal(parseFeeSample({ metrics: { ledger_base_fee: 140 } }).baseFee, 140);
  assert.equal(parseFeeSample({ metadata: { min_fee: 175 } }).baseFee, 175);
  assert.equal(parseFeeSample({ fee_charged: 900 }).baseFee, 900);
  assert.equal(parseFeeSample({ tps: 2500 }).baseFee, 100 + 2500 / 20);
  assert.equal(parseFeeSample({ tps: 2500 }).inferred, true);
  assert.equal(parseFeeSample({}), null);
});

test('ingests live samples into an ordered buffer and caps size', () => {
  let state = createFeeState();
  for (let index = 0; index < 10; index += 1) state = ingestFeeSample(state, sample(200 + index, index));
  assert.equal(state.samples.length, 10);
  assert.equal(state.latest.baseFee, 209);
  assert.equal(state.updatedAt, state.latest.timestamp);
});

test('replays historical fee spike batches through ingest', () => {
  let state = createFeeState();
  state = ingestFeeSample(state, { timestamp: Date.now(), fee_history: steadyHistory(72, 100) });
  assert.equal(state.samples.length, 72);
  assert.equal(state.latest.baseFee, 100);
});

test('drops samples older than the retention window', () => {
  const old = { timestamp: Date.now() - 8 * 24 * 60 * 60 * 1000, baseFee: 100 };
  const fresh = { timestamp: Date.now(), baseFee: 120 };
  const state = ingestFeeSample(createFeeState(), { fee_history: [old, fresh] });
  assert.equal(state.samples.length, 1);
  assert.equal(state.samples[0].baseFee, 120);
});

test('detects congestion levels and factors from historical fee spikes', () => {
  const calm = steadyHistory(120, 100);
  const spiked = [...calm, ...steadyHistory(12, 1500).map((value) => ({ ...value, baseFee: 1500 }))];
  const calmCongestion = estimateCongestion(calm);
  const spikedCongestion = estimateCongestion(spiked);
  assert.equal(calmCongestion.level, 'normal');
  assert.equal(calmCongestion.factor, 1);
  assert.equal(spikedCongestion.level, 'surge');
  assert.ok(spikedCongestion.factor >= 2.5);
  assert.ok(spikedCongestion.recentMean > calmCongestion.recentMean);
});

test('adjusts recommended fee tiers when surge spikes are fed in', () => {
  const calm = steadyHistory(120, 100);
  const spiked = [...calm, ...steadyHistory(12, 1500).map((value) => ({ ...value, baseFee: 1500 }))];
  const calmTiers = computeFeeTiers(calm);
  const spikeTiers = computeFeeTiers(spiked);
  assert.equal(calmTiers.baseFee, 100);
  assert.ok(spikeTiers.baseFee > calmTiers.baseFee);
  assert.ok(spikeTiers.low > calmTiers.low);
  assert.ok(spikeTiers.medium > calmTiers.medium);
  assert.ok(spikeTiers.high > calmTiers.high);
});

test('keeps fee-bump tier ordering low < medium < high', () => {
  const tiers = computeFeeTiers(steadyHistory(40, 250));
  assert.ok(tiers.low <= tiers.medium);
  assert.ok(tiers.medium <= tiers.high);
  const projection = feeBumpProjection(tiers);
  assert.deepEqual(
    projection.map((item) => item.tier),
    ['low', 'medium', 'high'],
  );
  assert.equal(projection[0].maxFee, tiers.low);
  assert.equal(projection[2].multiplier, 4);
});

test('buckets fee samples over custom timeframes as average base fees', () => {
  const history = [];
  for (let index = 0; index < 24 * 12; index += 1) history.push(sample(100 + (index % 4), index));
  const bucketed = bucketFees(history, '24h');
  assert.equal(bucketed.timeframe, '24h');
  assert.equal(bucketed.windowMs, TIMEFRAMES['24h'].windowMs);
  assert.ok(bucketed.buckets.length > 24);
  const populated = bucketed.buckets.filter((bucket) => bucket.count > 0);
  assert.ok(populated.length > 0);
  for (const bucket of populated) {
    assert.ok(bucket.avg >= 100 && bucket.avg <= 103);
    assert.ok(bucket.min <= bucket.avg && bucket.max >= bucket.avg);
  }
});

test('buckets default to the 24h timeframe for unknown keys', () => {
  const bucketed = bucketFees(steadyHistory(60), 'not-a-key');
  assert.equal(bucketed.timeframe, '24h');
});

test('calculates invocation costs from op count and Soroban resource inputs', () => {
  const result = calculateInvocation({
    operations: 2,
    instructions: 500_000,
    readBytes: 4_096,
    writeBytes: 2_048,
    events: 5,
    tier: 'medium',
    baseFee: 100,
  });
  assert.equal(result.operations, 2);
  assert.equal(result.inclusionFee, 200);
  assert.equal(result.resourceFee, Math.ceil(500_000 / 500 + 4096 / 2 + 2048 * 0.75 + 5 * 20));
  assert.equal(result.subtotal, result.inclusionFee + result.resourceFee);
  assert.equal(result.maxFee, ceilTo100(result.subtotal * 2));
  assert.equal(result.maxFeeXlm, result.maxFee / 10_000_000);
});

test('calculator uses the live base fee and defaults to low tier safely', () => {
  const live = calculateInvocation({ operations: 3, baseFee: 500 });
  const defaulted = calculateInvocation({ operations: 3, baseFee: 500, tier: 'nope' });
  assert.equal(live.inclusionFee, 1500);
  assert.equal(defaulted.tier, 'low');
  assert.equal(defaulted.maxFee, ceilTo100(defaulted.subtotal));
});

test('formats fees in stroops and XLM', () => {
  assert.equal(stroopsToXlm(10_000_000), 1);
  assert.ok(formatFee(250).includes('250 stroops'));
  assert.ok(formatFee(10_000_000).includes('1 XLM'));
  assert.equal(ceilTo100(50), 100);
  assert.equal(ceilTo100(151), 200);
});