use crate::storage::db;
use sqlx::SqlitePool;

#[derive(Debug, Clone)]
pub struct MarketSnapshot {
    pub win_rate_30:    f64,  // WR 30 trade terakhir
}

impl MarketSnapshot {
    pub async fn compute(pool: &SqlitePool) -> Self {
        Self {
            win_rate_30:     db::query_win_rate_last_n(pool, 30).await,
        }
    }
}
