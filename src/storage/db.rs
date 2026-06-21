use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::str::FromStr;
use tracing::error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeRecord {
    pub timestamp: String,
    pub token_addr: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub pnl_pct: f64,
    pub exit_type: String,
    pub hold_secs: i64,
    pub volume_entry: f64,
    pub velocity_score: f64,
    pub buyers_count: u32,
    pub entry_score: f64,
    pub hour_utc: u32,
}

pub async fn init_db(path: &str) -> SqlitePool {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite:{}", path))
        .expect("Format path database salah")
        .create_if_missing(true);

    let pool = SqlitePoolOptions::new()
        .max_connections(3)
        .connect_with(opts)
        .await
        .expect("Gagal koneksi SQLite");

    sqlx::query("PRAGMA journal_mode=WAL;").execute(&pool).await.unwrap();

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS trades (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp       TEXT NOT NULL,
            token_addr      TEXT NOT NULL,
            entry_price     REAL NOT NULL,
            exit_price      REAL NOT NULL,
            pnl_pct         REAL NOT NULL,
            exit_type       TEXT NOT NULL,
            hold_secs       INTEGER NOT NULL,
            volume_entry    REAL,
            velocity_score  REAL,
            buyers_count    INTEGER,
            entry_score     REAL,
            hour_utc        INTEGER
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS window_stats (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp       TEXT NOT NULL,
            scanned         INTEGER,
            passed          INTEGER,
            passed_rate     REAL,
            win_rate_30     REAL,
            avg_velocity    REAL,
            market_mode     TEXT
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS open_positions (
            token_addr      TEXT PRIMARY KEY,
            entry_price     REAL NOT NULL,
            entry_time      TEXT NOT NULL,
            volume_entry    REAL,
            velocity_score  REAL,
            buyers_count    INTEGER,
            entry_score     REAL
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS virtual_topups (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp       TEXT NOT NULL,
            amount_added    REAL NOT NULL,
            balance_after   REAL NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    // TABEL BARU UNTUK MULTI-STRATEGY TRACING
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS decision_traces (
            trace_id        TEXT PRIMARY KEY,
            strategy_id     TEXT NOT NULL,
            token_addr      TEXT NOT NULL,
            timestamp       TEXT NOT NULL,
            filters_json    TEXT NOT NULL,
            final_decision  TEXT NOT NULL
        )",
    )
    .execute(&pool)
    .await
    .unwrap();

    // Migrasi Skema Lama (tambahkan strategy_id jika belum ada)
    let _ = sqlx::query("ALTER TABLE trades ADD COLUMN strategy_id TEXT DEFAULT 'Legacy'")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE window_stats ADD COLUMN strategy_id TEXT DEFAULT 'Legacy'")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE open_positions ADD COLUMN strategy_id TEXT DEFAULT 'Legacy'")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE virtual_topups ADD COLUMN strategy_id TEXT DEFAULT 'Legacy'")
        .execute(&pool)
        .await;

    // Kolom Realized PnL untuk analisis fee
    let _ = sqlx::query("ALTER TABLE trades ADD COLUMN gross_pnl_sol REAL DEFAULT 0")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE trades ADD COLUMN fees_paid_sol REAL DEFAULT 0")
        .execute(&pool)
        .await;
    let _ = sqlx::query("ALTER TABLE trades ADD COLUMN realized_net_sol REAL DEFAULT 0")
        .execute(&pool)
        .await;

    pool
}

#[allow(dead_code)]
pub async fn insert_open_position(pool: &SqlitePool, t: &TradeRecord) {
    sqlx::query(
        "INSERT OR REPLACE INTO open_positions
         (token_addr, entry_price, entry_time, volume_entry, velocity_score, buyers_count, entry_score)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&t.token_addr)
    .bind(t.entry_price)
    .bind(&t.timestamp)
    .bind(t.volume_entry)
    .bind(t.velocity_score)
    .bind(t.buyers_count)
    .bind(t.entry_score)
    .execute(pool)
    .await
    .ok();
}

pub async fn delete_open_position(pool: &SqlitePool, addr: &str) {
    sqlx::query("DELETE FROM open_positions WHERE token_addr = ?")
        .bind(addr)
        .execute(pool)
        .await
        .ok();
}

pub async fn load_orphans(pool: &SqlitePool) -> Vec<TradeRecord> {
    let rows = sqlx::query("SELECT * FROM open_positions")
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    let mut orphans = Vec::new();
    for row in rows {
        use sqlx::Row;
        orphans.push(TradeRecord {
            timestamp: row.get("entry_time"),
            token_addr: row.get("token_addr"),
            entry_price: row.get("entry_price"),
            exit_price: 0.0,
            pnl_pct: -100.0, // assume loss or unknown
            exit_type: "ORPHAN".to_string(),
            hold_secs: 0,
            volume_entry: row.get("volume_entry"),
            velocity_score: row.get("velocity_score"),
            buyers_count: row.get("buyers_count"),
            entry_score: row.get("entry_score"),
            hour_utc: 0,
        });
    }
    orphans
}

#[derive(Debug)]
pub struct DailySummary {
    pub trades: i64,
    pub win_rate: f64,
    pub tp: i64,
    pub sl: i64,
    pub roi: f64,
}

pub async fn query_daily_summary(pool: &SqlitePool) -> DailySummary {
    let today_prefix = Utc::now().format("%Y-%m-%d").to_string();

    let row: (i64, i64, i64, f64) = sqlx::query_as(
        "SELECT COUNT(*), 
                SUM(CASE WHEN exit_type='TP' THEN 1 ELSE 0 END),
                SUM(CASE WHEN exit_type='SL' THEN 1 ELSE 0 END),
                SUM(pnl_pct)
         FROM trades WHERE timestamp LIKE ? || '%'",
    )
    .bind(&today_prefix)
    .fetch_one(pool)
    .await
    .unwrap_or((0, 0, 0, 0.0));

    DailySummary {
        trades: row.0,
        win_rate: if row.0 > 0 { row.1 as f64 / row.0 as f64 } else { 0.0 },
        tp: row.1,
        sl: row.2,
        roi: row.3,
    }
}

pub async fn insert_trade(pool: &SqlitePool, t: &TradeRecord) {
    let pnl_pct = if t.pnl_pct.is_finite() { t.pnl_pct } else { 0.0 };

    let res = sqlx::query(
        "INSERT INTO trades
         (timestamp, token_addr, entry_price, exit_price, pnl_pct,
          exit_type, hold_secs, volume_entry, velocity_score, buyers_count, entry_score, hour_utc)
         VALUES (?,?,?,?,?,?,?,?,?,?,?,?)",
    )
    .bind(&t.timestamp)
    .bind(&t.token_addr)
    .bind(t.entry_price)
    .bind(t.exit_price)
    .bind(pnl_pct)
    .bind(&t.exit_type)
    .bind(t.hold_secs)
    .bind(t.volume_entry)
    .bind(t.velocity_score)
    .bind(t.buyers_count)
    .bind(t.entry_score)
    .bind(t.hour_utc)
    .execute(pool)
    .await;

    if let Err(e) = res {
        error!("Gagal insert_trade untuk {}: {}", t.token_addr, e);
    }
}

pub async fn query_win_rate_last_n(pool: &SqlitePool, n: i64) -> f64 {
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT CAST(SUM(CASE WHEN exit_type='TP' THEN 1 ELSE 0 END) AS REAL)
                / COUNT(*) as wr
         FROM (SELECT exit_type FROM trades
               ORDER BY id DESC LIMIT ?)",
    )
    .bind(n)
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    row.map(|r| r.0).unwrap_or(0.0)
}

#[allow(dead_code)]
pub async fn query_tp_rate_last_hour(pool: &SqlitePool) -> f64 {
    let row: Option<(f64,)> = sqlx::query_as(
        "SELECT CAST(SUM(CASE WHEN exit_type='TP' THEN 1 ELSE 0 END) AS REAL) / COUNT(*)
         FROM trades
         WHERE timestamp >= datetime('now', '-60 minutes')",
    )
    .fetch_optional(pool)
    .await
    .unwrap_or(None);
    row.map(|r| r.0).unwrap_or(0.0)
}

#[allow(dead_code)]
pub async fn insert_window_stats(
    pool: &SqlitePool,
    scanned: i32,
    passed: i32,
    passed_rate: f64,
    win_rate_30: f64,
    avg_velocity: f64,
    market_mode: &str,
) {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "INSERT INTO window_stats
         (timestamp, scanned, passed, passed_rate, win_rate_30, avg_velocity, market_mode)
         VALUES (?,?,?,?,?,?,?)",
    )
    .bind(now)
    .bind(scanned)
    .bind(passed)
    .bind(passed_rate)
    .bind(win_rate_30)
    .bind(avg_velocity)
    .bind(market_mode)
    .execute(pool)
    .await;

    if let Err(e) = res {
        error!("Gagal insert_window_stats: {}", e);
    }
}

#[allow(dead_code)]
pub async fn insert_virtual_topup(pool: &SqlitePool, amount_added: f64, balance_after: f64) {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO virtual_topups (timestamp, amount_added, balance_after)
         VALUES (?, ?, ?)",
    )
    .bind(now)
    .bind(amount_added)
    .bind(balance_after)
    .execute(pool)
    .await
    .unwrap();
}
