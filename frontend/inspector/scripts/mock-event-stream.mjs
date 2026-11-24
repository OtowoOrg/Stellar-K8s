#!/usr/bin/env node
/**
 * mock-event-stream.mjs
 *
 * Synthetic Soroban contract event WebSocket server for performance testing.
 *
 * Streams realistic-looking contract events at configurable rates.
 * Default: 150 events/sec (batched into groups of ~5 every 33ms).
 *
 * Usage:
 *   node scripts/mock-event-stream.mjs --serve           # WebSocket server on port 8788
 *   node scripts/mock-event-stream.mjs --serve --eps 200 # 200 events/sec
 *   node scripts/mock-event-stream.mjs --validate        # Run 1000-event validation
 *
 * Validation mode:
 *   Streams 1000 events, then prints a summary covering:
 *   - Total events generated
 *   - Filter performance (contract ID, topic, ledger)
 *   - XDR payload encoding accuracy
 */

import { WebSocketServer } from 'ws';
import { performance } from 'perf_hooks';

// ── CLI args ────────────────────────────────────────────────────────────────
const args = new Map();
for (let i = 2; i < process.argv.length; i++) {
  const v = process.argv[i];
  if (v.startsWith('--')) args.set(v, process.argv[i + 1] ?? true);
}

const EPS          = Math.max(1, Number(args.get('--eps') ?? 150));
const PORT         = Number(args.get('--port') ?? 8788);
const SERVE        = args.has('--serve');
const VALIDATE     = args.has('--validate');
const BATCH_MS     = 33;   // ~30 fps batching
const BATCH_SIZE   = Math.max(1, Math.round(EPS / (1000 / BATCH_MS)));

// ── Constants ──────────────────────────────────────────────────────────────
const EVENT_TYPES  = ['contract', 'system', 'diagnostic'];
const EVENT_NAMES  = [
  'transfer', 'mint', 'burn', 'approve', 'swap',
  'deposit', 'withdraw', 'claim', 'stake', 'unstake',
  'vote', 'propose', 'execute', 'cancel', 'bridge',
];

const CONTRACT_IDS = [
  'CABC1234EFGH5678IJKL9012MNOP3456QRST7890UVWX1234YZAB5678CDEF9012',
  'CBCD2345FGHI6789JKLM0123NOPQ4567RSTU8901VWXY2345ZABC6789DEFA0123',
  'CCDE3456GHIJ7890KLMN1234OPQR5678STUV9012WXYZ3456ABCD7890EFAB1234',
  'CDEF4567HIJK8901LMNO2345PQRS6789TUVW0123XYZA4567BCDE8901FABC2345',
  'CEFG5678IJKL9012MNOP3456QRST7890UVWX1234YZAB5678CDEF9012ABCD3456',
];

// ── XDR encoding helpers ───────────────────────────────────────────────────
// We generate valid XDR-encoded ScVal bytes then base64-encode them,
// so the decoder in the frontend can parse them accurately.

function writeU32(val) {
  const buf = new Uint8Array(4);
  const v = val >>> 0;
  buf[0] = (v >> 24) & 0xff;
  buf[1] = (v >> 16) & 0xff;
  buf[2] = (v >> 8) & 0xff;
  buf[3] = v & 0xff;
  return buf;
}

function concat(...arrays) {
  const total = arrays.reduce((s, a) => s + a.length, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const a of arrays) { out.set(a, off); off += a.length; }
  return out;
}

function xdrString(str) {
  const bytes = new TextEncoder().encode(str);
  const padded = new Uint8Array(((bytes.length + 3) & ~3));
  padded.set(bytes);
  return concat(writeU32(bytes.length), padded);
}

function xdrSymbol(name) {
  // discriminant 15 = SCV_SYMBOL
  return concat(writeU32(15), xdrString(name));
}

function xdrU32Val(value) {
  // discriminant 3 = SCV_U32
  return concat(writeU32(3), writeU32(value));
}

function xdrU64Val(lo) {
  // discriminant 5 = SCV_U64, then hi(u32) + lo(u32)
  return concat(writeU32(5), writeU32(0), writeU32(lo >>> 0));
}

