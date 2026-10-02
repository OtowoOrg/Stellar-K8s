//! Minimal, allocation-light XDR decoder for the Stellar structures the
//! verifier needs, running natively inside WASM.
//!
//! The decoder works on a plain byte slice (the contract copies each host
//! `Bytes` argument into linear memory once), so decoding costs only WASM
//! instructions rather than one host call per field. Every structure is
//! decoded strictly: discriminants must be known, padding must be zero,
//! array lengths are bounded by the bytes remaining, and top-level parsers
//! reject trailing bytes. Anything the decoder accepts therefore has exactly
//! one encoding, which is what makes hashing the input meaningful.
//!
//! Supported structures:
//! * `LedgerHeader` (including the signed `StellarValue`),
//! * `TransactionResultSet` (all operation result types, classic and Soroban),
//! * `TransactionEnvelope` (v0, v1 and fee-bump) with every classic operation
//!   except `RevokeSponsorship`. Soroban transactions are reported as
//!   unsupported by the envelope decoder but are fully supported by
//!   hash-based inclusion proofs.

use alloc::vec::Vec;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum XdrError {
    /// Input ended before the structure was complete.
    Truncated,
    /// Unknown discriminant, bad padding, over-long array or similar.
    Invalid,
    /// Bytes remained after the top-level structure.
    TrailingBytes,
    /// Well-formed but outside what the envelope decoder supports.
    Unsupported,
}

pub type XdrResult<T> = Result<T, XdrError>;

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    pub fn position(&self) -> usize {
        self.pos
    }

    fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    pub fn finish(&self) -> XdrResult<()> {
        if self.pos == self.buf.len() {
            Ok(())
        } else {
            Err(XdrError::TrailingBytes)
        }
    }

    #[inline(always)]
    pub fn take(&mut self, n: usize) -> XdrResult<&'a [u8]> {
        if self.remaining() < n {
            return Err(XdrError::Truncated);
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    #[inline(always)]
    pub fn skip(&mut self, n: usize) -> XdrResult<()> {
        if self.remaining() < n {
            return Err(XdrError::Truncated);
        }
        self.pos += n;
        Ok(())
    }

    pub fn fixed<const N: usize>(&mut self) -> XdrResult<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    #[inline(always)]
    pub fn u32(&mut self) -> XdrResult<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline(always)]
    pub fn i32(&mut self) -> XdrResult<i32> {
        self.u32().map(|v| v as i32)
    }

    #[inline(always)]
    pub fn u64(&mut self) -> XdrResult<u64> {
        Ok((u64::from(self.u32()?) << 32) | u64::from(self.u32()?))
    }

    #[inline(always)]
    pub fn i64(&mut self) -> XdrResult<i64> {
        self.u64().map(|v| v as i64)
    }

    /// XDR `bool` / optional discriminant: exactly 0 or 1.
    pub fn flag(&mut self) -> XdrResult<bool> {
        match self.u32()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(XdrError::Invalid),
        }
    }

    /// Variable-length opaque / string with maximum length `max`.
    pub fn var_opaque(&mut self, max: u32) -> XdrResult<&'a [u8]> {
        let len = self.u32()?;
        if len > max {
            return Err(XdrError::Invalid);
        }
        let data = self.take(len as usize)?;
        let pad = (4 - (len as usize % 4)) % 4;
        if self.take(pad)?.iter().any(|b| *b != 0) {
            return Err(XdrError::Invalid);
        }
        Ok(data)
    }

    /// Array length prefix, bounded by `max` and by the bytes remaining
    /// (every element occupies at least `min_elem` bytes).
    pub fn array_len(&mut self, max: u32, min_elem: usize) -> XdrResult<u32> {
        let len = self.u32()?;
        if len > max {
            return Err(XdrError::Invalid);
        }
        if (len as usize).saturating_mul(min_elem) > self.remaining() {
            return Err(XdrError::Truncated);
        }
        Ok(len)
    }

    /// `union switch (int v) { case 0: void; }` extension point.
    pub fn empty_ext(&mut self) -> XdrResult<()> {
        match self.i32()? {
            0 => Ok(()),
            _ => Err(XdrError::Invalid),
        }
    }
}

// ---------------------------------------------------------------------------
// Shared primitives
// ---------------------------------------------------------------------------

