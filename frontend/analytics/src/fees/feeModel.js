export const STROOPS_PER_XLM = 10_000_000;
export const DEFAULT_BASE_FEE = 100;
export const MAX_SAMPLES = 200_000;
export const MAX_SAMPLE_AGE_MS = 7 * 24 * 60 * 60 * 1000;
export const CONGESTION_WINDOW = 12;
export const INFERRED_BASE_FEE_MIN = 100;
export const INFERRED_BASE_FEE_MAX = 4000;

export const TIMEFRAMES = {
  '1h': { windowMs: 60 * 60 * 1000, label: '1 hour' },
  '6h': { windowMs: 6 * 60 * 60 * 1000, label: '6 hours' },
  '24h': { windowMs: 24 * 60 * 60 * 1000, label: '24 hours' },
  '7d': { windowMs: 7 * 24 * 60 * 60 * 1000, label: '7 days' },
};

export const TIER_MULTIPLIERS = {
  low: 1,
  medium: 2,
  high: 4,
};

export const TIER_LEVELS = {
  normal: 'normal',
  elevated: 'elevated',
  high: 'high',
  surge: 'surge',
};

export const RESOURCE_RATES = {
  cpuPerInstruction: 1 / 500,
  readPerByte: 1 / 2,
  writePerByte: 3 / 4,
  perEvent: 20,
};

function asNumber(value, fallback = 0) {
  const number = Number(value);
  return Number.isFinite(number) ? number : fallback;
}

function clamp(value, min, max) {
  return Math.min(max, Math.max(min, value));
}

function round2(value) {
  return Math.round(value * 100) / 100;
}

function metric(source, names, fallback) {
  for (const name of names) {
    if (source?.[name] !== undefined) return asNumber(source[name], fallback);
    if (source?.metrics?.[name] !== undefined) return asNumber(source.metrics[name], fallback);
    if (source?.metadata?.[name] !== undefined) return asNumber(source.metadata[name], fallback);
  }
  return fallback;
}

export function ceilTo100(value) {
  return Math.max(100, Math.ceil(value / 100) * 100);
}

export function stroopsToXlm(stroops) {
  return stroops / STROOPS_PER_XLM;
}

export function formatFee(stroops) {
  const value = Math.round(stroops);
  const xlm = stroopsToXlm(value);
  const xlmText = xlm >= 0.0000001 ? Number(xlm.toFixed(7)).toString() : '0';
  return `${value.toLocaleString()} stroops · ${xlmText} XLM`;
}

function meanBaseFee(samples = []) {
  if (!samples.length) return 0;
  let total = 0;
  for (const sample of samples) total += sample.baseFee;
  return total / samples.length;
}

export function parseFeeSample(payload = {}) {
  const raw = payload ?? {};
  const baseFee = metric(raw, ['base_fee', 'baseFee', 'ledger_base_fee', 'ledgerBaseFee', 'effective_base_fee', 'effectiveBaseFee', 'fee_charged', 'feeCharged', 'min_fee', 'minFee'], Number.NaN);
  const tps = metric(raw, ['tps', 'transactions_per_second'], Number.NaN);
  const inferred = !Number.isFinite(baseFee);
  if (inferred && !Number.isFinite(tps)) return null;
  const fee = inferred ? clamp(INFERRED_BASE_FEE_MIN + tps / 20, INFERRED_BASE_FEE_MIN, INFERRED_BASE_FEE_MAX) : baseFee;
  const ledgerCloseMs = metric(raw, ['ledger_time_ms', 'ledgerTimeMs', 'ledger_close_ms', 'ledgerCloseMs'], Number.NaN);
  const ledgerSequence = asNumber(raw.ledger_sequence ?? raw.ledgerSequence, 0);
  return {
    timestamp: metric(raw, ['timestamp', 'ts', 'recorded_at', 'recordedAt'], Date.now()),
    baseFee: Math.round(fee),
    ledgerCloseMs: Number.isFinite(ledgerCloseMs) ? ledgerCloseMs : null,
    ledgerSequence,
    tps: Number.isFinite(tps) ? tps : null,
    inferred,
  };
}

export function createFeeState() {
  return {
    samples: [],
    latest: null,
    updatedAt: null,
  };
}

export function ingestFeeSample(state, payload = {}) {
  const history = Array.isArray(payload.fee_history)
    ? payload.fee_history
    : Array.isArray(payload.feeHistory)
      ? payload.feeHistory
      : null;
  if (history) {
    let next = state;
    for (const entry of history) next = ingestFeeSample(next, entry);
    if (history.length) {
      const timestamp = asNumber(payload.timestamp, next.samples[next.samples.length - 1]?.timestamp ?? Date.now());
      return { samples: next.samples, latest: next.latest, updatedAt: timestamp };
    }
    return next;
  }
  const sample = parseFeeSample(payload);
  if (!sample) return state;
  const cutoff = Date.now() - MAX_SAMPLE_AGE_MS;
  const samples = [...state.samples, sample];
  while (samples.length > MAX_SAMPLES) samples.shift();
  while (samples.length && samples[0].timestamp < cutoff) samples.shift();
  return { samples, latest: sample, updatedAt: sample.timestamp };
}

