//! Rule-based alert system — parse TOML configs and match against decoded transactions.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::decode::{Category, DecodedTx};

/// Top-level rules config file.
#[derive(Deserialize, Debug, Clone)]
pub struct RulesConfig {
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub global: GlobalConfig,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct GlobalConfig {
    /// Default cooldown between fires of the same rule (seconds).
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
    /// Max webhook fires per minute across all rules.
    #[serde(default = "default_rate_limit")]
    pub max_per_minute: u64,
    /// Batch window in seconds (0 = fire immediately).
    #[serde(default)]
    pub batch_secs: u64,
    /// Alert retention in days (auto-prune older alerts).
    #[serde(default = "default_retention")]
    pub retention_days: u64,
}

fn default_cooldown() -> u64 { 10 }
fn default_rate_limit() -> u64 { 30 }
fn default_retention() -> u64 { 30 }

#[derive(Deserialize, Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub trigger: Trigger,
    /// Webhook URL to POST to (optional — if absent, just logs).
    pub webhook: Option<String>,
    /// Override global cooldown for this rule.
    pub cooldown_secs: Option<u64>,
    /// Whether this rule is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// Match ETH transfers above a threshold.
    EthTransfer {
        #[serde(default)]
        min_eth: f64,
    },
    /// Match transactions to specific protocols/categories.
    Protocol {
        #[serde(default)]
        names: Vec<String>,
        #[serde(default)]
        categories: Vec<String>,
        /// Optional minimum ETH value.
        #[serde(default)]
        min_eth: f64,
    },
    /// Match specific function calls.
    FunctionCall {
        /// Function action strings to match (e.g. "swapExactETHForTokens").
        actions: Vec<String>,
        #[serde(default)]
        min_eth: f64,
    },
    /// Match any transaction above an ETH threshold.
    LargeValue {
        min_eth: f64,
    },
    /// Match transactions to a specific address.
    Address {
        address: String,
        #[serde(default)]
        min_eth: f64,
    },
}

/// A matched alert ready to be logged/sent.
#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub rule_name: String,
    pub block_number: Option<u64>,
    pub flashblock_index: u64,
    pub tx: AlertTx,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertTx {
    pub from: Option<String>,
    pub to: Option<String>,
    pub to_label: Option<String>,
    pub value_eth: f64,
    pub action: Option<String>,
    pub category: String,
}

impl From<&DecodedTx> for AlertTx {
    fn from(tx: &DecodedTx) -> Self {
        Self {
            from: tx.from.clone(),
            to: tx.to.clone(),
            to_label: tx.to_label.as_ref().map(|l| l.name.to_string()),
            value_eth: tx.value_eth,
            action: tx.action.clone(),
            category: format!("{:?}", tx.category).to_lowercase(),
        }
    }
}

/// Runtime state for rate limiting and cooldowns.
pub struct RuleEngine {
    pub config: RulesConfig,
    last_fired: HashMap<String, Instant>,
    fires_this_minute: Vec<Instant>,
}

impl RuleEngine {
    pub fn new(config: RulesConfig) -> Self {
        Self {
            config,
            last_fired: HashMap::new(),
            fires_this_minute: Vec::new(),
        }
    }

    pub fn from_toml(toml_str: &str) -> eyre::Result<Self> {
        let config: RulesConfig = toml::from_str(toml_str)?;
        Ok(Self::new(config))
    }

