/**
 * xdr_decoder.ts
 *
 * Lightweight XDR / ScVal decoder for Soroban contract event payloads.
 *
 * Soroban encodes all topic and value payloads as XDR-serialised ScVal
 * objects, then base64-encodes the bytes for transport.
 *
 * This module implements the subset of ScVal types needed to pretty-print
 * real-world contract events without pulling in a heavy XDR runtime:
 *
 *   ScValType (discriminant u32):
 *     0  SCV_BOOL
 *     1  SCV_VOID
 *     2  SCV_ERROR
 *     3  SCV_U32
 *     4  SCV_I32
 *     5  SCV_U64
 *     6  SCV_I64
 *     7  SCV_TIMEPOINT (u64)
 *     8  SCV_DURATION  (u64)
 *     9  SCV_U128
 *     10 SCV_I128
 *     11 SCV_U256
 *     12 SCV_I256
 *     13 SCV_BYTES
 *     14 SCV_STRING
 *     15 SCV_SYMBOL
 *     16 SCV_VEC
 *     17 SCV_MAP
 *     18 SCV_ADDRESS
 *     19 SCV_LEDGER_KEY_CONTRACT_INSTANCE
 *     20 SCV_LEDGER_KEY_NONCE
 *     21 SCV_CONTRACT_INSTANCE
 *
 * Reference: https://github.com/stellar/stellar-xdr
 */

// ---------------------------------------------------------------------------
// Decoded ScVal representation (JSON-friendly)
// ---------------------------------------------------------------------------

export type ScValDecoded =
  | { type: 'bool'; value: boolean }
  | { type: 'void' }
  | { type: 'error'; code: number; message: string }
  | { type: 'u32'; value: number }
  | { type: 'i32'; value: number }
  | { type: 'u64'; value: string }      // string to avoid JS precision loss
  | { type: 'i64'; value: string }
  | { type: 'timepoint'; value: string }
  | { type: 'duration'; value: string }
  | { type: 'u128'; value: string }
  | { type: 'i128'; value: string }
  | { type: 'u256'; value: string }
  | { type: 'i256'; value: string }
  | { type: 'bytes'; value: string; hex: string }
  | { type: 'string'; value: string }
  | { type: 'symbol'; value: string }
  | { type: 'vec'; items: ScValDecoded[] }
  | { type: 'map'; entries: Array<{ key: ScValDecoded; value: ScValDecoded }> }
  | { type: 'address'; value: string }
  | { type: 'ledger_key_contract_instance' }
  | { type: 'ledger_key_nonce'; nonce: string }
  | { type: 'contract_instance' }
  | { type: 'unknown'; discriminant: number; raw: string };

// ---------------------------------------------------------------------------
// XDR reader — minimal big-endian buffer reader
// ---------------------------------------------------------------------------

class XdrReader {
  private pos = 0;
  private readonly buf: Uint8Array;

  constructor(buf: Uint8Array) {
    this.buf = buf;
  }

  get remaining(): number {
    return this.buf.length - this.pos;
  }

  readU8(): number {
    if (this.pos >= this.buf.length) throw new Error('XDR underflow (u8)');
    return this.buf[this.pos++];
  }

  readU32(): number {
    if (this.pos + 4 > this.buf.length) throw new Error('XDR underflow (u32)');
    const v =
      ((this.buf[this.pos] << 24) |
        (this.buf[this.pos + 1] << 16) |
        (this.buf[this.pos + 2] << 8) |
        this.buf[this.pos + 3]) >>>
      0;
    this.pos += 4;
    return v;
  }

  readI32(): number {
    const v = this.readU32();
    return v | 0; // sign-extend
  }

  /** Read a u64 as a decimal string (avoids float precision loss). */
  readU64(): string {
    const hi = this.readU32();
    const lo = this.readU32();
    // Use BigInt for accuracy.
    return ((BigInt(hi) << 32n) | BigInt(lo >>> 0)).toString(10);
  }

  /** Read a signed i64 as a decimal string. */
  readI64(): string {
    const hi = this.readU32();
    const lo = this.readU32();
    const full = (BigInt(hi) << 32n) | BigInt(lo >>> 0);
    // Interpret as signed 64-bit.
    const signed =
      full >= 0x8000_0000_0000_0000n
        ? full - 0x1_0000_0000_0000_0000n
        : full;
    return signed.toString(10);
  }

