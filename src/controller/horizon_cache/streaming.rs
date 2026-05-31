//! Streaming and compression for Horizon query responses.

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Compressed response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressedResponse {
    pub original_size: usize,
    pub compressed_size: usize,
    pub compression_ratio: f64,
    pub data: Vec<u8>,
    pub encoding: String,
}

/// Streams and optionally compresses query responses.
pub struct ResponseStreamer {
    compression_enabled: bool,
}

impl ResponseStreamer {
    pub fn new(compression_enabled: bool) -> Self {
        Self {
            compression_enabled,
        }
    }

    /// Compress response data using gzip.
    pub fn compress(&self, data: &[u8]) -> CompressedResponse {
        if !self.compression_enabled || data.len() < 128 {
            return CompressedResponse {
                original_size: data.len(),
                compressed_size: data.len(),
                compression_ratio: 1.0,
                data: data.to_vec(),
                encoding: "identity".to_string(),
            };
        }

        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(data).unwrap();
        let compressed = encoder.finish().unwrap();

        CompressedResponse {
            original_size: data.len(),
            compressed_size: compressed.len(),
            compression_ratio: data.len() as f64 / compressed.len().max(1) as f64,
            data: compressed,
            encoding: "gzip".to_string(),
        }
    }

    /// Decompress gzip response data.
    pub fn decompress(response: &CompressedResponse) -> Vec<u8> {
        if response.encoding == "identity" {
            return response.data.clone();
        }

        let mut decoder = GzDecoder::new(&response.data[..]);
        let mut decompressed = Vec::new();
        decoder.read_to_end(&mut decompressed).unwrap();
        decompressed
    }

    /// Chunk data for streaming delivery.
    pub fn chunk(data: &[u8], chunk_size: usize) -> impl Iterator<Item = &[u8]> {
        data.chunks(chunk_size.max(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_decompress_roundtrip() {
        let streamer = ResponseStreamer::new(true);
        let original = vec![0u8; 1024];
        let compressed = streamer.compress(&original);
        assert!(compressed.compression_ratio > 1.0);
        let restored = ResponseStreamer::decompress(&compressed);
        assert_eq!(restored, original);
    }

    #[test]
    fn small_payload_not_compressed() {
        let streamer = ResponseStreamer::new(true);
        let data = b"small";
        let response = streamer.compress(data);
        assert_eq!(response.encoding, "identity");
    }

    #[test]
    fn chunk_streaming() {
        let data = b"hello world streaming test";
        let chunks: Vec<_> = ResponseStreamer::chunk(data, 5).collect();
        assert!(chunks.len() > 1);
    }
}
