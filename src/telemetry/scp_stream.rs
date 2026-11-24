use std::sync::Arc;

use serde::Serialize;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::kafka::KafkaProducer;

#[derive(Debug, Clone, Serialize)]
pub struct ScpMessage {
    pub ledger_seq: u64,
    pub node_id: String,
    pub quorum_set_hash: [u8; 32],
    pub slot_index: u64,
    pub message_type: String,
    pub payload: Vec<u8>,
}

impl ScpMessage {
    pub fn partition_key(&self) -> Vec<u8> {
        self.quorum_set_hash.to_vec()
    }
}

pub async fn run(
    producer: Arc<KafkaProducer>,
    mut rx: mpsc::Receiver<ScpMessage>,
) {
    info!("SCP stream processor started");
    while let Some(msg) = rx.recv().await {
        let key = msg.partition_key();
        let payload = match serde_json::to_vec(&msg) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to serialize SCP message: {e}");
                continue;
            }
        };
        if let Err(e) = producer.send(&key, &payload).await {
            error!("Failed to send SCP message to Kafka: {e}");
        }
    }
    info!("SCP stream processor stopped");
}
