//! Transaction decoding — RLP parsing, function signatures, address labels.

use std::collections::HashMap;

use serde::Serialize;

/// Known contract addresses on Base mainnet.
pub fn known_addresses() -> HashMap<&'static str, AddressLabel> {
    let mut m = HashMap::new();

    // DEXes
    m.insert("0x2626664c2603336e57b271c5c0b26f421741e481", AddressLabel::new("Uniswap V3 Router", Category::Dex));
    m.insert("0x3fc91a3afd70395cd496c647d5a6cc9d4b2b7fad", AddressLabel::new("Uniswap Universal Router", Category::Dex));
    m.insert("0xcf77a3ba9a5ca399b7c97c74d54e5b1beb874e43", AddressLabel::new("Aerodrome Router", Category::Dex));
    m.insert("0x6cb442acf35158d5eda88fe602221b67b400be3e", AddressLabel::new("Aerodrome V2 Router", Category::Dex));
    m.insert("0x327df1e6de05895d2ab08513aadd9313fe505d86", AddressLabel::new("BaseSwap Router", Category::Dex));
    m.insert("0x1b8eea9315be495187d873da7773a874545d9d48", AddressLabel::new("SushiSwap Router", Category::Dex));
    m.insert("0xd9aac140860e5b0abd5e1d8a3b3a39e09cccc517", AddressLabel::new("Odos Router", Category::Dex));

    // Bridges
    m.insert("0x4200000000000000000000000000000000000010", AddressLabel::new("L2 Standard Bridge", Category::Bridge));
    m.insert("0x4200000000000000000000000000000000000007", AddressLabel::new("L2 Cross Domain Messenger", Category::Bridge));
    m.insert("0x3154cf16ccdb4c6d922629664174b904d80f2c35", AddressLabel::new("Base Bridge", Category::Bridge));
    m.insert("0xaf28bcb48c40dbc86f52d459a6562f658fc94b1e", AddressLabel::new("Stargate Bridge", Category::Bridge));
    m.insert("0x1a44076050125825900e736c501f859c50fe728c", AddressLabel::new("LayerZero Endpoint", Category::Bridge));

    // Tokens
    m.insert("0x833589fcd6edb6e08f4c7c32d4f71b54bda02913", AddressLabel::new("USDC", Category::Token));
    m.insert("0x50c5725949a6f0c72e6c4a641f24049a917db0cb", AddressLabel::new("DAI", Category::Token));
    m.insert("0x4200000000000000000000000000000000000006", AddressLabel::new("WETH", Category::Token));
    m.insert("0x2ae3f1ec7f1f5012cfeab0185bfc7aa3cf0dec22", AddressLabel::new("cbETH", Category::Token));
    m.insert("0xd9aaec86b65d86f6a7b5b1b0c42ffa531710b6ca", AddressLabel::new("USDbC", Category::Token));
    m.insert("0xb6fe221fe9eef5aba221c348ba20a1bf5e73624c", AddressLabel::new("rETH", Category::Token));

    // Lending
    m.insert("0xa238dd80c259a72e81d7e4664a9801593f98d1c5", AddressLabel::new("Aave V3 Pool", Category::Lending));
    m.insert("0x9c4ec768c28520b50860ea7a15bd7213a9ff58bf", AddressLabel::new("Compound V3 USDC", Category::Lending));
    m.insert("0x46e6b214b524310239732d51387075e0e70970bf", AddressLabel::new("Moonwell", Category::Lending));

    // NFT
    m.insert("0x00000000000000adc04c56bf30ac9d3c0aaf14dc", AddressLabel::new("Seaport 1.5", Category::Nft));
    m.insert("0x0000000000000068f116a894984e2db1123eb395", AddressLabel::new("Seaport 1.6", Category::Nft));

    // System
    m.insert("0x4200000000000000000000000000000000000015", AddressLabel::new("L1Block", Category::System));
    m.insert("0x4200000000000000000000000000000000000011", AddressLabel::new("Sequencer Fee Vault", Category::System));
    m.insert("0x420000000000000000000000000000000000001a", AddressLabel::new("Base Fee Vault", Category::System));
    m.insert("0xdeaddeaddeaddeaddeaddeaddeaddeaddead0001", AddressLabel::new("L1 Attributes Depositor", Category::System));

    m
}