/// `PublicKey` / `AccountID` / `NodeID`: only `PUBLIC_KEY_TYPE_ED25519`.
fn account_id(r: &mut Reader) -> XdrResult<[u8; 32]> {
    match r.i32()? {
        0 => r.fixed::<32>(),
        _ => Err(XdrError::Invalid),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Muxed {
    pub key: [u8; 32],
    pub id: Option<u64>,
}

fn muxed_account(r: &mut Reader) -> XdrResult<Muxed> {
    match r.i32()? {
        0 => Ok(Muxed {
            key: r.fixed()?,
            id: None,
        }),
        0x100 => {
            let id = r.u64()?;
            Ok(Muxed {
                key: r.fixed()?,
                id: Some(id),
            })
        }
        _ => Err(XdrError::Invalid),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Asset {
    Native,
    Alphanum4 { code: [u8; 4], issuer: [u8; 32] },
    Alphanum12 { code: [u8; 12], issuer: [u8; 32] },
}

fn asset(r: &mut Reader) -> XdrResult<Asset> {
    match r.i32()? {
        0 => Ok(Asset::Native),
        1 => Ok(Asset::Alphanum4 {
            code: r.fixed()?,
            issuer: account_id(r)?,
        }),
        2 => Ok(Asset::Alphanum12 {
            code: r.fixed()?,
            issuer: account_id(r)?,
        }),
        _ => Err(XdrError::Invalid),
    }
}

/// Skips an `AccountID` without copying the key.
fn skip_account_id(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0 => r.skip(32),
        _ => Err(XdrError::Invalid),
    }
}

/// Skips an `Asset` without copying it.
fn skip_asset(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0 => Ok(()),
        1 => {
            r.skip(4)?;
            skip_account_id(r)
        }
        2 => {
            r.skip(12)?;
            skip_account_id(r)
        }
        _ => Err(XdrError::Invalid),
    }
}

fn price(r: &mut Reader) -> XdrResult<()> {
    r.skip(8)
}

fn claimable_balance_id(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0 => r.skip(32),
        _ => Err(XdrError::Invalid),
    }
}

fn signer_key(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0..=2 => r.skip(32),
        3 => {
            r.skip(32)?;
            r.var_opaque(64).map(|_| ())
        }
        _ => Err(XdrError::Invalid),
    }
}

// ---------------------------------------------------------------------------
// Ledger header
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValueSignature {
    pub node_id: [u8; 32],
    pub signature: [u8; 64],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LedgerHeader {
    pub ledger_version: u32,
    pub previous_ledger_hash: [u8; 32],
    pub tx_set_hash: [u8; 32],
    pub close_time: u64,
    pub value_signature: Option<ValueSignature>,
    pub tx_set_result_hash: [u8; 32],
    pub bucket_list_hash: [u8; 32],
    pub ledger_seq: u32,
}

pub fn parse_ledger_header(buf: &[u8]) -> XdrResult<LedgerHeader> {
    let mut r = Reader::new(buf);
    let ledger_version = r.u32()?;
    let previous_ledger_hash = r.fixed()?;

    // StellarValue
    let tx_set_hash = r.fixed()?;
    let close_time = r.u64()?;
    let upgrades = r.array_len(6, 4)?;
    for _ in 0..upgrades {
        r.var_opaque(128)?;
    }
    let value_signature = match r.i32()? {
        0 => None,
        1 => {
            let node_id = account_id(&mut r)?;
            let sig = r.var_opaque(64)?;
            let signature: [u8; 64] = sig.try_into().map_err(|_| XdrError::Invalid)?;
            Some(ValueSignature { node_id, signature })
        }
        _ => return Err(XdrError::Invalid),
    };

    let tx_set_result_hash = r.fixed()?;
    let bucket_list_hash = r.fixed()?;
    let ledger_seq = r.u32()?;
    // totalCoins, feePool, inflationSeq, idPool, baseFee, baseReserve,
    // maxTxSetSize, skipList[4]
    r.skip(8 + 8 + 4 + 8 + 4 + 4 + 4 + 4 * 32)?;
    match r.i32()? {
        0 => {}
        1 => {
            r.skip(4)?; // flags
            r.empty_ext()?;
        }
        _ => return Err(XdrError::Invalid),
    }
    r.finish()?;

    Ok(LedgerHeader {
        ledger_version,
        previous_ledger_hash,
        tx_set_hash,
        close_time,
        value_signature,
        tx_set_result_hash,
        bucket_list_hash,
        ledger_seq,
    })
}

// ---------------------------------------------------------------------------
// Transaction results
// ---------------------------------------------------------------------------

fn claim_atom(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0 => r.skip(32 + 8)?, // sellerEd25519, offerID
        1 => {
            skip_account_id(r)?;
            r.skip(8)?; // offerID
        }
        2 => r.skip(32)?, // liquidityPoolID
        _ => return Err(XdrError::Invalid),
    }
    skip_asset(r)?;
    r.skip(8)?;
    skip_asset(r)?;
    r.skip(8)
}

