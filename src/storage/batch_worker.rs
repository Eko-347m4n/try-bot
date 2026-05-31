use tokio::sync::mpsc;
use sqlx::SqlitePool;
use tracing::{error, info};
use crate::core::types::TraceRecord;
use chrono::Utc;

pub struct BatchWorker {
    pool: SqlitePool,
    rx: mpsc::Receiver<TraceRecord>,
}

impl BatchWorker {
    pub fn new(pool: SqlitePool, rx: mpsc::Receiver<TraceRecord>) -> Self {
        Self { pool, rx }
    }

    pub async fn run(mut self) {
        info!("Memulai Async Storage Batch Worker...");
        let mut buffer = Vec::new();
        const BATCH_SIZE: usize = 100;
        
        // Timeout untuk flush meskipun buffer belum penuh
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));

        loop {
            tokio::select! {
                record = self.rx.recv() => {
                    match record {
                        Some(trace) => {
                            buffer.push(trace);
                            if buffer.len() >= BATCH_SIZE {
                                self.flush(&mut buffer).await;
                            }
                        }
                        None => {
                            // Channel closed
                            info!("Trace channel closed. Flushing remaining records...");
                            self.flush(&mut buffer).await;
                            break;
                        }
                    }
                }
                _ = interval.tick() => {
                    if !buffer.is_empty() {
                        self.flush(&mut buffer).await;
                    }
                }
            }
        }
    }

    async fn flush(&self, buffer: &mut Vec<TraceRecord>) {
        // Karena SQLite lebih efisien dengan transaksi manual
        let mut tx = match self.pool.begin().await {
            Ok(tx) => tx,
            Err(e) => {
                error!("Gagal memulai transaksi untuk batch insert: {}", e);
                return;
            }
        };

        for record in buffer.iter() {
            match record {
                TraceRecord::Decision(dt) => {
                    let filters_json = serde_json::to_string(&dt.filters).unwrap_or_else(|_| "[]".to_string());
                    
                    let res = sqlx::query(
                        "INSERT INTO decision_traces (trace_id, strategy_id, token_addr, timestamp, filters_json, final_decision) 
                         VALUES (?, ?, ?, ?, ?, ?)"
                    )
                    .bind(dt.trace_id.to_string())
                    .bind(&dt.strategy_id)
                    .bind(&dt.token_address)
                    .bind(dt.timestamp.to_rfc3339())
                    .bind(filters_json)
                    .bind(&dt.final_decision)
                    .execute(&mut *tx)
                    .await;

                    if let Err(e) = res {
                        error!("Gagal insert decision_trace: {}", e);
                    }
                }
                TraceRecord::Trade(tt) => {
                    let now = Utc::now().to_rfc3339();
                    let res = sqlx::query(
                        "INSERT INTO trades (timestamp, strategy_id, token_addr, entry_price, exit_price, pnl_pct, exit_type, hold_secs) 
                         VALUES (?, ?, ?, ?, ?, ?, ?, ?)"
                    )
                    .bind(now)
                    .bind(&tt.strategy_id)
                    .bind(&tt.token_addr)
                    .bind(tt.entry_price)
                    .bind(tt.exit_price)
                    .bind(tt.pnl_pct)
                    .bind(&tt.exit_type)
                    .bind(tt.hold_secs)
                    .execute(&mut *tx)
                    .await;

                    if let Err(e) = res {
                        error!("Gagal insert trade: {}", e);
                    }
                }
            }
        }

        if let Err(e) = tx.commit().await {
            error!("Gagal commit batch transaksi: {}", e);
        } else {
            // Berhasil flush
            buffer.clear();
        }
    }
}
