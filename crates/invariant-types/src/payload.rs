use serde::{Deserialize, Serialize};

/// Codec used to encode/decode payload bytes.
/// Matches the SDK's supported serialization formats.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    Cbor,
    Json,
    Borsh,
}

/// Opaque bytes with an associated codec.
///
/// SDK boundary handles conversion to/from the SDK's Payload type.
/// For Invariant types they are just bytes
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub bytes: Vec<u8>,
    pub codec: Codec,
}

impl Payload {
    /// Create a payload from raw bytes and their codec.
    pub fn new(bytes: Vec<u8>, codec: Codec) -> Self {
        Self { bytes, codec }
    }
}