fn claim_atoms(r: &mut Reader) -> XdrResult<()> {
    let n = r.array_len(u32::MAX, 4)?;
    for _ in 0..n {
        claim_atom(r)?;
    }
    Ok(())
}

fn manage_offer_result(r: &mut Reader) -> XdrResult<()> {
    if r.i32()? != 0 {
        return Ok(());
    }
    claim_atoms(r)?;
    match r.i32()? {
        0 | 1 => {
            // OfferEntry
            skip_account_id(r)?;
            r.skip(8)?;
            skip_asset(r)?;
            skip_asset(r)?;
            r.skip(8)?;
            price(r)?;
            r.skip(4)?;
            r.empty_ext()
        }
        2 => Ok(()),
        _ => Err(XdrError::Invalid),
    }
}

fn path_payment_result(r: &mut Reader) -> XdrResult<()> {
    match r.i32()? {
        0 => {
            claim_atoms(r)?;
            skip_account_id(r)?;
            skip_asset(r)?;
            r.skip(8)
        }
        -9 => skip_asset(r), // NO_ISSUER
        _ => Ok(()),
    }
}

/// Operation result body keyed by `OperationType`.
fn operation_inner_result(r: &mut Reader, op_type: i32) -> XdrResult<()> {
    match op_type {
        2 | 13 => path_payment_result(r),
        3 | 4 | 12 => manage_offer_result(r),
        8 => {
            if r.i32()? == 0 {
                r.skip(8)?;
            }
            Ok(())
        }
        9 => {
            if r.i32()? == 0 {
                let n = r.array_len(u32::MAX, 44)?;
                r.skip(n as usize * 44)?; // (AccountID, int64) pairs
            }
            Ok(())
        }
        14 => {
            if r.i32()? == 0 {
                claimable_balance_id(r)?;
            }
            Ok(())
        }
        24 => {
            if r.i32()? == 0 {
                r.skip(32)?;
            }
            Ok(())
        }
        0 | 1 | 5 | 6 | 7 | 10 | 11 | 15..=23 | 25 | 26 => r.i32().map(|_| ()),
        _ => Err(XdrError::Invalid),
    }
}

fn operation_results(r: &mut Reader) -> XdrResult<()> {
    let n = r.array_len(u32::MAX, 4)?;
    for _ in 0..n {
        match r.i32()? {
            0 => {
                let op_type = r.i32()?;
                operation_inner_result(r, op_type)?;
            }
            -6..=-1 => {}
            _ => return Err(XdrError::Invalid),
        }
    }
    Ok(())
}

/// Result code body shared by inner and outer results (non fee-bump codes).
fn plain_result_body(r: &mut Reader, code: i32) -> XdrResult<()> {
    match code {
        0 | -1 => operation_results(r),
        -17..=-2 if code != -13 => Ok(()),
        _ => Err(XdrError::Invalid),
    }
}

/// Parses a `TransactionResult`, returning its result code.
fn transaction_result(r: &mut Reader) -> XdrResult<i32> {
    r.skip(8)?; // feeCharged
    let code = r.i32()?;
    match code {
        1 | -13 => {
            // InnerTransactionResultPair
            r.skip(32)?;
            r.skip(8)?;
            let inner = r.i32()?;
            plain_result_body(r, inner)?;
            r.empty_ext()?;
        }
        _ => plain_result_body(r, code)?,
    }
    r.empty_ext()?;
    Ok(code)
}

/// Walks a `TransactionResultSet`, calling `visit(index, tx_hash, code)` for
/// every pair in application order. Returns the number of pairs.
pub fn parse_result_set(buf: &[u8], mut visit: impl FnMut(u32, &[u8; 32], i32)) -> XdrResult<u32> {
    let mut r = Reader::new(buf);
    let n = r.array_len(u32::MAX, 32 + 16)?;
    for i in 0..n {
        let hash = r.fixed::<32>()?;
        let code = transaction_result(&mut r)?;
        visit(i, &hash, code);
    }
    r.finish()?;
    Ok(n)
}

