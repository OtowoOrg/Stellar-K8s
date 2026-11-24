#!/usr/bin/env node
import { WebSocketServer } from 'ws';

const args = new Map();
for (let index = 2; index < process.argv.length; index += 1) {
  const value = process.argv[index];
  if (value.startsWith('--')) args.set(value, process.argv[index + 1] ?? true);
}

const port = Number(args.get('--port') ?? 8788);
const intervalMs = Math.max(200, Number(args.get('--interval') ?? 1000));
const historyHours = Math.max(1, Number(args.get('--history-hours') ?? 24));
const spikeHours = (args.get('--spike-hours') ?? '9,21').split(',').map(Number).filter(Number.isFinite);
const serve = args.has('--serve');
const HISTORY_BUCKET_MS = 5 * 60 * 1000;

function baseFeeAt(timestamp) {
  const date = new Date(timestamp);
  const hour = date.getHours();
  const minutes = date.getMinutes();
  const inSpike = spikeHours.some((spike) => hour === spike && minutes < 40);
  const steady = 100 + Math.round(Math.sin((hour + minutes / 60) / 24 * Math.PI * 4) * 18);
  if (!inSpike) return Math.max(90, steady);
  return 900 + ((hour + minutes) % 5) * 120;
}

function historySample(timestamp, sequence) {
  return {
    timestamp,
    base_fee: baseFeeAt(timestamp),
    ledger_close_ms: 3.8 + (sequence % 7) / 10,
    ledger_sequence: sequence,
    tps: 820 + (sequence % 180),
  };
}

function liveSample(sequence) {
  const timestamp = Date.now();
  return {
    timestamp,
    base_fee: baseFeeAt(timestamp),
    ledger_close_ms: 3.7 + ((timestamp / 10) % 5) / 10,
    ledger_sequence: sequence,
    tps: 780 + ((sequence * 13) % 240),
  };
}

function buildHistory() {
  const history = [];
  const now = Date.now();
  const start = now - historyHours * 3_600_000;
  let sequence = Math.floor(start / 5000);
  for (let timestamp = start; timestamp < now; timestamp += HISTORY_BUCKET_MS) {
    history.push(historySample(timestamp, sequence++));
  }
  return history;
}

function emit(record) {
  if (!process.stdout.destroyed) process.stdout.write(`${JSON.stringify(record)}\n`);
}

process.stdout.on('error', (error) => {
  if (error.code === 'EPIPE') process.exit(0);
});

if (!serve) {
  emit({ timestamp: Date.now(), fee_history: buildHistory() });
  let sequence = 0;
  setInterval(() => emit(liveSample(sequence++)), intervalMs);
} else {
  const server = new WebSocketServer({ port });
  server.on('listening', () => {
    console.log(`Mock fee stream listening on ws://localhost:${port}`);
    console.log(`History: ${historyHours}h at 5-minute buckets, spikes at hour(s) ${spikeHours.join(', ')}, ${intervalMs}ms live ticks`);
  });
  server.on('connection', (socket) => {
    socket.send(JSON.stringify({ timestamp: Date.now(), fee_history: buildHistory() }));
    let sequence = 0;
    const timer = setInterval(() => {
      if (socket.readyState === socket.OPEN) socket.send(JSON.stringify(liveSample(sequence++)));
    }, intervalMs);
    socket.on('close', () => clearInterval(timer));
  });
}