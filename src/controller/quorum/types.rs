//! Core data structures for SCP state and quorum configuration

use serde::{Deserialize, Serialize};

/// SCP state retrieved from Stellar Core
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScpState {
    pub node_id: String,
    pub quorum_set: QuorumSetInfo,
    pub ballot_state: BallotState,
    pub nomination_state: NominationState,
}

/// Quorum set configuration
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct QuorumSetInfo {
    /// Threshold - number of validators that must agree
    #[serde(rename = "t")]
    pub threshold: u32,

    /// List of validator public keys
    #[serde(rename = "v", default)]
    pub validators: Vec<String>,

    /// Nested inner quorum sets
    #[serde(rename = "innerSets", default)]
    pub inner_sets: Vec<InnerQuorumSetInfo>,
}

/// Inner quorum set (nested structure)
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InnerQuorumSetInfo {
    /// Threshold for this inner set
    #[serde(rename = "t")]
    pub threshold: u32,

    /// Validators in this inner set
    #[serde(rename = "v", default)]
    pub validators: Vec<String>,
}

/// Ballot state from SCP
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct BallotState {
    /// Current SCP phase (PREPARE, CONFIRM, EXTERNALIZE)
    pub phase: String,

    /// Ballot counter
    #[serde(rename = "ballotCounter", default)]
    pub ballot_counter: u32,

    /// Hash of the value being voted on
    #[serde(rename = "valueHash", default)]
    pub value_hash: String,
}

/// Nomination state from SCP
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NominationState {
    /// Values being voted on
    #[serde(default)]
    pub votes: Vec<String>,

    /// Values that have been accepted
    #[serde(default)]
    pub accepted: Vec<String>,
}

/// Peer information from Stellar Core
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PeerInfo {
    /// Peer public key
    pub id: String,

    /// Peer address
    #[serde(default)]
    pub address: String,

    /// Connection state
    #[serde(default)]
    pub state: String,
}
