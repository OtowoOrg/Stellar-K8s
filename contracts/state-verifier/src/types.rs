//! Contract-facing types, storage keys, errors and events.

use soroban_sdk::{contracterror, contractevent, contracttype, Address, Bytes, BytesN, Vec};

pub(crate) const DAY_IN_LEDGERS: u32 = 17_280;
pub(crate) const INSTANCE_BUMP: u32 = 30 * DAY_IN_LEDGERS;
pub(crate) const INSTANCE_THRESHOLD: u32 = INSTANCE_BUMP - DAY_IN_LEDGERS;
pub(crate) const ENTRY_BUMP: u32 = 90 * DAY_IN_LEDGERS;
pub(crate) const ENTRY_THRESHOLD: u32 = ENTRY_BUMP - DAY_IN_LEDGERS;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VerifierError {
    /// Input is not a valid encoding of the expected XDR structure.
    MalformedXdr = 1,
    /// Valid envelope the decoder does not support (Soroban, RevokeSponsorship).
    UnsupportedTransaction = 2,
    NotRelayer = 3,
    /// The header's SCP value is signed by a node outside the trusted set.
    UntrustedValidator = 4,
    /// The header carries an unsigned (`STELLAR_VALUE_BASIC`) SCP value.
    UnsignedLedgerValue = 5,
    /// A different header is already recorded for this ledger sequence.
    LedgerConflict = 6,
    UnknownLedger = 7,
    /// The header does not hash to the child ledger's `previousLedgerHash`.
    NotAncestor = 8,
    /// The result set does not hash to the header's `txSetResultHash`.
    ResultsHashMismatch = 9,
    TooManyTransactions = 10,
    ResultsNotCommitted = 11,
    IndexOutOfRange = 12,
    InvalidProofLength = 13,
    /// The inclusion proof does not reproduce the committed root.
    NotIncluded = 14,
}

/// A verified ledger header.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerRecord {
    pub ledger_seq: u32,
    pub hash: BytesN<32>,
    pub previous_ledger_hash: BytesN<32>,
    pub tx_set_hash: BytesN<32>,
    pub tx_set_result_hash: BytesN<32>,
    pub close_time: u64,
    pub ledger_version: u32,
    /// `true` when anchored by a relayer, `false` when proven as an ancestor
    /// of an already verified ledger.
    pub anchored: bool,
}

/// Merkle commitment over a ledger's `(tx_hash, result_code)` pairs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultsCommitment {
    pub root: BytesN<32>,
    pub tx_count: u32,
    pub depth: u32,
}

/// Proof outcome returned to callers.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTransaction {
    pub ledger_seq: u32,
    pub ledger_hash: BytesN<32>,
    pub close_time: u64,
    pub tx_hash: BytesN<32>,
    pub result_code: i32,
    /// `txSUCCESS` or `txFEE_BUMP_INNER_SUCCESS`.
    pub successful: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MuxedAccount {
    pub key: BytesN<32>,
    pub id: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetCode {
    /// Asset code as encoded (4 or 12 bytes, NUL padded).
    pub code: Bytes,
    pub issuer: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asset {
    Native,
    CreditAlphanum4(AssetCode),
    CreditAlphanum12(AssetCode),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Memo {
    None,
    Text(Bytes),
    Id(u64),
    Hash(BytesN<32>),
    Return(BytesN<32>),
}

/// Time bounds; `{0, 0}` (Stellar's "no bounds") when the transaction has
/// none, and `max_time == 0` means no upper bound.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimeBounds {
    pub min_time: u64,
    pub max_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateAccountOp {
    pub destination: BytesN<32>,
    pub starting_balance: i64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentOp {
    pub destination: MuxedAccount,
    pub asset: Asset,
    pub amount: i64,
}

/// Path payment. For strict-receive, `send_amount` is `sendMax` and
/// `dest_amount` is exact; for strict-send, `send_amount` is exact and
/// `dest_amount` is `destMin`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathPaymentOp {
    pub send_asset: Asset,
    pub send_amount: i64,
    pub destination: MuxedAccount,
    pub dest_asset: Asset,
    pub dest_amount: i64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationBody {
    CreateAccount(CreateAccountOp),
    Payment(PaymentOp),
    PathPaymentStrictReceive(PathPaymentOp),
    PathPaymentStrictSend(PathPaymentOp),
    AccountMerge(MuxedAccount),
    /// Supported operation of the given `OperationType` whose fields are
    /// validated but not surfaced.
    Other(u32),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationSource {
    /// The operation runs as the transaction source account.
    Transaction,
    Account(MuxedAccount),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Operation {
    pub source: OperationSource,
    pub body: OperationBody,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedTransaction {
    pub source: MuxedAccount,
    /// Whether the envelope is a fee bump.
    pub fee_bump: bool,
    /// Account paying the fee: the fee-bump payer, otherwise `source`.
    pub fee_source: MuxedAccount,
    /// Maximum fee (the outer fee for fee bumps).
    pub fee: i64,
    pub seq_num: i64,
    pub time_bounds: TimeBounds,
    pub memo: Memo,
    pub operations: Vec<Operation>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedEnvelope {
    pub inclusion: VerifiedTransaction,
    pub transaction: DecodedTransaction,
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum DataKey {
    Admin,
    NetworkId,
    Relayer(Address),
    Validator(BytesN<32>),
    Ledger(u32),
    Results(u32),
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LedgerCheckpointed {
    #[topic]
    pub ledger_seq: u32,
    pub ledger_hash: BytesN<32>,
    pub relayer: Address,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AncestorProven {
    #[topic]
    pub ledger_seq: u32,
    pub ledger_hash: BytesN<32>,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResultsCommitted {
    #[topic]
    pub ledger_seq: u32,
    pub root: BytesN<32>,
    pub tx_count: u32,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayerSet {
    #[topic]
    pub relayer: Address,
    pub enabled: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorSet {
    #[topic]
    pub node_id: BytesN<32>,
    pub trusted: bool,
}

#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChanged {
    pub admin: Address,
}