export function estimateCongestion(samples = []) {
  const recent = samples.slice(-CONGESTION_WINDOW);
  const recentMean = meanBaseFee(recent);
  const baselineMean = meanBaseFee(samples);
  const factor = recentMean > 0 && baselineMean > 0 ? round2(recentMean / baselineMean) : 1;
  const clamped = Math.max(1, factor);
  let level = TIER_LEVELS.normal;
  if (clamped >= 2.5) level = TIER_LEVELS.surge;
  else if (clamped >= 1.6) level = TIER_LEVELS.high;
  else if (clamped >= 1.2) level = TIER_LEVELS.elevated;
  return { factor: clamped, level, recentMean, baselineMean };
}

export function computeFeeTiers(samples = []) {
  const congestion = estimateCongestion(samples);
  const baseRaw = congestion.recentMean || DEFAULT_BASE_FEE;
  const baseFee = ceilTo100(baseRaw);
  return {
    low: ceilTo100(baseFee * TIER_MULTIPLIERS.low),
    medium: ceilTo100(baseFee * TIER_MULTIPLIERS.medium),
    high: ceilTo100(baseFee * TIER_MULTIPLIERS.high),
    baseFee,
    congestion,
  };
}

export function feeBumpProjection(tiers = {}) {
  return ['low', 'medium', 'high'].map((tier) => ({
    tier,
    maxFee: tiers[tier] ?? 0,
    xlm: stroopsToXlm(tiers[tier] ?? 0),
    multiplier: TIER_MULTIPLIERS[tier],
  }));
}

export function bucketFees(samples = [], timeframe = '24h') {
  const resolvedTimeframe = TIMEFRAMES[timeframe] ? timeframe : '24h';
  const config = TIMEFRAMES[resolvedTimeframe];
  const now = Date.now();
  const bucketMs = Math.max(1000, Math.round(config.windowMs / 72 / 1000) * 1000);
  const start = now - config.windowMs;
  const buckets = [];
  for (let ts = start; ts <= now; ts += bucketMs) {
    buckets.push({ ts, count: 0, avg: null, min: null, max: null });
  }
  for (const sample of samples) {
    if (sample.timestamp < start || sample.timestamp > now) continue;
    const index = Math.floor((sample.timestamp - start) / bucketMs);
    if (index < 0 || index >= buckets.length) continue;
    const bucket = buckets[index];
    bucket.count += 1;
    bucket.avg = (bucket.avg ?? 0) + sample.baseFee;
    bucket.min = bucket.min === null ? sample.baseFee : Math.min(bucket.min, sample.baseFee);
    bucket.max = bucket.max === null ? sample.baseFee : Math.max(bucket.max, sample.baseFee);
  }
  return {
    timeframe: resolvedTimeframe,
    windowMs: config.windowMs,
    bucketMs,
    buckets: buckets.map((bucket) => ({
      ...bucket,
      avg: bucket.count ? Math.round(bucket.avg / bucket.count) : null,
    })),
  };
}

export function calculateInvocation(input = {}) {
  const operations = Math.max(0, asNumber(input.operations, 1));
  const instructions = Math.max(0, asNumber(input.instructions, 0));
  const readBytes = Math.max(0, asNumber(input.readBytes, 0));
  const writeBytes = Math.max(0, asNumber(input.writeBytes, 0));
  const events = Math.max(0, asNumber(input.events, 0));
  const tier = TIER_MULTIPLIERS[input.tier] ? input.tier : 'low';
  const baseFee = Math.max(1, asNumber(input.baseFee, DEFAULT_BASE_FEE));
  const inclusionFee = Math.ceil(baseFee * Math.max(1, operations));
  const resourceFee = Math.ceil(
    instructions * RESOURCE_RATES.cpuPerInstruction
      + readBytes * RESOURCE_RATES.readPerByte
      + writeBytes * RESOURCE_RATES.writePerByte
      + events * RESOURCE_RATES.perEvent,
  );
  const subtotal = inclusionFee + resourceFee;
  const multiplier = TIER_MULTIPLIERS[tier];
  const maxFee = ceilTo100(Math.ceil(subtotal) * multiplier);
  return {
    operations,
    instructions,
    readBytes,
    writeBytes,
    events,
    tier,
    baseFee,
    inclusionFee,
    resourceFee,
    subtotal,
    multiplier,
    maxFee,
    maxFeeXlm: stroopsToXlm(maxFee),
  };
}