/// Splits a `TransactionResultSet` into its encoded `TransactionResultPair`s:
/// the reference helper for off-chain tooling. Not a contract export, so it
/// is stripped from the WASM.
pub fn split_result_set(buf: &[u8]) -> XdrResult<Vec<&[u8]>> {
    let mut r = Reader::new(buf);
    let n = r.array_len(u32::MAX, 32 + 16)?;
    let mut pairs = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let start = r.position();
        r.skip(32)?;
        transaction_result(&mut r)?;
        pairs.push(&buf[start..r.position()]);
    }
    r.finish()?;
    Ok(pairs)
}

// ---------------------------------------------------------------------------
// Transaction envelopes
// ---------------------------------------------------------------------------

pub const ENVELOPE_TYPE_TX_V0: i32 = 0;
pub const ENVELOPE_TYPE_TX: i32 = 2;
pub const ENVELOPE_TYPE_TX_FEE_BUMP: i32 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Memo<'a> {
    None,
    Text(&'a [u8]),
    Id(u64),
    Hash([u8; 32]),
    Return([u8; 32]),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpBody {
    CreateAccount {
        destination: [u8; 32],
        starting_balance: i64,
    },
    Payment {
        destination: Muxed,
        asset: Asset,
        amount: i64,
    },
    PathPaymentStrictReceive {
        send_asset: Asset,
        send_max: i64,
        destination: Muxed,
        dest_asset: Asset,
        dest_amount: i64,
    },
    PathPaymentStrictSend {
        send_asset: Asset,
        send_amount: i64,
        destination: Muxed,
        dest_asset: Asset,
        dest_min: i64,
    },
    AccountMerge {
        destination: Muxed,
    },
    /// A supported operation whose fields are not surfaced.
    Other {
        op_type: u32,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Operation {
    pub source: Option<Muxed>,
    pub body: OpBody,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transaction<'a> {
    pub source: Muxed,
    pub fee: u32,
    pub seq_num: i64,
    /// `(min_time, max_time)` when the transaction carries time bounds.
    pub time_bounds: Option<(u64, u64)>,
    pub memo: Memo<'a>,
    pub operations: Vec<Operation>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FeeBump {
    pub fee_source: Muxed,
    pub fee: i64,
}

/// Decoded envelope plus the exact bytes the transaction hash commits to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Envelope<'a> {
    /// `ENVELOPE_TYPE_TX` or `ENVELOPE_TYPE_TX_FEE_BUMP`; v0 envelopes hash
    /// as `ENVELOPE_TYPE_TX`.
    pub hash_envelope_type: i32,
    /// Prefix a v0 transaction needs to become its v1 encoding.
    pub v0_prefix: bool,
    /// The XDR-encoded (fee-bump) transaction covered by the hash.
    pub hashed_bytes: &'a [u8],
    pub fee_bump: Option<FeeBump>,
    pub tx: Transaction<'a>,
}

fn time_bounds(r: &mut Reader) -> XdrResult<(u64, u64)> {
    Ok((r.u64()?, r.u64()?))
}

fn preconditions(r: &mut Reader) -> XdrResult<Option<(u64, u64)>> {
    match r.i32()? {
        0 => Ok(None),
        1 => time_bounds(r).map(Some),
        2 => {
            let tb = if r.flag()? {
                Some(time_bounds(r)?)
            } else {
                None
            };
            if r.flag()? {
                r.skip(8)?; // LedgerBounds
            }
            if r.flag()? {
                r.skip(8)?; // minSeqNum
            }
            r.skip(8 + 4)?; // minSeqAge, minSeqLedgerGap
            let n = r.array_len(2, 4)?;
            for _ in 0..n {
                signer_key(r)?;
            }
            Ok(tb)
        }
        _ => Err(XdrError::Invalid),
    }
}

fn memo<'a>(r: &mut Reader<'a>) -> XdrResult<Memo<'a>> {
    match r.i32()? {
        0 => Ok(Memo::None),
        1 => r.var_opaque(28).map(Memo::Text),
        2 => r.u64().map(Memo::Id),
        3 => r.fixed().map(Memo::Hash),
        4 => r.fixed().map(Memo::Return),
        _ => Err(XdrError::Invalid),
    }
}

const MAX_PREDICATE_DEPTH: u32 = 4;

fn claim_predicate(r: &mut Reader, depth: u32) -> XdrResult<()> {
    if depth > MAX_PREDICATE_DEPTH {
        return Err(XdrError::Invalid);
    }
    match r.i32()? {
        0 => Ok(()),
        1 | 2 => {
            let n = r.array_len(2, 4)?;
            for _ in 0..n {
                claim_predicate(r, depth + 1)?;
            }
            Ok(())
        }
        3 => {
            if r.flag()? {
                claim_predicate(r, depth + 1)?;
            }
            Ok(())
        }
        4 | 5 => r.skip(8),
        _ => Err(XdrError::Invalid),
    }
}

fn path(r: &mut Reader) -> XdrResult<()> {
    let n = r.array_len(5, 4)?;
    for _ in 0..n {
        skip_asset(r)?;
    }
    Ok(())
}

fn optional_u32(r: &mut Reader) -> XdrResult<()> {
    if r.flag()? {
        r.skip(4)?;
    }
    Ok(())
}

fn operation_body(r: &mut Reader, op_type: i32) -> XdrResult<OpBody> {
    let other = OpBody::Other {
        op_type: op_type as u32,
    };
    let body = match op_type {
        0 => OpBody::CreateAccount {
            destination: account_id(r)?,
            starting_balance: r.i64()?,
        },
        1 => OpBody::Payment {
            destination: muxed_account(r)?,
            asset: asset(r)?,
            amount: r.i64()?,
        },
        2 | 13 => {
            let send_asset = asset(r)?;
            let send = r.i64()?;
            let destination = muxed_account(r)?;
            let dest_asset = asset(r)?;
            let dest = r.i64()?;
            path(r)?;
            if op_type == 2 {
                OpBody::PathPaymentStrictReceive {
                    send_asset,
                    send_max: send,
                    destination,
                    dest_asset,
                    dest_amount: dest,
                }
            } else {
                OpBody::PathPaymentStrictSend {
                    send_asset,
                    send_amount: send,
                    destination,
                    dest_asset,
                    dest_min: dest,
                }
            }
        }
        3 | 12 => {
            skip_asset(r)?;
            skip_asset(r)?;
            r.skip(8)?;
            price(r)?;
            r.skip(8)?;
            other
        }
        4 => {
            skip_asset(r)?;
            skip_asset(r)?;
            r.skip(8)?;
            price(r)?;
            other
        }
        5 => {
            if r.flag()? {
                skip_account_id(r)?;
            }
            for _ in 0..6 {
                optional_u32(r)?;
            }
            if r.flag()? {
                r.var_opaque(32)?;
            }
            if r.flag()? {
                signer_key(r)?;
                r.skip(4)?;
            }
            other
        }
        6 => {
            match r.i32()? {
                0 => {}
                1 => r.skip(4 + 36)?,
                2 => r.skip(12 + 36)?,
                3 => {
                    if r.i32()? != 0 {
                        return Err(XdrError::Invalid);
                    }
                    skip_asset(r)?;
                    skip_asset(r)?;
                    r.skip(4)?;
                }
                _ => return Err(XdrError::Invalid),
            }
            r.skip(8)?;
            other
        }
        7 => {
            skip_account_id(r)?;
            match r.i32()? {
                1 => r.skip(4)?,
                2 => r.skip(12)?,
                _ => return Err(XdrError::Invalid),
            }
            r.skip(4)?;
            other
        }
        8 => OpBody::AccountMerge {
            destination: muxed_account(r)?,
        },
        9 | 17 => other,
        10 => {
            r.var_opaque(64)?;
            if r.flag()? {
                r.var_opaque(64)?;
            }
            other
        }
        11 => {
            r.skip(8)?;
            other
        }
        14 => {
            skip_asset(r)?;
            r.skip(8)?;
            let n = r.array_len(10, 4)?;
            for _ in 0..n {
                if r.i32()? != 0 {
                    return Err(XdrError::Invalid);
                }
                skip_account_id(r)?;
                claim_predicate(r, 1)?;
            }
            other
        }
        15 | 20 => {
            claimable_balance_id(r)?;
            other
        }
        16 => {
            skip_account_id(r)?;
            other
        }
        19 => {
            skip_asset(r)?;
            muxed_account(r)?;
            r.skip(8)?;
            other
        }
        21 => {
            skip_account_id(r)?;
            skip_asset(r)?;
            r.skip(8)?;
            other
        }
        22 => {
            r.skip(32 + 8 + 8)?;
            price(r)?;
            price(r)?;
            other
        }
        23 => {
            r.skip(32 + 8 + 8 + 8)?;
            other
        }
        18 | 24..=26 => return Err(XdrError::Unsupported),
        _ => return Err(XdrError::Invalid),
    };
    Ok(body)
}

fn operations(r: &mut Reader) -> XdrResult<Vec<Operation>> {
    let n = r.array_len(100, 8)?;
    let mut ops = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let source = if r.flag()? {
            Some(muxed_account(r)?)
        } else {
            None
        };
        let op_type = r.i32()?;
        ops.push(Operation {
            source,
            body: operation_body(r, op_type)?,
        });
    }
    Ok(ops)
}