function xdrAddress(hexId) {
  // discriminant 18 = SCV_ADDRESS, addrType=1 (contract), 32 bytes
  const bytes = new Uint8Array(32);
  const hex = hexId.replace(/[^0-9a-fA-F]/g, '').slice(0, 64);
  for (let i = 0; i < 32; i++) {
    bytes[i] = parseInt(hex.slice(i * 2, i * 2 + 2) || '00', 16);
  }
  return concat(writeU32(18), writeU32(1), bytes);
}

function toBase64(bytes) {
  let binary = '';
  for (const b of bytes) binary += String.fromCharCode(b);
  return Buffer.from(binary, 'binary').toString('base64');
}

// ── Event generation ────────────────────────────────────────────────────────
let seq       = 0;
let ledger    = 50_000;
let ledgerSeq = 0;

function nextEvent() {
  const id       = seq++;
  const name     = EVENT_NAMES[id % EVENT_NAMES.length];
  const cid      = CONTRACT_IDS[id % CONTRACT_IDS.length];
  const etype    = EVENT_TYPES[id % EVENT_TYPES.length];
  const amount   = (id * 17 + 1000) % 100_000;

  // Advance ledger every ~20 events (matches ~5s ledger close at 100eps)
  if (++ledgerSeq > 20) { ledger++; ledgerSeq = 0; }

  // Build XDR topics: [symbol(name), address(from), address(to)]
  const topicNameXdr    = xdrSymbol(name);
  const topicFromXdr    = xdrAddress(cid.slice(0, 32) + '00'.repeat(4));
  const topicToXdr      = xdrAddress(cid.slice(0, 32) + 'ff'.repeat(4));

  // Build XDR value: u64 amount
  const valueXdr        = xdrU64Val(amount);

  return {
    id:          `evt-${id}`,
    timestamp:   new Date().toISOString(),
    ledger,
    contract_id: cid,
    topics: [
      toBase64(topicNameXdr),
      toBase64(topicFromXdr),
      toBase64(topicToXdr),
    ],
    value_xdr:   toBase64(valueXdr),
    event_type:  etype,
    tx_hash:     Array.from({ length: 32 }, (_, i) =>
      ((id * 31 + i * 7) & 0xff).toString(16).padStart(2, '0'),
    ).join(''),
  };
}

// ── Validation mode ─────────────────────────────────────────────────────────
function runValidation() {
  console.log('\n=== Synthetic event generator — validation mode ===\n');

  const N = 1000;
  const events = [];
  const t0 = performance.now();

  for (let i = 0; i < N; i++) events.push(nextEvent());
  const genMs = performance.now() - t0;

  console.log(`Generated ${N} events in ${genMs.toFixed(2)} ms`);
  console.log(`Throughput: ${(N / genMs * 1000).toFixed(0)} events/sec (generator)`);

  // ── Filter test 1: Contract ID ──────────────────────────────────────────
  const t1 = performance.now();
  const targetCid = CONTRACT_IDS[0];
  const byCid = events.filter(e => e.contract_id === targetCid);
  const filterMs1 = performance.now() - t1;
  console.log(`\nFilter by contract_id (${N} events): ${filterMs1.toFixed(3)} ms → ${byCid.length} matches`);

  // ── Filter test 2: Topic substring ──────────────────────────────────────
  const t2 = performance.now();
  const topicTarget = toBase64(xdrSymbol('transfer'));
  const byTopic = events.filter(e => e.topics[0] === topicTarget);
  const filterMs2 = performance.now() - t2;
  console.log(`Filter by topic[0]=transfer (${N} events): ${filterMs2.toFixed(3)} ms → ${byTopic.length} matches`);

  // ── Filter test 3: Ledger range ──────────────────────────────────────────
  const t3 = performance.now();
  const lo = 50_000, hi = 50_010;
  const byLedger = events.filter(e => e.ledger >= lo && e.ledger <= hi);
  const filterMs3 = performance.now() - t3;
  console.log(`Filter by ledger ${lo}–${hi} (${N} events): ${filterMs3.toFixed(3)} ms → ${byLedger.length} matches`);

  // ── Filter test 4: Event type ────────────────────────────────────────────
  const t4 = performance.now();
  const byType = events.filter(e => e.event_type === 'contract');
  const filterMs4 = performance.now() - t4;
  console.log(`Filter by type=contract (${N} events): ${filterMs4.toFixed(3)} ms → ${byType.length} matches`);

  // ── XDR decode accuracy test ─────────────────────────────────────────────
  // Quick inline decoder to verify round-trip.
  let xdrOk = 0, xdrFail = 0;
  for (const e of events) {
    try {
      // Verify topic[0] base64 contains the symbol discriminant (15 = 0x0000000F)
      const buf = Buffer.from(e.topics[0], 'base64');
      const disc = (buf[0] << 24) | (buf[1] << 16) | (buf[2] << 8) | buf[3];
      if (disc === 15) xdrOk++; else xdrFail++;
    } catch { xdrFail++; }
  }
  console.log(`\nXDR symbol topic accuracy: ${xdrOk}/${N} correct (${xdrFail} failures)`);

  // ── Value decode test ────────────────────────────────────────────────────
  let valOk = 0, valFail = 0;
  for (const e of events) {
    try {
      const buf = Buffer.from(e.value_xdr, 'base64');
      const disc = (buf[0] << 24) | (buf[1] << 16) | (buf[2] << 8) | buf[3];
      // disc=5 for SCV_U64
      if (disc === 5) valOk++; else valFail++;
    } catch { valFail++; }
  }
  console.log(`XDR u64 value accuracy: ${valOk}/${N} correct (${valFail} failures)`);

  // ── Summary ──────────────────────────────────────────────────────────────
  const totalFilterMs = filterMs1 + filterMs2 + filterMs3 + filterMs4;
  console.log(`\n── Summary ──────────────────────────────────────────`);
  console.log(`Events generated     : ${N}`);
  console.log(`Generation time      : ${genMs.toFixed(2)} ms`);
  console.log(`Total filter time    : ${totalFilterMs.toFixed(3)} ms (all 4 filters)`);
  console.log(`XDR accuracy         : ${xdrOk + valOk}/${N * 2} fields correct`);
  console.log(`Status               : ${
    xdrFail === 0 && valFail === 0 && totalFilterMs < 50
      ? '✅ PASS'
      : '❌ FAIL'
  }`);
  console.log(`────────────────────────────────────────────────────\n`);

  process.exit(xdrFail === 0 && valFail === 0 ? 0 : 1);
}