/// Known function selectors (first 4 bytes of calldata).
pub fn known_selectors() -> HashMap<[u8; 4], &'static str> {
    let mut m = HashMap::new();

    // ERC20
    m.insert(hex4("a9059cbb"), "transfer");
    m.insert(hex4("23b872dd"), "transferFrom");
    m.insert(hex4("095ea7b3"), "approve");

    // DEX - Uniswap
    m.insert(hex4("3593564c"), "execute (Universal Router)");
    m.insert(hex4("38ed1739"), "swapExactTokensForTokens");
    m.insert(hex4("7ff36ab5"), "swapExactETHForTokens");
    m.insert(hex4("18cbafe5"), "swapExactTokensForETH");
    m.insert(hex4("5ae401dc"), "multicall");
    m.insert(hex4("ac9650d8"), "multicall (v2)");
    m.insert(hex4("04e45aaf"), "exactInputSingle");
    m.insert(hex4("b858183f"), "exactInput");
    m.insert(hex4("414bf389"), "exactInputSingle (v3)");

    // Aerodrome
    m.insert(hex4("b6f9de95"), "swapExactETHForTokens (fee)");
    m.insert(hex4("cac88ea9"), "swapExactTokensForTokens (Aero)");

    // Bridge
    m.insert(hex4("32b7006d"), "depositETHTo");
    m.insert(hex4("a3a79548"), "depositERC20To");

    // Lending
    m.insert(hex4("617ba037"), "supply (Aave)");
    m.insert(hex4("69328dec"), "withdraw (Aave)");
    m.insert(hex4("c5ebeaec"), "borrow (Aave)");
    m.insert(hex4("573ade81"), "repay (Aave)");
    m.insert(hex4("f2b9fdb8"), "supply (Compound)");

    // NFT
    m.insert(hex4("fb0f3ee1"), "fulfillBasicOrder (Seaport)");
    m.insert(hex4("87201b41"), "fulfillOrder (Seaport)");
    m.insert(hex4("42842e0e"), "safeTransferFrom (ERC721)");

    // General
    m.insert(hex4("d0e30db0"), "deposit (wrap ETH)");
    m.insert(hex4("2e1a7d4d"), "withdraw (unwrap ETH)");

    m
}

fn hex4(s: &str) -> [u8; 4] {
    let bytes = hex::decode(s).expect("valid hex");
    [bytes[0], bytes[1], bytes[2], bytes[3]]
}

#[derive(Debug, Clone, Serialize)]
pub struct AddressLabel {
    pub name: &'static str,
    pub category: Category,
}

impl AddressLabel {
    pub const fn new(name: &'static str, category: Category) -> Self {
        Self { name, category }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Dex,
    Bridge,
    Token,
    Lending,
    Nft,
    System,
    Unknown,
}

impl Category {
    pub fn emoji(&self) -> &'static str {
        match self {
            Self::Dex => "🔄",
            Self::Bridge => "🌉",
            Self::Token => "💰",
            Self::Lending => "🏦",
            Self::Nft => "🖼️",
            Self::System => "⚙️",
            Self::Unknown => "📦",
        }
    }

    pub fn color(&self) -> &'static str {
        match self {
            Self::Dex => "#22d3ee",
            Self::Bridge => "#a78bfa",
            Self::Token => "#4ade80",
            Self::Lending => "#fbbf24",
            Self::Nft => "#f472b6",
            Self::System => "#6b7280",
            Self::Unknown => "#9ca3af",
        }
    }
}

/// A decoded transaction with labels.
#[derive(Debug, Clone, Serialize)]
pub struct DecodedTx {
    pub hash: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub to_label: Option<AddressLabel>,
    pub value_wei: u128,
    pub value_eth: f64,
    pub action: Option<String>,
    pub category: Category,
    pub gas_used: Option<u64>,
}

