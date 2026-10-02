//! Account and storage-slot decoding for Ethereum state proofs.
//!
//! Verifying a trie path is only half the job: a bridge also needs to read the
//! leaf value that the path leads to. This module decodes the two value shapes
//! Ethereum stores:
//!
//! * **Account** — `[nonce, balance, storageRoot, codeHash]`, an RLP list of
//!   four byte strings. `nonce` and `balance` are minimal big-endian integers;
//!   the other two are 32-byte hashes.
//! * **Storage slot** — a single minimal big-endian integer. A zero value is
//!   stored as the empty string and means "this mapping entry is unset".

use alloc::vec::Vec;

use crate::rlp::{self, RlpError, RlpItem};

/// Length of an Ethereum address in bytes.
pub const ADDRESS_LENGTH: usize = 20;
/// Length of a hash in bytes.
pub const HASH_LENGTH: usize = 32;

/// Reasons a decoded value can be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The value was not well-formed RLP.
    Rlp(RlpError),
    /// The value had the wrong shape for its kind.
    MalformedAccount,
    /// The account field had the wrong byte length.
    InvalidHashLength,
    /// A scalar was longer than 32 bytes.
    ScalarTooLarge,
}

impl From<RlpError> for DecodeError {
    fn from(e: RlpError) -> Self {
        DecodeError::Rlp(e)
    }
}

/// A decoded Ethereum account leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    /// Transaction count.
    pub nonce: u64,
    /// Balance in wei as a minimal big-endian byte string.
    pub balance: Vec<u8>,
    /// Root of the account's storage trie.
    pub storage_root: [u8; 32],
    /// Keccak-256 of the deployed code, or the empty hash.
    pub code_hash: [u8; 32],
}

impl Account {
    /// True when the account carries no code, i.e. it is an externally owned
    /// account rather than a contract.
    pub fn is_eoa(&self) -> bool {
        self.code_hash == crate::trie::EMPTY_CODE_HASH
    }
}