  /** Read a u128 (two u64 big-endian) as a decimal string. */
  readU128(): string {
    const hi = BigInt(this.readU64());
    const lo = BigInt(this.readU64());
    return ((hi << 64n) | lo).toString(10);
  }

  /** Read a u256 (four u64 big-endian) as a decimal string. */
  readU256(): string {
    const a = BigInt(this.readU64());
    const b = BigInt(this.readU64());
    const c = BigInt(this.readU64());
    const d = BigInt(this.readU64());
    return ((a << 192n) | (b << 128n) | (c << 64n) | d).toString(10);
  }

  /** Read an XDR opaque<> block (length-prefixed, padded to 4 bytes). */
  readOpaque(): Uint8Array {
    const len = this.readU32();
    if (this.pos + len > this.buf.length) throw new Error('XDR underflow (opaque)');
    const bytes = this.buf.slice(this.pos, this.pos + len);
    // Skip padding to next 4-byte boundary.
    this.pos += len + ((4 - (len % 4)) % 4);
    return bytes;
  }

  /** Read an XDR string<> block (same as opaque, returned as UTF-8 text). */
  readString(): string {
    const bytes = this.readOpaque();
    return new TextDecoder('utf-8', { fatal: false }).decode(bytes);
  }

  /** Peek at the current position without advancing. */
  peekU32(): number {
    if (this.pos + 4 > this.buf.length) throw new Error('XDR underflow (peek)');
    return (
      ((this.buf[this.pos] << 24) |
        (this.buf[this.pos + 1] << 16) |
        (this.buf[this.pos + 2] << 8) |
        this.buf[this.pos + 3]) >>>
      0
    );
  }

  sliceRemaining(): Uint8Array {
    return this.buf.slice(this.pos);
  }
}

// ---------------------------------------------------------------------------
// Core ScVal decoder
// ---------------------------------------------------------------------------

function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}