/// `Transaction` (v1). Soroban transactions (ext v1) are unsupported.
fn transaction<'a>(r: &mut Reader<'a>) -> XdrResult<Transaction<'a>> {
    let source = muxed_account(r)?;
    let fee = r.u32()?;
    let seq_num = r.i64()?;
    let time_bounds = preconditions(r)?;
    let memo = memo(r)?;
    let operations = operations(r)?;
    match r.i32()? {
        0 => {}
        1 => return Err(XdrError::Unsupported),
        _ => return Err(XdrError::Invalid),
    }
    Ok(Transaction {
        source,
        fee,
        seq_num,
        time_bounds,
        memo,
        operations,
    })
}

/// `TransactionV0`, whose v1 encoding is `KEY_TYPE_ED25519 || v0 bytes`.
fn transaction_v0<'a>(r: &mut Reader<'a>) -> XdrResult<Transaction<'a>> {
    let key = r.fixed()?;
    let fee = r.u32()?;
    let seq_num = r.i64()?;
    let time_bounds = if r.flag()? {
        Some(time_bounds(r)?)
    } else {
        None
    };
    let memo = memo(r)?;
    let operations = operations(r)?;
    r.empty_ext()?;
    Ok(Transaction {
        source: Muxed { key, id: None },
        fee,
        seq_num,
        time_bounds,
        memo,
        operations,
    })
}