/// Decodes a 32-byte hash field.
fn decode_hash(bytes: &[u8]) -> Result<[u8; 32], DecodeError> {
    if bytes.len() != HASH_LENGTH {
        return Err(DecodeError::InvalidHashLength);
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(bytes);
    Ok(out)
}

/// Decodes an account leaf value.
///
/// The empty-account leaf (`RLP("")`) is a legacy encoding for an account with
/// zero nonce, zero balance and both roots empty; it is accepted and mapped to
/// the equivalent all-zero account.
pub fn decode_account(value: &[u8], limits: &rlp::Limits) -> Result<Account, DecodeError> {
    // The empty trie value means an empty account.
    if value.is_empty() {
        return Ok(Account {
            nonce: 0,
            balance: alloc::vec![0],
            storage_root: [0u8; 32],
            code_hash: [0u8; 32],
        });
    }

    let decoded = rlp::decode(value, limits)?;
    let items = decoded.as_list().ok_or(DecodeError::MalformedAccount)?;
    if items.len() != 4 {
        return Err(DecodeError::MalformedAccount);
    }

    let nonce_bytes = items[0].as_bytes().ok_or(DecodeError::MalformedAccount)?;
    let balance_bytes = items[1].as_bytes().ok_or(DecodeError::MalformedAccount)?;
    let storage_root_bytes = items[2].as_bytes().ok_or(DecodeError::MalformedAccount)?;
    let code_hash_bytes = items[3].as_bytes().ok_or(DecodeError::MalformedAccount)?;

    if balance_bytes.len() > HASH_LENGTH {
        return Err(DecodeError::ScalarTooLarge);
    }

    Ok(Account {
        nonce: rlp::scalar_to_u64(nonce_bytes)?,
        balance: balance_bytes.to_vec(),
        storage_root: decode_hash(storage_root_bytes)?,
        code_hash: decode_hash(code_hash_bytes)?,
    })
}

/// Decodes a storage-slot leaf value into a minimal big-endian integer.
///
/// An empty value means the entry is unset and decodes to zero, matching how
/// Ethereum omits zero-valued mapping entries from the trie.
pub fn decode_storage_value(value: &[u8], limits: &rlp::Limits) -> Result<Vec<u8>, DecodeError> {
    if value.is_empty() {
        return Ok(alloc::vec![0]);
    }
    let decoded = rlp::decode(value, limits)?;
    let bytes = decoded.as_bytes().ok_or(DecodeError::MalformedAccount)?;
    if bytes.len() > HASH_LENGTH {
        return Err(DecodeError::ScalarTooLarge);
    }
    Ok(bytes.to_vec())
}

/// Normalises an address to 20 bytes, rejecting anything else.
pub fn parse_address(address: &[u8]) -> Result<[u8; ADDRESS_LENGTH], DecodeError> {
    if address.len() != ADDRESS_LENGTH {
        return Err(DecodeError::InvalidHashLength);
    }
    let mut out = [0u8; ADDRESS_LENGTH];
    out.copy_from_slice(address);
    Ok(out)
}

/// Strips leading zero bytes, keeping at least one byte so zero encodes as
/// `0x00` rather than as the empty string (Ethereum's canonical form for the
/// integer zero is a single `0x00` byte).
fn minimal_be_bytes(value: &[u8]) -> Vec<u8> {
    let mut start = 0;
    while start + 1 < value.len() && value[start] == 0 {
        start += 1;
    }
    value[start..].to_vec()
}

/// Encodes an account leaf value, used to build fixtures and round-trip tests.
///
/// Scalars are emitted minimally: RLP is only canonical for integers without
/// leading zero bytes, and this encoder is held to the same standard the
/// decoder enforces.
pub fn encode_account(account: &Account) -> Vec<u8> {
    rlp::encode(&RlpItem::List(alloc::vec![
        RlpItem::Bytes(minimal_be_bytes(&account.nonce.to_be_bytes())),
        RlpItem::Bytes(minimal_be_bytes(&account.balance)),
        RlpItem::Bytes(account.storage_root.to_vec()),
        RlpItem::Bytes(account.code_hash.to_vec()),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn limits() -> rlp::Limits {
        rlp::Limits::default()
    }

    #[test]
    fn decodes_a_realistic_account() {
        // [1, 1024, storageRoot, codeHash]
        let value = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0x04, 0x00]),
            RlpItem::Bytes([0xaa; 32].to_vec()),
            RlpItem::Bytes([0xbb; 32].to_vec()),
        ]));
        let account = decode_account(&value, &limits()).unwrap();
        assert_eq!(account.nonce, 1);
        assert_eq!(account.balance, vec![0x04, 0x00]);
        assert_eq!(account.storage_root, [0xaa; 32]);
        assert_eq!(account.code_hash, [0xbb; 32]);
    }

    #[test]
    fn decodes_empty_account_leaf() {
        let account = decode_account(&[], &limits()).unwrap();
        assert_eq!(account.nonce, 0);
        assert_eq!(account.balance, vec![0]);
        assert_eq!(account.code_hash, [0u8; 32]);
    }

    #[test]
    fn rejects_wrong_field_count() {
        let value = rlp::encode(&RlpItem::List(vec![RlpItem::Bytes(vec![1])]));
        assert_eq!(
            decode_account(&value, &limits()),
            Err(DecodeError::MalformedAccount)
        );
    }

    #[test]
    fn rejects_short_hash_fields() {
        let value = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0x01; 31]),
            RlpItem::Bytes([0xbb; 32].to_vec()),
        ]));
        assert_eq!(
            decode_account(&value, &limits()),
            Err(DecodeError::InvalidHashLength)
        );
    }

    #[test]
    fn rejects_oversized_balance() {
        let value = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0xff; 33]),
            RlpItem::Bytes([0xaa; 32].to_vec()),
            RlpItem::Bytes([0xbb; 32].to_vec()),
        ]));
        assert_eq!(
            decode_account(&value, &limits()),
            Err(DecodeError::ScalarTooLarge)
        );
    }

    #[test]
    fn identifies_eoa_by_code_hash() {
        let value = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes([0xaa; 32].to_vec()),
            RlpItem::Bytes(crate::trie::EMPTY_CODE_HASH.to_vec()),
        ]));
        let account = decode_account(&value, &limits()).unwrap();
        assert!(account.is_eoa());

        let value = rlp::encode(&RlpItem::List(vec![
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes(vec![0x01]),
            RlpItem::Bytes([0xaa; 32].to_vec()),
            RlpItem::Bytes([0xcc; 32].to_vec()),
        ]));
        let account = decode_account(&value, &limits()).unwrap();
        assert!(!account.is_eoa());
    }

    #[test]
    fn account_encoding_round_trips() {
        let account = Account {
            nonce: 7,
            balance: vec![0x0d, 0xe0, 0xb6, 0xb3, 0xa7, 0x64, 0x00, 0x00],
            storage_root: [0x11; 32],
            code_hash: [0x22; 32],
        };
        let encoded = encode_account(&account);
        let decoded = decode_account(&encoded, &limits()).unwrap();
        assert_eq!(decoded, account);
    }

    #[test]
    fn decodes_storage_value() {
        let encoded = rlp::encode_bytes(&[0x01, 0x02]);
        assert_eq!(
            decode_storage_value(&encoded, &limits()).unwrap(),
            vec![0x01, 0x02]
        );
        // Empty value means an unset mapping entry.
        assert_eq!(
            decode_storage_value(&[], &limits()).unwrap(),
            Vec::<u8>::from([0])
        );
        assert_eq!(
            decode_storage_value(&rlp::encode_bytes(&[]), &limits()).unwrap(),
            Vec::<u8>::new()
        );
    }

    #[test]
    fn rejects_oversized_storage_value() {
        let encoded = rlp::encode_bytes(&[0xff; 33]);
        assert_eq!(
            decode_storage_value(&encoded, &limits()),
            Err(DecodeError::ScalarTooLarge)
        );
    }

    #[test]
    fn parses_addresses() {
        let mut raw = [0u8; 20];
        raw[0] = 0xde;
        assert_eq!(parse_address(&raw), Ok(raw));
        assert_eq!(
            parse_address(&[0u8; 19]),
            Err(DecodeError::InvalidHashLength)
        );
    }
}