/// Decode a raw RLP-encoded transaction.
/// Base transactions are EIP-1559 (type 2), prefixed with 0x02.
pub fn decode_raw_tx(hex_str: &str) -> Option<DecodedTx> {
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);
    let bytes = hex::decode(hex_str).ok()?;

    if bytes.is_empty() {
        return None;
    }

    let addresses = known_addresses();
    let selectors = known_selectors();

    // Type byte
    let (tx_type, rlp_bytes) = if bytes[0] <= 0x7f {
        (bytes[0], &bytes[1..])
    } else {
        (0u8, &bytes[..])
    };

    // Parse RLP list
    let items = decode_rlp_list(rlp_bytes)?;

    // EIP-1559 (type 2): [chainId, nonce, maxPriorityFeePerGas, maxFeePerGas, gasLimit, to, value, data, accessList, v, r, s]
    // EIP-2930 (type 1): [chainId, nonce, gasPrice, gasLimit, to, value, data, accessList, v, r, s]
    // Legacy (type 0): [nonce, gasPrice, gasLimit, to, value, data, v, r, s]
    // Deposit (type 0x7e): different format

    let (to_bytes, value_bytes, data_bytes) = match tx_type {
        0x02 if items.len() >= 8 => {
            // EIP-1559: to=5, value=6, data=7
            (items.get(5)?, items.get(6)?, items.get(7)?)
        }
        0x01 if items.len() >= 7 => {
            // EIP-2930: to=4, value=5, data=6
            (items.get(4)?, items.get(5)?, items.get(6)?)
        }
        0x7e => {
            // Deposit tx: skip for now
            return None;
        }
        _ if items.len() >= 6 => {
            // Legacy: to=3, value=4, data=5
            (items.get(3)?, items.get(4)?, items.get(5)?)
        }
        _ => return None,
    };

    let to_hex = if to_bytes.is_empty() {
        None
    } else {
        Some(format!("0x{}", hex::encode(to_bytes)))
    };

    let value_wei = bytes_to_u128(value_bytes);
    let value_eth = value_wei as f64 / 1e18;

    // Look up address label
    let to_lower = to_hex.as_ref().map(|a| a.to_lowercase());
    let to_label = to_lower
        .as_ref()
        .and_then(|addr| addresses.get(addr.as_str()).cloned());

    // Decode function selector
    let action = if data_bytes.len() >= 4 {
        let mut sel = [0u8; 4];
        sel.copy_from_slice(&data_bytes[..4]);
        selectors.get(&sel).map(|s| s.to_string())
    } else if !data_bytes.is_empty() {
        None
    } else if value_wei > 0 {
        Some("ETH transfer".to_string())
    } else {
        None
    };

    let category = to_label
        .as_ref()
        .map(|l| l.category)
        .unwrap_or(Category::Unknown);

    Some(DecodedTx {
        hash: None, // set later from receipt
        from: None, // not in raw tx without recovery
        to: to_hex,
        to_label,
        value_wei,
        value_eth,
        action,
        category,
        gas_used: None,
    })
}

/// Minimal RLP list decoder — returns the items in a top-level list.
fn decode_rlp_list(data: &[u8]) -> Option<Vec<Vec<u8>>> {
    if data.is_empty() {
        return None;
    }

    let (payload, _) = decode_rlp_item(data)?;

    // If it's a list, decode items within
    if data[0] >= 0xc0 {
        let mut items = Vec::new();
        let mut pos = 0;
        while pos < payload.len() {
            let (item, consumed) = decode_rlp_item(&payload[pos..])?;
            items.push(item.to_vec());
            pos += consumed;
        }
        Some(items)
    } else {
        None
    }
}

fn decode_rlp_item(data: &[u8]) -> Option<(&[u8], usize)> {
    if data.is_empty() {
        return None;
    }

    let prefix = data[0];

    if prefix < 0x80 {
        // Single byte
        Some((&data[..1], 1))
    } else if prefix <= 0xb7 {
        // Short string (0-55 bytes)
        let len = (prefix - 0x80) as usize;
        if data.len() < 1 + len {
            return None;
        }
        Some((&data[1..1 + len], 1 + len))
    } else if prefix <= 0xbf {
        // Long string
        let len_of_len = (prefix - 0xb7) as usize;
        if data.len() < 1 + len_of_len {
            return None;
        }
        let len = bytes_to_usize(&data[1..1 + len_of_len]);
        if data.len() < 1 + len_of_len + len {
            return None;
        }
        Some((&data[1 + len_of_len..1 + len_of_len + len], 1 + len_of_len + len))
    } else if prefix <= 0xf7 {
        // Short list (0-55 bytes)
        let len = (prefix - 0xc0) as usize;
        if data.len() < 1 + len {
            return None;
        }
        Some((&data[1..1 + len], 1 + len))
    } else {
        // Long list
        let len_of_len = (prefix - 0xf7) as usize;
        if data.len() < 1 + len_of_len {
            return None;
        }
        let len = bytes_to_usize(&data[1..1 + len_of_len]);
        if data.len() < 1 + len_of_len + len {
            return None;
        }
        Some((&data[1 + len_of_len..1 + len_of_len + len], 1 + len_of_len + len))
    }
}

fn bytes_to_usize(bytes: &[u8]) -> usize {
    let mut result = 0usize;
    for &b in bytes {
        result = (result << 8) | b as usize;
    }
    result
}

fn bytes_to_u128(bytes: &[u8]) -> u128 {
    let mut result = 0u128;
    for &b in bytes {
        result = (result << 8) | b as u128;
    }
    result
}
