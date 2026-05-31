use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterResult {
    pub passed: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ExitDecision {
    TakeProfit,
    StopLoss,
    TimeoutStale,
    ForceClose,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterExecution {
    pub name: String,
    pub result: String, // "PASS" or "FAIL"
    pub details: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTrace {
    pub trace_id: Uuid,
    pub strategy_id: String,
    pub token_address: String,
    pub timestamp: DateTime<Utc>,
    pub filters: Vec<FilterExecution>,
    pub final_decision: String, // "BUY", "SKIP", "REJECT"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeTrace {
    pub strategy_id: String,
    pub token_addr: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_pct: f64,
    pub exit_type: String,
    pub hold_secs: i64,
}

#[derive(Debug, Clone)]
pub enum TraceRecord {
    Decision(DecisionTrace),
    Trade(TradeTrace),
    // Akan ditambahkan Trace untuk Wallet
}
