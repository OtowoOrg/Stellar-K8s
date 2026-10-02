//! Mempool parser for streaming mempool metrics.

use serde::{Deserialize, Serialize};

/// Representation of a pending transaction in the mempool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MempoolTx {
    /// Transaction hash (hex string).
    pub tx_hash: String,
    /// Size of the transaction in bytes.
    pub size_bytes: u64,
    /// Fee bid in stroops (or appropriate unit).
    pub fee_bid: i64,
    /// Optional additional metadata.
    pub metadata: Option<String>,
}

/// Simple line‑based parser.
///
/// Expected input format (CSV):
/// `tx_hash,size_bytes,fee_bid[,metadata]`
/// Whitespace is trimmed. Malformed lines return `None`.
pub fn parse_mempool_line(line: &str) -> Option<MempoolTx> {
    let parts: Vec<&str> = line.split(',').map(str::trim).collect();
    if parts.len() < 3 {
        return None;
    }
    let tx_hash = parts[0].to_string();
    let size_bytes = parts[1].parse::<u64>().ok()?;
    let fee_bid = parts[2].parse::<i64>().ok()?;
    let metadata = if parts.len() > 3 {
        Some(parts[3..].join(","))
    } else {
        None
    };
    Some(MempoolTx {
        tx_hash,
        size_bytes,
        fee_bid,
        metadata,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_line() {
        let line = "abcd1234,512,2000,extra";
        let tx = parse_mempool_line(line).expect("should parse");
        assert_eq!(tx.tx_hash, "abcd1234");
        assert_eq!(tx.size_bytes, 512);
        assert_eq!(tx.fee_bid, 2000);
        assert_eq!(tx.metadata.unwrap(), "extra");
    }

    #[test]
    fn test_parse_minimal_line() {
        let line = "ef01,256,1500";
        let tx = parse_mempool_line(line).expect("should parse");
        assert_eq!(tx.tx_hash, "ef01");
        assert_eq!(tx.size_bytes, 256);
        assert_eq!(tx.fee_bid, 1500);
        assert!(tx.metadata.is_none());
    }

    #[test]
    fn test_parse_invalid_line() {
        assert!(parse_mempool_line("bad,line").is_none());
    }
}