function decodeScValInner(r: XdrReader): ScValDecoded {
  const discriminant = r.readU32();

  switch (discriminant) {
    case 0: { // SCV_BOOL
      const v = r.readI32();
      return { type: 'bool', value: v !== 0 };
    }
    case 1: // SCV_VOID
      return { type: 'void' };

    case 2: { // SCV_ERROR
      const code = r.readU32();
      // error body is a union; read the inner discriminant + optional message
      const innerDisc = r.readU32();
      let message = `code=${code} inner=${innerDisc}`;
      // If the error has a string (type 1 = SERR_WASM_VM for example)
      try {
        if (r.remaining >= 4 && r.peekU32() < 0xffffff) {
          message = r.readString();
        }
      } catch { /* ok */ }
      return { type: 'error', code, message };
    }

    case 3: // SCV_U32
      return { type: 'u32', value: r.readU32() };

    case 4: // SCV_I32
      return { type: 'i32', value: r.readI32() };

    case 5: // SCV_U64
      return { type: 'u64', value: r.readU64() };

    case 6: // SCV_I64
      return { type: 'i64', value: r.readI64() };

    case 7: // SCV_TIMEPOINT
      return { type: 'timepoint', value: r.readU64() };

    case 8: // SCV_DURATION
      return { type: 'duration', value: r.readU64() };

    case 9: { // SCV_U128
      // u128 is encoded as hi_u64 + lo_u64
      const hi = r.readU64();
      const lo = r.readU64();
      const val = ((BigInt(hi) << 64n) | BigInt(lo)).toString(10);
      return { type: 'u128', value: val };
    }

    case 10: { // SCV_I128
      const hi = r.readI64();
      const lo = r.readU64();
      const val = ((BigInt(hi) << 64n) | BigInt(lo)).toString(10);
      return { type: 'i128', value: val };
    }

    case 11: { // SCV_U256
      const v = r.readU256();
      return { type: 'u256', value: v };
    }

    case 12: { // SCV_I256
      // I256 uses XDR int256 (signed). Read 32 bytes, interpret as big-endian.
      const bytes = new Uint8Array(32);
      for (let i = 0; i < 32; i++) bytes[i] = r.readU8();
      let big = 0n;
      for (const b of bytes) big = (big << 8n) | BigInt(b);
      const signed =
        big >= (1n << 255n) ? big - (1n << 256n) : big;
      return { type: 'i256', value: signed.toString(10) };
    }

    case 13: { // SCV_BYTES
      const bytes = r.readOpaque();
      return {
        type: 'bytes',
        value: btoa(String.fromCharCode(...bytes)),
        hex: bytesToHex(bytes),
      };
    }

    case 14: // SCV_STRING
      return { type: 'string', value: r.readString() };

    case 15: // SCV_SYMBOL
      return { type: 'symbol', value: r.readString() };

    case 16: { // SCV_VEC — option<vec>: first a presence flag
      const present = r.readU32();
      if (present === 0) return { type: 'vec', items: [] };
      const len = r.readU32();
      const items: ScValDecoded[] = [];
      for (let i = 0; i < len; i++) items.push(decodeScValInner(r));
      return { type: 'vec', items };
    }

    case 17: { // SCV_MAP — option<map>: first a presence flag
      const present = r.readU32();
      if (present === 0) return { type: 'map', entries: [] };
      const len = r.readU32();
      const entries: Array<{ key: ScValDecoded; value: ScValDecoded }> = [];
      for (let i = 0; i < len; i++) {
        const key = decodeScValInner(r);
        const value = decodeScValInner(r);
        entries.push({ key, value });
      }
      return { type: 'map', entries };
    }

    case 18: { // SCV_ADDRESS — ScAddress: accountId (0) or contractId (1)
      const addrType = r.readU32();
      if (addrType === 0) {
        // AccountID — Ed25519 public key
        const keyType = r.readU32(); // 0 = KEY_TYPE_ED25519
        const bytes = new Uint8Array(32);
        for (let i = 0; i < 32; i++) bytes[i] = r.readU8();
        if (keyType === 0) {
          return { type: 'address', value: `account:${bytesToHex(bytes)}` };
        }
        return { type: 'address', value: `account_type${keyType}:${bytesToHex(bytes)}` };
      } else {
        // ContractID — 32-byte hash
        const bytes = new Uint8Array(32);
        for (let i = 0; i < 32; i++) bytes[i] = r.readU8();
        return { type: 'address', value: `contract:${bytesToHex(bytes)}` };
      }
    }

    case 19: // SCV_LEDGER_KEY_CONTRACT_INSTANCE
      return { type: 'ledger_key_contract_instance' };

    case 20: { // SCV_LEDGER_KEY_NONCE
      const nonce = r.readI64();
      return { type: 'ledger_key_nonce', nonce };
    }

    case 21: // SCV_CONTRACT_INSTANCE — complex; just signal the type
      return { type: 'contract_instance' };

    default: {
      // Unknown discriminant — return raw hex of remaining bytes.
      const raw = bytesToHex(r.sliceRemaining());
      return { type: 'unknown', discriminant, raw };
    }
  }
}

// ---------------------------------------------------------------------------
// Public decoding API
// ---------------------------------------------------------------------------

/**
 * Decode a base64-encoded XDR ScVal into a JSON-friendly object.
 *
 * Returns a structured `ScValDecoded` on success, or an error sentinel.
 */
export function decodeScVal(base64: string): ScValDecoded {
  try {
    const binary = atob(base64);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    const reader = new XdrReader(bytes);
    return decodeScValInner(reader);
  } catch (err) {
    return {
      type: 'unknown',
      discriminant: -1,
      raw: base64,
    };
  }
}

/**
 * Decode all topic ScVals and the value ScVal for a contract event.
 * Returns a pretty JSON string.
 */
export function decodeEventPayload(
  topics: string[],
  valueXdr: string,
): { topics: ScValDecoded[]; value: ScValDecoded } {
  return {
    topics: topics.map(decodeScVal),
    value: decodeScVal(valueXdr),
  };
}

/**
 * Returns the first symbol-type topic as a human-readable event name,
 * or null if no symbol topic is present.
 */
export function extractEventName(topics: string[]): string | null {
  for (const t of topics) {
    const decoded = decodeScVal(t);
    if (decoded.type === 'symbol') return decoded.value;
    if (decoded.type === 'string') return decoded.value;
  }
  return null;
}

/**
 * Pretty-print a decoded ScVal tree as indented JSON.
 */
export function prettyPrintScVal(val: ScValDecoded, indent = 2): string {
  return JSON.stringify(val, null, indent);
}
