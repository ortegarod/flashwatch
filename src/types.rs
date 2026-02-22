//! Core types for flashblock data.

use serde::{Deserialize, Serialize};

/// A JSON-RPC request envelope.
#[derive(Serialize)]
pub struct JsonRpcRequest<'a> {
    pub jsonrpc: &'a str,
    pub id: u64,
    pub method: &'a str,
    pub params: serde_json::Value,
}

/// A JSON-RPC response envelope.
#[derive(Deserialize, Debug)]
pub struct JsonRpcResponse<T> {
    pub id: Option<u64>,
    pub result: Option<T>,
    pub error: Option<JsonRpcError>,
}

#[derive(Deserialize, Debug)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// Raw flashblock message from the Base WebSocket feed.
/// The feed pushes diffs — index 0 has the full base, subsequent indices are incremental.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FlashblockMessage {
    /// Payload ID identifying the block being built
    pub payload_id: String,
    /// Flashblock index within the current block (0 = initial, 1+ = incremental)
    pub index: u64,
    /// Base block header fields (only present when index == 0)
    pub base: Option<FlashblockBase>,
    /// Diff data — new transactions, updated state root, gas used
    pub diff: FlashblockDiff,
    /// Metadata (optional)
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

/// Block header fields sent with the initial flashblock (index 0).
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FlashblockBase {
    pub parent_hash: Option<String>,
    pub fee_recipient: Option<String>,
    pub block_number: Option<String>,
    pub gas_limit: Option<String>,
    pub timestamp: Option<String>,
    pub base_fee_per_gas: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Incremental diff for each flashblock.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct FlashblockDiff {
    pub state_root: Option<String>,
    pub block_hash: Option<String>,
    pub gas_used: Option<String>,
    #[serde(default)]
    pub transactions: Vec<serde_json::Value>,
    pub receipts: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl FlashblockMessage {
    /// Parse block number from the base header (hex).
    pub fn block_number(&self) -> Option<u64> {
        self.base
            .as_ref()
            .and_then(|b| b.block_number.as_ref())
            .and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok())
    }

    /// Parse gas used from the diff (hex).
    pub fn gas_used(&self) -> Option<u64> {
        self.diff
            .gas_used
            .as_ref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
    }

    /// Parse gas limit from base header (hex).
    pub fn gas_limit(&self) -> Option<u64> {
        self.base
            .as_ref()
            .and_then(|b| b.gas_limit.as_ref())
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
    }

    /// Number of transactions in this flashblock diff.
    pub fn tx_count(&self) -> usize {
        self.diff.transactions.len()
    }

    /// Parse base fee from hex (in Gwei).
    pub fn base_fee_gwei(&self) -> Option<f64> {
        self.base.as_ref().and_then(|b| {
            b.base_fee_per_gas.as_ref().and_then(|f| {
                u64::from_str_radix(f.trim_start_matches("0x"), 16)
                    .ok()
                    .map(|wei| wei as f64 / 1e9)
            })
        })
    }

    /// Parse timestamp from base header.
    pub fn timestamp(&self) -> Option<u64> {
        self.base
            .as_ref()
            .and_then(|b| b.timestamp.as_ref())
            .and_then(|t| u64::from_str_radix(t.trim_start_matches("0x"), 16).ok())
    }
}

/// Accumulated state for the current block being built.
#[derive(Default, Debug)]
pub struct BlockState {
    pub payload_id: String,
    pub block_number: Option<u64>,
    pub gas_limit: Option<u64>,
    pub base_fee_gwei: Option<f64>,
    pub timestamp: Option<u64>,
    pub flashblock_count: u64,
    pub total_gas_used: u64,
    pub total_tx_count: usize,
}

impl BlockState {
    pub fn update(&mut self, msg: &FlashblockMessage) {
        if msg.payload_id != self.payload_id {
            // New block — reset
            self.payload_id = msg.payload_id.clone();
            self.flashblock_count = 0;
            self.total_gas_used = 0;
            self.total_tx_count = 0;
            self.block_number = msg.block_number();
            self.gas_limit = msg.gas_limit();
            self.base_fee_gwei = msg.base_fee_gwei();
            self.timestamp = msg.timestamp();
        }
        self.flashblock_count += 1;
        if let Some(gas) = msg.gas_used() {
            self.total_gas_used += gas;
        }
        self.total_tx_count += msg.tx_count();
    }
}

/// Global metrics across all blocks.
#[derive(Default, Debug)]
pub struct FlashblockMetrics {
    pub total_flashblocks: u64,
    pub total_transactions: u64,
    pub total_gas_used: u64,
    pub blocks_seen: u64,
    pub current_block: BlockState,
    pub flashblocks_per_second: f64,
    pub last_received: Option<std::time::Instant>,
}

impl FlashblockMetrics {
    pub fn update(&mut self, msg: &FlashblockMessage) {
        let prev_payload = self.current_block.payload_id.clone();
        self.current_block.update(msg);
        if msg.payload_id != prev_payload && !prev_payload.is_empty() {
            self.blocks_seen += 1;
        }

        self.total_flashblocks += 1;
        self.total_transactions += msg.tx_count() as u64;
        if let Some(gas) = msg.gas_used() {
            self.total_gas_used += gas;
        }
        self.last_received = Some(std::time::Instant::now());
    }
}