    /// Check a decoded transaction against all rules. Returns alerts for matches.
    pub fn check(
        &mut self,
        tx: &DecodedTx,
        block_number: Option<u64>,
        flashblock_index: u64,
    ) -> Vec<Alert> {
        let now = Instant::now();
        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Prune old fires for rate limiting
        self.fires_this_minute.retain(|t| now.duration_since(*t) < Duration::from_secs(60));

        let mut alerts = Vec::new();

        for rule in &self.config.rules {
            if !rule.enabled {
                continue;
            }

            // Rate limit check
            if self.fires_this_minute.len() as u64 >= self.config.global.max_per_minute {
                break;
            }

            // Cooldown check
            let cooldown = rule.cooldown_secs.unwrap_or(self.config.global.cooldown_secs);
            if let Some(last) = self.last_fired.get(&rule.name) {
                if now.duration_since(*last) < Duration::from_secs(cooldown) {
                    continue;
                }
            }

            if matches_rule(&rule.trigger, tx) {
                self.last_fired.insert(rule.name.clone(), now);
                self.fires_this_minute.push(now);

                alerts.push(Alert {
                    rule_name: rule.name.clone(),
                    block_number,
                    flashblock_index,
                    tx: AlertTx::from(tx),
                    timestamp: epoch_secs,
                });
            }
        }

        alerts
    }
}

fn matches_rule(trigger: &Trigger, tx: &DecodedTx) -> bool {
    match trigger {
        Trigger::EthTransfer { min_eth } => {
            tx.value_eth >= *min_eth
                && tx.action.as_deref() == Some("ETH transfer")
        }
        Trigger::Protocol { names, categories, min_eth } => {
            if tx.value_eth < *min_eth {
                return false;
            }
            let label_match = if names.is_empty() {
                true
            } else {
                tx.to_label.as_ref().map_or(false, |l| {
                    names.iter().any(|n| l.name.eq_ignore_ascii_case(n))
                })
            };
            let cat_match = if categories.is_empty() {
                true
            } else {
                let cat_str = format!("{:?}", tx.category).to_lowercase();
                categories.iter().any(|c| c.to_lowercase() == cat_str)
            };
            label_match && cat_match
        }
        Trigger::FunctionCall { actions, min_eth } => {
            if tx.value_eth < *min_eth {
                return false;
            }
            tx.action.as_ref().map_or(false, |a| {
                actions.iter().any(|act| a.contains(act))
            })
        }
        Trigger::LargeValue { min_eth } => {
            tx.value_eth >= *min_eth
        }
        Trigger::Address { address, min_eth } => {
            if tx.value_eth < *min_eth {
                return false;
            }
            tx.to.as_ref().map_or(false, |to| {
                to.eq_ignore_ascii_case(address)
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::{AddressLabel, Category};

    fn make_tx(value_eth: f64, action: Option<&str>, category: Category, label: Option<&'static str>) -> DecodedTx {
        DecodedTx {
            hash: None,
            from: None,
            to: Some("0x1234".into()),
            to_label: label.map(|n| AddressLabel::new(n, category)),
            value_wei: (value_eth * 1e18) as u128,
            value_eth,
            action: action.map(String::from),
            category,
            gas_used: None,
        }
    }

    #[test]
    fn test_eth_transfer_trigger() {
        let trigger = Trigger::EthTransfer { min_eth: 5.0 };
        let tx = make_tx(10.0, Some("ETH transfer"), Category::Unknown, None);
        assert!(matches_rule(&trigger, &tx));

        let small = make_tx(1.0, Some("ETH transfer"), Category::Unknown, None);
        assert!(!matches_rule(&trigger, &small));
    }

    #[test]
    fn test_large_value_trigger() {
        let trigger = Trigger::LargeValue { min_eth: 1.0 };
        let tx = make_tx(2.5, Some("swap"), Category::Dex, None);
        assert!(matches_rule(&trigger, &tx));
    }

    #[test]
    fn test_protocol_trigger() {
        let trigger = Trigger::Protocol {
            names: vec!["Uniswap V3 Router".into()],
            categories: vec![],
            min_eth: 0.0,
        };
        let tx = make_tx(0.1, Some("swap"), Category::Dex, Some("Uniswap V3 Router"));
        assert!(matches_rule(&trigger, &tx));

        let other = make_tx(0.1, Some("swap"), Category::Dex, Some("Aerodrome Router"));
        assert!(!matches_rule(&trigger, &other));
    }
}
