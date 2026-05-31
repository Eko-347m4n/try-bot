use sqlx::SqlitePool;
use serde::Serialize;
use tracing::info;

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct StrategyPerformance {
    pub strategy_id: String,
    pub trades: i64,
    pub win_rate: f64,
    pub realized_pnl: f64,
}

pub struct AnalyticsEngine {
    pool: SqlitePool,
}

impl AnalyticsEngine {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn print_performance_report(&self) {
        info!("========================================");
        info!("MULTI-STRATEGY PERFORMANCE REPORT");
        info!("========================================");

        // Fetch dari decision_traces untuk lihat berapa yang di-buy/reject per strategy
        use sqlx::Row;
        let rows = sqlx::query(
            r#"SELECT strategy_id, 
                      COUNT(*) as total_evals, 
                      SUM(CASE WHEN final_decision = 'BUY' THEN 1 ELSE 0 END) as buys 
               FROM decision_traces 
               GROUP BY strategy_id"#
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        for row in rows {
            let strategy_id: String = row.get(0);
            let total_evals: i64 = row.get(1);
            let buys: i64 = row.get(2);
            info!(
                "[{}] Evaluasi: {} | Buy: {} ({:.2}%)", 
                strategy_id, 
                total_evals, 
                buys, 
                if total_evals > 0 { (buys as f64 / total_evals as f64) * 100.0 } else { 0.0 }
            );
        }
        
        info!("========================================");
        
        // Asumsi data wallet snapshots akan disimpan, kita print placeholder
        info!("Wallet snapshots will provide full ROI per strategy in the future.");
    }
}
