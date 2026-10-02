//! Recursive Length Prefix (RLP) decoder and encoder, bounded for on-chain use.
//!
//! RLP is Ethereum's canonical serialisation format and is the only encoding
//! accepted for Merkle Patricia Trie (MPT) nodes. It encodes two shapes:
//!
//! * **byte string** — a length-prefixed blob, e.g. `0x83 0x01 0x02 0x03` is
//!   the three bytes `0x010203`.
//! * **list** — a length-prefixed sequence of nested items.
//!
//! # Why the canonical checks matter
//!
//! RLP is only *canonically* encoded if short forms are used whenever the
//! payload permits and length fields carry no leading zeros. Without those
//! checks the same logical node has many byte-distinct encodings, each with a
//! different Keccak-256 hash. A verifier that hashes the raw bytes supplied by
//! an untrusted prover could then be shown one encoding and be made to
//! reconstruct a *different* hash, so [`decode`] rejects every non-canonical
//! form. Re-encoding a decoded value with [`encode`] always reproduces the
//! original bytes.
//!
//! # Bounded memory
//!
//! Every decode is capped by [`Limits`]. The decoder refuses oversized items,
//! oversized item counts, oversized payloads and excessive nesting, so a
//! malicious proof cannot exhaust the Soroban budget. Nothing is allocated
//! before its length prefix has been validated and charged against the budget.

use alloc::vec::Vec;

/// A decoded RLP item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RlpItem {
    /// A byte string.
    Bytes(Vec<u8>),
    /// A list of nested items.
    List(Vec<RlpItem>),
}

impl RlpItem {
    /// Returns the byte-string payload, or `None` for a list.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            RlpItem::Bytes(b) => Some(b),
            RlpItem::List(_) => None,
        }
    }

    /// Returns the list payload, or `None` for a byte string.
    pub fn as_list(&self) -> Option<&[RlpItem]> {
        match self {
            RlpItem::List(l) => Some(l),
            RlpItem::Bytes(_) => None,
        }
    }
}

/// Reasons an RLP payload can be rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RlpError {
    /// Input ended before the item was complete.
    UnexpectedEof,
    /// A reserved prefix byte was encountered.
    InvalidPrefix,
    /// The payload length is larger than the configured budget.
    PayloadTooLarge,
    /// The list has more items than [`Limits::max_list_items`].
    TooManyItems,
    /// Nesting is deeper than [`Limits::max_depth`].
    DepthExceeded,
    /// A byte string is longer than [`Limits::max_item_bytes`].
    ItemTooLarge,
    /// A long-form encoding was used where the short form would fit.
    NonCanonicalShortForm,
    /// A length field carried a leading zero byte.
    NonCanonicalLength,
    /// A length field was wider than 8 bytes.
    LengthFieldTooWide,
    /// A single byte below `0x80` was encoded as `0x81 0xNN`.
    NonCanonicalSingleByte,
    /// Bytes remained after a complete top-level item was decoded.
    TrailingBytes,
}

/// Decode bounds. Callers that trust the Soroban budget should keep these
/// tight; the defaults are sized for real mainnet account and storage proofs.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Maximum number of items decoded across the whole payload.
    pub max_items: usize,
    /// Maximum byte length of any single byte string.
    pub max_item_bytes: usize,
    /// Maximum nesting depth of lists.
    pub max_depth: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_items: 64,
            max_item_bytes: 1024,
            max_depth: 4,
        }
    }
}

/// Decodes a complete RLP payload, rejecting trailing bytes.
pub fn decode(input: &[u8], limits: &Limits) -> Result<RlpItem, RlpError> {
    let mut d = Decoder {
        input,
        pos: 0,
        limits: *limits,
        items: 0,
    };
    let item = d.item(0)?;
    if d.pos != input.len() {
        return Err(RlpError::TrailingBytes);
    }
    Ok(item)
}

struct Decoder<'a> {
    input: &'a [u8],
    pos: usize,
    limits: Limits,
    items: usize,
}

impl<'a> Decoder<'a> {
    /// Charges one item and one unit of nesting against the budget.
    fn charge(&mut self, depth: usize) -> Result<(), RlpError> {
        if depth > self.limits.max_depth {
            return Err(RlpError::DepthExceeded);
        }
        self.items += 1;
        if self.items > self.limits.max_items {
            return Err(RlpError::TooManyItems);
        }
        Ok(())
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], RlpError> {
        let end = self.pos.checked_add(n).ok_or(RlpError::PayloadTooLarge)?;
        if end > self.input.len() {
            return Err(RlpError::UnexpectedEof);
        }
        let out = &self.input[self.pos..end];
        self.pos = end;
        Ok(out)
    }