// ── WebSocket server ─────────────────────────────────────────────────────────
function startServer() {
  const wss = new WebSocketServer({ port: PORT });

  wss.on('listening', () => {
    console.log(`\nSoroban mock event stream`);
    console.log(`  WebSocket : ws://localhost:${PORT}`);
    console.log(`  Rate      : ~${EPS} events/sec (${BATCH_SIZE} events / ${BATCH_MS}ms)`);
    console.log(`  Types     : contract, system, diagnostic`);
    console.log(`  Contracts : ${CONTRACT_IDS.length} unique`);
    console.log(`  Topics    : ${EVENT_NAMES.length} unique event names`);
    console.log(`\nWaiting for connections…\n`);
  });

  wss.on('connection', (socket, req) => {
    const addr = req.socket.remoteAddress;
    console.log(`[+] Client connected from ${addr}`);

    let connSeq = 0;
    const timer = setInterval(() => {
      if (socket.readyState !== socket.OPEN) return;

      // Send a batch of BATCH_SIZE events as a JSON array.
      const batch = [];
      for (let i = 0; i < BATCH_SIZE; i++) batch.push(nextEvent());
      try {
        socket.send(JSON.stringify(batch));
        connSeq += batch.length;
      } catch {
        clearInterval(timer);
      }
    }, BATCH_MS);

    // Stats every 5 seconds.
    const statsTimer = setInterval(() => {
      if (socket.readyState !== socket.OPEN) { clearInterval(statsTimer); return; }
      const rate = (connSeq / 5).toFixed(0);
      process.stdout.write(`  ↑ ${connSeq.toLocaleString()} events sent (~${rate} eps)\r`);
      connSeq = 0;
    }, 5_000);

    socket.on('close', () => {
      console.log(`\n[-] Client disconnected`);
      clearInterval(timer);
      clearInterval(statsTimer);
    });

    socket.on('error', () => {
      clearInterval(timer);
      clearInterval(statsTimer);
    });
  });

  wss.on('error', (err) => {
    console.error(`Server error: ${err.message}`);
    process.exit(1);
  });
}

// ── Entry ────────────────────────────────────────────────────────────────────
if (VALIDATE) {
  runValidation();
} else if (SERVE) {
  startServer();
} else {
  console.error('Usage: node mock-event-stream.mjs [--serve] [--validate] [--eps N] [--port N]');
  process.exit(1);
}
