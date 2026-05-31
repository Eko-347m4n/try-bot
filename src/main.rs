mod analytics;
mod broker;
mod config;
mod core;
mod engine;
mod queue;
mod strategy;
mod stream;
mod tracker;
mod utils;
mod wallet;
mod state;
mod storage;
mod telegram;

use crate::config::{BotConfig, StrategyParameters};
use crate::engine::rolling_stats::MarketSnapshot;
use crate::engine::dynamic_config::{DynamicConfig, SharedConfig};
use crate::stream::pumpfun_listener::PumpfunListener;
use crate::queue::event_queue::BotEvent;
use crate::state::SessionState;
use crate::storage::db;
use crate::telegram::{TelegramNotifier, start_command_handler};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio::time::Duration;
use tracing::{info, warn};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    // Setup Log Rotation (File + Terminal)
    let file_appender = tracing_appender::rolling::daily("./logs", "bot.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false); // Matikan warna di file agar bersih
    
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout);

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::from_default_env().add_directive(tracing::Level::INFO.into()))
        .with(stdout_layer)
        .with(file_layer)
        .init();

    let _ = dotenvy::dotenv(); // Load .env file
    info!("Memulai pumpfun-quant-bot (Professional Version)...");

    let bot_config = BotConfig::default();
    let strategy_params = StrategyParameters::default();

    // Initialize Shared State
    let shared_state = Arc::new(Mutex::new(SessionState::default()));

    // Initialize DB
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "trades.db".to_string());
    let db_pool = db::init_db(&db_path).await;

    let analytics = crate::analytics::engine::AnalyticsEngine::new(db_pool.clone());
    analytics.print_performance_report().await;

    // Initialize Telegram Notifier (optional if keys not set)
    let telegram_token = std::env::var("TELOXIDE_TOKEN").unwrap_or_default();
    let chat_id = std::env::var("TELEGRAM_CHAT_ID").unwrap_or_default();
    
    let notifier = if !telegram_token.is_empty() && !chat_id.is_empty() {
        let n = TelegramNotifier::new(telegram_token.clone(), chat_id);
        start_command_handler(telegram_token, shared_state.clone(), db_pool.clone()).await;
        Some(n)
    } else {
        None
    };

    // Session Recovery
    let orphans = db::load_orphans(&db_pool).await;
    if !orphans.is_empty() {
        warn!("{} posisi orphan ditemukan dari sesi sebelumnya.", orphans.len());
        if let Some(n) = &notifier {
            n.send_generic_alert(format!("⚠️ *SESSION RECOVERY*: {} posisi orphan dari sesi sebelumnya ditandai sebagai STALE.", orphans.len())).await;
        }
        for orphan in orphans {
            db::insert_trade(&db_pool, &orphan).await;
            db::delete_open_position(&db_pool, &orphan.token_addr).await;
        }
    }

    // Initialize Dynamic Config
    let initial_snapshot = MarketSnapshot::compute(&db_pool).await;
    let initial_ctx = {
        let s = shared_state.lock().await;
        s.market_ctx.clone()
    };
    let shared_config: SharedConfig = Arc::new(RwLock::new(DynamicConfig::from_context(&initial_snapshot, &initial_ctx)));

    // Background Task: Update Config Setiap 1 Menit
    {
        let pool = db_pool.clone();
        let config = shared_config.clone();
        let state = shared_state.clone();
        let notifier_clone = notifier.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            let mut last_mode = {
                let cfg = config.read().await;
                cfg.mode.clone()
            };

            loop {
                interval.tick().await;
                let snapshot = MarketSnapshot::compute(&pool).await;
                let context = {
                    let s = state.lock().await;
                    s.market_ctx.clone()
                };
                let new_cfg = DynamicConfig::from_context(&snapshot, &context);

                if new_cfg.mode != last_mode {
                    if let Some(n) = &notifier_clone {
                        n.send_generic_alert(format!("🔄 *MODE BERUBAH*: {:?} — {}", new_cfg.mode, new_cfg.reason)).await;
                    }
                    last_mode = new_cfg.mode.clone();
                }
                *config.write().await = new_cfg;
            }
        });
    }

    // Background Task: Heartbeat Setiap Jam
    if let Some(n) = &notifier {
        let n_clone = n.clone();
        let s_clone = shared_state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
            loop {
                interval.tick().await;
                let s = s_clone.lock().await;
                let regime = format!("{:?}", s.market_ctx.regime);
                let msg = format!(
                    "💓 *HEARTBEAT*\n\n\
                     Regime: *{}*\n\
                     Uptime: {}m\n\
                     Trades: {}\n\
                     Win Rate: {:.1}%\n\
                     ROI: {:.2}%\n\
                     Active Pos: {}\n\
                     Balance: {:.3} SOL",
                    regime, s.uptime_minutes(), s.total_trades, s.win_rate() * 100.0,
                    s.total_roi_pct(), s.active_positions, s.virtual_balance
                );
                n_clone.send_generic_alert(msg).await;
            }
        });
    }

    // Background Task: Daily Summary (Midnight UTC)
    if let Some(n) = &notifier {
        let n_clone = n.clone();
        let pool = db_pool.clone();
        tokio::spawn(async move {
            loop {
                let now = chrono::Utc::now();
                let next_midnight = (now + chrono::Duration::days(1))
                    .date_naive().and_hms_opt(0, 0, 0).unwrap()
                    .and_utc();
                let wait = (next_midnight - now).to_std().unwrap_or_default();
                tokio::time::sleep(wait).await;

                let summary = db::query_daily_summary(&pool).await;
                let msg = format!(
                    "📅 *DAILY SUMMARY*\n\n\
                     Total Trades: {}\n\
                     Win Rate: {:.1}%\n\
                     TP: {} | SL: {}\n\
                     ROI Hari Ini: {:.2}%",
                    summary.trades, summary.win_rate * 100.0,
                    summary.tp, summary.sl, summary.roi
                );
                n_clone.send_generic_alert(msg).await;
            }
        });
    }

    // WebSocket Watchdog
    if let Some(n) = &notifier {
        let n_clone = n.clone();
        let s_clone = shared_state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let last = s_clone.lock().await.last_ws_event;
                if last.elapsed().as_secs() > 120 {
                    warn!("WS silence detected — {}s tanpa event.", last.elapsed().as_secs());
                    n_clone.send_generic_alert(format!("⚠️ *WS SILENCE*: Tidak ada event >120s ({}s). Kemungkinan koneksi zombie.", last.elapsed().as_secs())).await;
                }
            }
        });
    }

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (listener_tx, listener_rx) = mpsc::unbounded_channel();

    let (trace_tx, trace_rx) = mpsc::channel(10000);
    
    // Setup Async Batch Worker untuk SQLite Trace Logging
    let batch_worker = crate::storage::batch_worker::BatchWorker::new(db_pool.clone(), trace_rx);
    tokio::spawn(async move {
        batch_worker.run().await;
    });

    let mut dispatcher = crate::engine::dispatcher::Dispatcher::new(
        crate::strategy::builder::StrategyBuilder::build_all(notifier.clone())
            .into_iter()
            .map(|s| Box::new(s) as Box<dyn crate::strategy::instance::Strategy>)
            .collect(),
        trace_tx,
        tx.clone(),
        strategy_params
    );

    let listener = PumpfunListener::new(bot_config.websocket_url, tx.clone(), notifier.clone());
    tokio::spawn(async move {
        let _ = listener.start(listener_rx).await;
    });

    info!("Bot aktif. Menunggu event...");
    
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Sinyal shutdown diterima. Menghentikan bot...");
                // Note: SessionEnd event should still be handled, but in Multi-Strategy
                // it might just gracefully shutdown. For now, break directly.
                break;
            }
            Some(event) = rx.recv() => {
                // Update timestamp last event received
                {
                    let mut s = shared_state.lock().await;
                    s.last_ws_event = std::time::Instant::now();
                }

                if let BotEvent::Unsubscribe(_) = &event {
                    let _ = listener_tx.send(event);
                    continue;
                }

                let is_session_end = matches!(event, BotEvent::SessionEnd);
                dispatcher.process_event(event);
                
                // Update shared state with latest strategy statuses
                {
                    let mut s = shared_state.lock().await;
                    s.strategy_statuses = dispatcher.get_strategy_statuses();
                }

                if is_session_end { break; }
            }
        }
    }

    Ok(())
}