fn signatures(r: &mut Reader) -> XdrResult<()> {
    let n = r.array_len(20, 8)?;
    for _ in 0..n {
        r.skip(4)?; // hint
        r.var_opaque(64)?;
    }
    Ok(())
}

pub fn parse_envelope(buf: &[u8]) -> XdrResult<Envelope<'_>> {
    let mut r = Reader::new(buf);
    let envelope_type = r.i32()?;
    let start = r.position();
    let envelope = match envelope_type {
        ENVELOPE_TYPE_TX_V0 | ENVELOPE_TYPE_TX => {
            let tx = if envelope_type == ENVELOPE_TYPE_TX {
                transaction(&mut r)?
            } else {
                transaction_v0(&mut r)?
            };
            let hashed_bytes = &buf[start..r.position()];
            signatures(&mut r)?;
            Envelope {
                hash_envelope_type: ENVELOPE_TYPE_TX,
                v0_prefix: envelope_type == ENVELOPE_TYPE_TX_V0,
                hashed_bytes,
                fee_bump: None,
                tx,
            }
        }
        ENVELOPE_TYPE_TX_FEE_BUMP => {
            let fee_source = muxed_account(&mut r)?;
            let fee = r.i64()?;
            if r.i32()? != ENVELOPE_TYPE_TX {
                return Err(XdrError::Invalid);
            }
            let tx = transaction(&mut r)?;
            signatures(&mut r)?;
            r.empty_ext()?;
            let hashed_bytes = &buf[start..r.position()];
            signatures(&mut r)?;
            Envelope {
                hash_envelope_type: ENVELOPE_TYPE_TX_FEE_BUMP,
                v0_prefix: false,
                hashed_bytes,
                fee_bump: Some(FeeBump { fee_source, fee }),
                tx,
            }
        }
        _ => return Err(XdrError::Invalid),
    };
    r.finish()?;
    Ok(envelope)
}