    /// Reads a big-endian length field of `len_of_len` bytes, rejecting
    /// non-minimal encodings and absurd widths.
    fn read_length(&mut self, len_of_len: usize) -> Result<usize, RlpError> {
        if len_of_len > 8 {
            return Err(RlpError::LengthFieldTooWide);
        }
        let raw = self.take(len_of_len)?;
        if raw[0] == 0 {
            return Err(RlpError::NonCanonicalLength);
        }
        let mut len: usize = 0;
        for byte in raw {
            len = len
                .checked_mul(256)
                .and_then(|v| v.checked_add(*byte as usize))
                .ok_or(RlpError::PayloadTooLarge)?;
        }
        Ok(len)
    }

    /// Decodes one item at the given nesting depth.
    fn item(&mut self, depth: usize) -> Result<RlpItem, RlpError> {
        self.charge(depth)?;
        let prefix = *self.take(1)?.first().ok_or(RlpError::UnexpectedEof)?;

        match prefix {
            // 0x00..=0x7f encodes itself.
            0x00..=0x7f => Ok(RlpItem::Bytes(alloc::vec![prefix])),

            // 0x80..=0xb7: short string.
            0x80..=0xb7 => {
                let len = (prefix - 0x80) as usize;
                self.string(len)
            }

            // 0xb8..=0xbf: long string.
            0xb8..=0xbf => {
                let len = self.read_length((prefix - 0xb7) as usize)?;
                if len < 56 {
                    return Err(RlpError::NonCanonicalShortForm);
                }
                self.string(len)
            }

            // 0xc0..=0xf7: short list.
            0xc0..=0xf7 => self.list((prefix - 0xc0) as usize, depth),

            // 0xf8..=0xff: long list.
            0xf8..=0xff => {
                let len = self.read_length((prefix - 0xf7) as usize)?;
                if len < 56 {
                    return Err(RlpError::NonCanonicalShortForm);
                }
                self.list(len, depth)
            }
        }
    }

    fn string(&mut self, len: usize) -> Result<RlpItem, RlpError> {
        if len > self.limits.max_item_bytes {
            return Err(RlpError::ItemTooLarge);
        }
        // A single byte below 0x80 must be encoded as itself, not 0x81 NN.
        if len == 1 && self.input[self.pos] < 0x80 {
            return Err(RlpError::NonCanonicalSingleByte);
        }
        Ok(RlpItem::Bytes(self.take(len)?.to_vec()))
    }

    fn list(&mut self, payload_len: usize, depth: usize) -> Result<RlpItem, RlpError> {
        // Reserve the whole list payload against the byte budget before
        // decoding any child, so a lying length prefix cannot slip through.
        let body = self.take(payload_len)?;
        let mut inner = Decoder {
            input: body,
            pos: 0,
            limits: self.limits,
            items: 0,
        };
        let mut out = Vec::new();
        while inner.pos < body.len() {
            out.push(inner.item(depth + 1)?);
        }
        // Fold the child's item count into our own budget.
        self.items = self.items.saturating_add(inner.items);
        if self.items > self.limits.max_items {
            return Err(RlpError::TooManyItems);
        }
        Ok(RlpItem::List(out))
    }
}

/// Encodes an item in canonical form.
pub fn encode(item: &RlpItem) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(item, &mut out);
    out
}

fn encode_into(item: &RlpItem, out: &mut Vec<u8>) {
    match item {
        RlpItem::Bytes(b) => {
            if b.len() == 1 && b[0] < 0x80 {
                out.push(b[0]);
            } else {
                encode_header(b.len(), 0x80, 0xb7, out);
                out.extend_from_slice(b);
            }
        }
        RlpItem::List(items) => {
            let mut body = Vec::new();
            for child in items {
                encode_into(child, &mut body);
            }
            encode_header(body.len(), 0xc0, 0xf7, out);
            out.extend_from_slice(&body);
        }
    }
}

fn encode_header(len: usize, short_base: u8, long_base: u8, out: &mut Vec<u8>) {
    if len < 56 {
        out.push(short_base + len as u8);
    } else {
        let be = len.to_be_bytes();
        let start = be.iter().position(|b| *b != 0).unwrap_or(be.len() - 1);
        let width = be.len() - start;
        out.push(long_base + width as u8);
        out.extend_from_slice(&be[start..]);
    }
}

