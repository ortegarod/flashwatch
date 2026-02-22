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

/// A JSON-RPC subscription notification.
#[derive(Deserialize, Debug)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: Option<String>,
    pub params: Option<SubscriptionParams>,
}

#[derive(Deserialize, Debug)]
pub struct SubscriptionParams {
    pub subscription: String,
    pub result: serde_json::Value,
}

#[derive(Deserialize, Debug)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// Flashblock as received from the `newFlashblocks` subscription.
/// Uses generic serde_json::Value for flexibility — Base may add fields.
#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Flashblock {
    /// Block hash (zero hash for pending flashblocks)
    pub hash: Option<String>,
    /// Parent block hash
    #[serde(rename = "parentHash")]
    pub parent_hash: Option<String>,
    /// Block number (hex)
    pub number: Option<String>,
    /// Timestamp (hex)
    pub timestamp: Option<String>,
    /// Gas used (hex)
    #[serde(rename = "gasUsed")]
    pub gas_used: Option<String>,
    /// Gas limit (hex)
    #[serde(rename = "gasLimit")]
    pub gas_limit: Option<String>,
    /// Transaction list (hashes or full objects)
    pub transactions: Option<serde_json::Value>,
    /// Base fee per gas (hex)
    #[serde(rename = "baseFeePerGas")]
    pub base_fee_per_gas: Option<String>,

    /// Catch-all for other fields
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Flashblock {
    /// Parse the block number from hex.
    pub fn block_number(&self) -> Option<u64> {
        self.number.as_ref().and_then(|n| u64::from_str_radix(n.trim_start_matches("0x"), 16).ok())
    }

    /// Parse gas used from hex.
    pub fn gas_used_val(&self) -> Option<u64> {
        self.gas_used
            .as_ref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
    }

    /// Parse gas limit from hex.
    pub fn gas_limit_val(&self) -> Option<u64> {
        self.gas_limit
            .as_ref()
            .and_then(|g| u64::from_str_radix(g.trim_start_matches("0x"), 16).ok())
    }

    /// Count transactions.
    pub fn tx_count(&self) -> usize {
        match &self.transactions {
            Some(serde_json::Value::Array(txs)) => txs.len(),
            _ => 0,
        }
    }

    /// Parse base fee from hex (in Gwei).
    pub fn base_fee_gwei(&self) -> Option<f64> {
        self.base_fee_per_gas.as_ref().and_then(|b| {
            u64::from_str_radix(b.trim_start_matches("0x"), 16)
                .ok()
                .map(|wei| wei as f64 / 1e9)
        })
    }
}

/// Metrics snapshot for the monitor display.
#[derive(Default, Debug)]
pub struct FlashblockMetrics {
    pub total_flashblocks: u64,
    pub total_transactions: u64,
    pub total_gas_used: u64,
    pub current_block_number: u64,
    pub flashblocks_in_current_block: u32,
    pub avg_tx_per_flashblock: f64,
    pub avg_gas_per_flashblock: f64,
    pub last_base_fee_gwei: f64,
    pub flashblocks_per_second: f64,
    pub last_received: Option<std::time::Instant>,
}

impl FlashblockMetrics {
    pub fn update(&mut self, fb: &Flashblock) {
        self.total_flashblocks += 1;
        let tx_count = fb.tx_count() as u64;
        self.total_transactions += tx_count;

        if let Some(gas) = fb.gas_used_val() {
            self.total_gas_used += gas;
        }

        if let Some(num) = fb.block_number() {
            if num != self.current_block_number {
                self.current_block_number = num;
                self.flashblocks_in_current_block = 1;
            } else {
                self.flashblocks_in_current_block += 1;
            }
        }

        if let Some(fee) = fb.base_fee_gwei() {
            self.last_base_fee_gwei = fee;
        }

        if self.total_flashblocks > 0 {
            self.avg_tx_per_flashblock =
                self.total_transactions as f64 / self.total_flashblocks as f64;
            self.avg_gas_per_flashblock =
                self.total_gas_used as f64 / self.total_flashblocks as f64;
        }

        self.last_received = Some(std::time::Instant::now());
    }
}
