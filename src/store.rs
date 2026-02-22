//! SQLite alert storage — write matches, query history.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{params, Connection};

use crate::rules::Alert;

pub struct AlertStore {
    conn: Mutex<Connection>,
}

impl AlertStore {
    pub fn open(path: &Path) -> eyre::Result<Self> {
        let conn = Connection::open(path)?;

        conn.execute_batch("
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;

            CREATE TABLE IF NOT EXISTS alerts (
                id          INTEGER PRIMARY KEY AUTOINCREMENT,
                rule_name   TEXT NOT NULL,
                block_number INTEGER,
                fb_index    INTEGER NOT NULL,
                timestamp   INTEGER NOT NULL,
                to_addr     TEXT,
                to_label    TEXT,
                value_eth   REAL NOT NULL,
                action      TEXT,
                category    TEXT NOT NULL,
                payload     TEXT NOT NULL,
                created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            );

            CREATE INDEX IF NOT EXISTS idx_alerts_rule ON alerts(rule_name);
            CREATE INDEX IF NOT EXISTS idx_alerts_ts ON alerts(timestamp);
            CREATE INDEX IF NOT EXISTS idx_alerts_category ON alerts(category);
            CREATE INDEX IF NOT EXISTS idx_alerts_block ON alerts(block_number);
        ")?;

        Ok(Self { conn: Mutex::new(conn) })
    }

    pub fn insert(&self, alert: &Alert) -> eyre::Result<()> {
        let payload = serde_json::to_string(alert)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO alerts (rule_name, block_number, fb_index, timestamp, to_addr, to_label, value_eth, action, category, payload)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                alert.rule_name,
                alert.block_number.map(|n| n as i64),
                alert.flashblock_index as i64,
                alert.timestamp as i64,
                alert.tx.to,
                alert.tx.to_label,
                alert.tx.value_eth,
                alert.tx.action,
                alert.tx.category,
                payload,
            ],
        )?;
        Ok(())
    }

    /// Query alerts with optional filters.
    pub fn query(&self, params: &AlertQuery) -> eyre::Result<Vec<serde_json::Value>> {
        let conn = self.conn.lock().unwrap();

        let mut where_clauses = Vec::new();
        let mut bind_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

        if let Some(ref rule) = params.rule {
            where_clauses.push(format!("rule_name = ?{}", bind_values.len() + 1));
            bind_values.push(Box::new(rule.clone()));
        }

        if let Some(ref category) = params.category {
            where_clauses.push(format!("category = ?{}", bind_values.len() + 1));
            bind_values.push(Box::new(category.clone()));
        }

        if let Some(min_eth) = params.min_eth {
            where_clauses.push(format!("value_eth >= ?{}", bind_values.len() + 1));
            bind_values.push(Box::new(min_eth));
        }

        if let Some(since) = params.since_ts {
            where_clauses.push(format!("timestamp >= ?{}", bind_values.len() + 1));
            bind_values.push(Box::new(since as i64));
        }

        let where_sql = if where_clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", where_clauses.join(" AND "))
        };

        let limit = params.limit.unwrap_or(100).min(1000);

        let sql = format!(
            "SELECT payload FROM alerts {} ORDER BY id DESC LIMIT {}",
            where_sql, limit
        );

        let refs: Vec<&dyn rusqlite::types::ToSql> = bind_values.iter().map(|b| b.as_ref()).collect();

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(refs.as_slice(), |row| {
            let payload: String = row.get(0)?;
            Ok(payload)
        })?;

        let mut results = Vec::new();
        for row in rows {
            if let Ok(payload) = row {
                if let Ok(val) = serde_json::from_str(&payload) {
                    results.push(val);
                }
            }
        }

        Ok(results)
    }

    /// Get summary stats.
    pub fn stats(&self) -> eyre::Result<serde_json::Value> {
        let conn = self.conn.lock().unwrap();

        let total: i64 = conn.query_row("SELECT COUNT(*) FROM alerts", [], |r| r.get(0))?;

        let last_hour: i64 = conn.query_row(
            "SELECT COUNT(*) FROM alerts WHERE timestamp > unixepoch() - 3600",
            [], |r| r.get(0),
        )?;

        let mut stmt = conn.prepare(
            "SELECT rule_name, COUNT(*) as cnt FROM alerts GROUP BY rule_name ORDER BY cnt DESC LIMIT 10"
        )?;
        let by_rule: Vec<serde_json::Value> = stmt.query_map([], |row| {
            let name: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok(serde_json::json!({"rule": name, "count": count}))
        })?.filter_map(|r| r.ok()).collect();

        let mut stmt = conn.prepare(
            "SELECT category, COUNT(*) as cnt FROM alerts GROUP BY category ORDER BY cnt DESC"
        )?;
        let by_category: Vec<serde_json::Value> = stmt.query_map([], |row| {
            let cat: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok(serde_json::json!({"category": cat, "count": count}))
        })?.filter_map(|r| r.ok()).collect();

        Ok(serde_json::json!({
            "total_alerts": total,
            "last_hour": last_hour,
            "by_rule": by_rule,
            "by_category": by_category,
        }))
    }

    /// Prune alerts older than the given number of days. Returns count deleted.
    pub fn prune(&self, retention_days: u64) -> eyre::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let deleted = conn.execute(
            "DELETE FROM alerts WHERE timestamp < unixepoch() - ?1",
            params![retention_days * 86400],
        )?;
        if deleted > 0 {
            let _ = conn.execute_batch("PRAGMA incremental_vacuum;");
        }
        Ok(deleted)
    }
}

/// Query parameters for the /alerts endpoint.
#[derive(Debug, Default)]
pub struct AlertQuery {
    pub rule: Option<String>,
    pub category: Option<String>,
    pub min_eth: Option<f64>,
    pub since_ts: Option<u64>,
    pub limit: Option<usize>,
}

impl AlertQuery {
    /// Parse from URL query string params.
    pub fn from_params(params: &std::collections::HashMap<String, String>) -> Self {
        let since_ts = params.get("last").and_then(|v| parse_duration_secs(v)).map(|secs| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            now - secs
        }).or_else(|| params.get("since").and_then(|v| v.parse().ok()));

        Self {
            rule: params.get("rule").cloned(),
            category: params.get("category").cloned(),
            min_eth: params.get("min_eth").and_then(|v| v.parse().ok()),
            since_ts,
            limit: params.get("limit").and_then(|v| v.parse().ok()),
        }
    }
}

/// Parse human duration like "1h", "30m", "24h", "7d" into seconds.
fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() { return None; }

    let (num, suffix) = s.split_at(s.len() - 1);
    let n: u64 = num.parse().ok()?;

    match suffix {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        _ => None,
    }
}