/// Encodes a single byte string.
pub fn encode_bytes(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    encode_into(&RlpItem::Bytes(bytes.to_vec()), &mut out);
    out
}

/// Decodes a big-endian integer, rejecting non-minimal (zero-padded) encodings.
///
/// Ethereum stores scalars as minimal big-endian byte strings, so a leading
/// zero byte means the input is not a valid canonical scalar.
pub fn scalar_to_u64(bytes: &[u8]) -> Result<u64, RlpError> {
    if bytes.len() > 8 {
        return Err(RlpError::ItemTooLarge);
    }
    if bytes.len() > 1 && bytes[0] == 0 {
        return Err(RlpError::NonCanonicalLength);
    }
    let mut value: u64 = 0;
    for b in bytes {
        value = (value << 8) | *b as u64;
    }
    Ok(value)
}

/// Left-pads a big-endian integer to `len` bytes, or errors on overflow.
pub fn scalar_to_padded(bytes: &[u8], len: usize) -> Result<Vec<u8>, RlpError> {
    if bytes.len() > len {
        return Err(RlpError::ItemTooLarge);
    }
    let mut out = alloc::vec![0u8; len - bytes.len()];
    out.extend_from_slice(bytes);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn limits() -> Limits {
        Limits::default()
    }

    #[test]
    fn decodes_single_byte() {
        // 0x7f is the largest self-encoding byte.
        assert_eq!(
            decode(&[0x7f], &limits()).unwrap(),
            RlpItem::Bytes(vec![0x7f])
        );
        assert_eq!(
            decode(&[0x00], &limits()).unwrap(),
            RlpItem::Bytes(vec![0x00])
        );
    }

    #[test]
    fn decodes_short_and_long_strings() {
        assert_eq!(
            decode(&[0x83, 0x01, 0x02, 0x03], &limits()).unwrap(),
            RlpItem::Bytes(vec![1, 2, 3])
        );
        let long = [0xb8, 0x3c];
        let payload: alloc::vec::Vec<u8> = (0u16..60).map(|i| i as u8).collect();
        let mut input = long.to_vec();
        input.extend_from_slice(&payload);
        assert_eq!(decode(&input, &limits()).unwrap(), RlpItem::Bytes(payload));
    }

    #[test]
    fn decodes_nested_lists() {
        // ["cat", ["dog"]] -- body is 4 + 5 = 9 bytes, so 0xc9.
        let input = [0xc9, 0x83, b'c', b'a', b't', 0xc4, 0x83, b'd', b'o', b'g'];
        let item = decode(&input, &limits()).unwrap();
        let list = item.as_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].as_bytes().unwrap(), b"cat");
        assert_eq!(list[1].as_list().unwrap()[0].as_bytes().unwrap(), b"dog");
    }

    #[test]
    fn decodes_empty_string_and_empty_list() {
        assert_eq!(decode(&[0x80], &limits()).unwrap(), RlpItem::Bytes(vec![]));
        assert_eq!(decode(&[0xc0], &limits()).unwrap(), RlpItem::List(vec![]));
    }

    #[test]
    fn rejects_truncated_payload() {
        assert_eq!(
            decode(&[0x83, 0x01], &limits()),
            Err(RlpError::UnexpectedEof)
        );
        assert_eq!(
            decode(&[0xb8, 0x40], &limits()),
            Err(RlpError::UnexpectedEof)
        );
    }

    #[test]
    fn rejects_trailing_bytes() {
        assert_eq!(
            decode(&[0x00, 0x00], &limits()),
            Err(RlpError::TrailingBytes)
        );
    }

    #[test]
    fn rejects_non_canonical_single_byte() {
        // 0x01 must not be encoded as 0x81 0x01.
        assert_eq!(
            decode(&[0x81, 0x01], &limits()),
            Err(RlpError::NonCanonicalSingleByte)
        );
    }

    #[test]
    fn rejects_non_canonical_short_form() {
        // A 55-byte string must use the short form, not 0xb8 0x37.
        let mut input = vec![0xb8, 0x37];
        input.extend(core::iter::repeat_n(0x01, 0x37));
        assert_eq!(
            decode(&input, &limits()),
            Err(RlpError::NonCanonicalShortForm)
        );
    }

    #[test]
    fn rejects_length_with_leading_zero() {
        assert_eq!(
            decode(&[0xb8, 0x00, 0x40], &limits()),
            Err(RlpError::NonCanonicalLength)
        );
    }

    #[test]
    fn rejects_oversized_length_field() {
        // A 2-byte length field (0xb9) still has to be backed by real bytes;
        // claiming 257 without supplying them is a truncation, not a width
        // error. The width guard itself is unreachable through a prefix byte
        // (0xb8..=0xbf and 0xf8..=0xff top out at 8 bytes) and is kept purely
        // as defence in depth.
        assert_eq!(
            decode(&[0xb9, 0x01, 0x01], &limits()),
            Err(RlpError::UnexpectedEof)
        );
    }

    #[test]
    fn enforces_item_byte_budget() {
        // 16 bytes must use the short form (0x90); the long form would be
        // non-canonical and rejected before the budget is even consulted.
        let mut input = vec![0x90];
        input.extend(core::iter::repeat_n(0x00, 16));
        assert!(decode(&input, &limits()).is_ok());

        let strict = Limits {
            max_items: 64,
            max_item_bytes: 8,
            max_depth: 4,
        };
        assert_eq!(decode(&input, &strict), Err(RlpError::ItemTooLarge));
    }

    #[test]
    fn enforces_item_count_budget() {
        // A 17-item branch is the largest legal trie node: short list form
        // 0xc0 + 17 = 0xd1.
        let mut full = vec![0xd1];
        full.extend(core::iter::repeat_n(0x80, 17));
        assert!(decode(&full, &limits()).is_ok());

        let strict = Limits {
            max_items: 8,
            max_item_bytes: 1024,
            max_depth: 4,
        };
        assert_eq!(decode(&full, &strict), Err(RlpError::TooManyItems));
    }

    #[test]
    fn enforces_depth_budget() {
        // Four nested single-item lists. Built with the encoder so the depth
        // count is unambiguous. Four is the default budget: the deepest shape
        // MPT ever produces is a node list wrapping an account list, i.e. 2.
        let mut nested = RlpItem::List(vec![]);
        for _ in 0..4 {
            nested = RlpItem::List(vec![nested]);
        }
        let input = encode(&nested);
        assert_eq!(decode(&input, &limits()).map(|_| ()), Ok(()));

        // A depth budget of 2 must refuse the same payload.
        let strict = Limits {
            max_items: 64,
            max_item_bytes: 1024,
            max_depth: 2,
        };
        assert_eq!(decode(&input, &strict), Err(RlpError::DepthExceeded));
    }

    #[test]
    fn encode_round_trips_canonical_forms() {
        let cases: alloc::vec::Vec<&[u8]> = alloc::vec![
            &[0x00],
            &[0x7f],
            &[0x80],
            &[0x83, 0x01, 0x02, 0x03],
            &[0xc0],
            &[0xc9, 0x83, b'c', b'a', b't', 0xc4, 0x83, b'd', b'o', b'g'],
        ];
        for case in cases {
            let decoded = decode(case, &limits()).unwrap();
            assert_eq!(encode(&decoded), case.to_vec());
        }
    }

    #[test]
    fn encodes_long_payloads_canonically() {
        for len in [56usize, 57, 256, 1024] {
            let payload: alloc::vec::Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            let encoded = encode_bytes(&payload);
            assert_eq!(
                decode(&encoded, &limits()).unwrap(),
                RlpItem::Bytes(payload),
                "round-trip failed at len {len}"
            );
        }
    }

    #[test]
    fn scalar_rejects_zero_padding() {
        assert_eq!(
            scalar_to_u64(&[0x00, 0x01]),
            Err(RlpError::NonCanonicalLength)
        );
        assert_eq!(scalar_to_u64(&[0x01]), Ok(1));
        assert_eq!(scalar_to_u64(&[]), Ok(0));
        assert_eq!(scalar_to_u64(&[0x01, 0x00]), Ok(256));
        assert_eq!(scalar_to_u64(&[0xff; 9]), Err(RlpError::ItemTooLarge));
    }

    #[test]
    fn scalar_pads_to_width() {
        assert_eq!(scalar_to_padded(&[0x01], 4).unwrap(), vec![0, 0, 0, 1]);
        assert_eq!(scalar_to_padded(&[], 2).unwrap(), vec![0, 0]);
        assert_eq!(scalar_to_padded(&[1, 2, 3], 2), Err(RlpError::ItemTooLarge));
    }
}
