use crate::queue::event_queue::BotEvent;
use crate::state::SharedState;
use crate::storage::db::{self, TradeRecord};
use crate::telegram::{TelegramNotifier, TradeResult};
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use tracing::{info, warn, error};
use std::collections::{HashMap, VecDeque};
use chrono::{DateTime, Utc, Timelike};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct Position {
    pub entry_price: f64,
    pub latest_price: f64,
    pub entry_time: DateTime<Utc>,
    pub last_update: Instant,
    pub volume_at_entry: f64,
    pub velocity_score: f64,
    pub buyers_count: u32,
    pub entry_score: f64,
    pub entry_size_sol: f64,
}

pub struct SimulationEngine {
    tx: mpsc::UnboundedSender<BotEvent>,
    open_positions: HashMap<String, Position>,
    closed_trades: Vec<f64>,
    last_prices: HashMap<String, f64>,
    hold_times_secs: Vec<i64>,
    state: SharedState,
    db: SqlitePool,
    notifier: Option<TelegramNotifier>,
    recent_outcomes: VecDeque<bool>, // true = TP, false = SL
    peak_roi: f64,
}

impl SimulationEngine {
    pub fn new(
        tx: mpsc::UnboundedSender<BotEvent>,
        state: SharedState,
        db: SqlitePool,
        notifier: Option<TelegramNotifier>,
    ) -> Self {
        Self {
            tx,
            open_positions: HashMap::new(),
            closed_trades: Vec::new(),
            last_prices: HashMap::new(),
            hold_times_secs: Vec::new(),
            state,
            db,
            notifier,
            recent_outcomes: VecDeque::with_capacity(20),
            peak_roi: 0.0,
        }
    }

    pub async fn process_event(&mut self, event: BotEvent) {
        match event {
            BotEvent::Heartbeat | BotEvent::NewToken(_) => {
                let mut s = self.state.lock().await;
                s.last_ws_event = std::time::Instant::now();
            }
            BotEvent::BuySignal { token_address, price, volume_at_entry, velocity_score, buyers_count, entry_score } => {
                self.virtual_buy(token_address, price, volume_at_entry, velocity_score, buyers_count, entry_score).await;
            }
            BotEvent::PriceUpdate { token_address, price, .. } => {
                {
                    let mut s = self.state.lock().await;
                    s.market_events += 1;
                    s.last_ws_event = std::time::Instant::now();
                }
                
                if let Some(pos) = self.open_positions.get_mut(&token_address) {
                    pos.latest_price = price;
                    pos.last_update = std::time::Instant::now();
                    info!("📈 Update Harga ({}): {:.10} SOL", token_address, price);
                }
                self.last_prices.insert(token_address.clone(), price);
                self.check_exit_conditions(token_address, price).await;
            }
            BotEvent::SessionEnd => {
                self.handle_session_end().await;
            }
            _ => {}
        }
        
        self.check_stale_positions().await;
    }

    async fn virtual_buy(&mut self, address: String, entry_price: f64, volume_at_entry: f64, velocity_score: f64, buyers_count: u32, entry_score: f64) {
        if self.open_positions.contains_key(&address) { return; }

        const MAX_POSITIONS: usize = 5;
        const MIN_VIRTUAL_BALANCE: f64 = 0.2; // SOL

        let mut s = self.state.lock().await;

        if s.active_positions >= MAX_POSITIONS as u32 {
            warn!("Max posisi tercapai ({}), skip BuySignal untuk {}", MAX_POSITIONS, address);
            return;
        }

        if s.virtual_balance < MIN_VIRTUAL_BALANCE {
            info!("🔄 Auto-Topup: Saldo virtual {:.3} SOL menipis. Menambah 1.0 SOL.", s.virtual_balance);
            let topup_amount = 1.0;
            s.virtual_balance += topup_amount;
            
            // Catat ke database
            db::insert_virtual_topup(&self.db, topup_amount, s.virtual_balance).await;

            if let Some(notifier) = &self.notifier {
                notifier.send_generic_alert(format!("⚠️ *AUTO-TOPUP*: Saldo virtual menipis. Menambahkan 1.0 SOL. Saldo baru: {:.3} SOL", s.virtual_balance)).await;
            }
        }

        const BOOTSTRAP_TRADES: u32 = 5;
        const BOOTSTRAP_MAX_SOL: f64 = 0.05;

        let entry_size = if s.total_trades < BOOTSTRAP_TRADES {
            BOOTSTRAP_MAX_SOL
        } else {
            0.1 // Default entry size
        };

        if s.total_trades < BOOTSTRAP_TRADES {
            info!("🚀 BOOTSTRAP BUY ({}): Menggunakan size kecil {:.3} SOL untuk mengumpulkan data.", s.total_trades + 1, entry_size);
        }

        s.virtual_balance -= entry_size;
        s.active_positions += 1;
        s.total_trades += 1;
        
        let record = TradeRecord {
            timestamp: Utc::now().to_rfc3339(),
            token_addr: address.clone(),
            entry_price,
            exit_price: 0.0,
            pnl_pct: 0.0,
            exit_type: "OPEN".to_string(),
            hold_secs: 0,
            volume_entry: volume_at_entry,
            velocity_score,
            buyers_count,
            entry_score,
            hour_utc: Utc::now().hour(),
        };

        db::insert_open_position(&self.db, &record).await;

        let pos = Position {
            entry_price,
            latest_price: entry_price,
            entry_time: Utc::now(),
            last_update: Instant::now(),
            volume_at_entry,
            velocity_score,
            buyers_count,
            entry_score,
            entry_size_sol: entry_size,
        };

        info!("🟢 VIRTUAL BUY: {} | Balance: {:.3} SOL | Size: {:.2}", address, s.virtual_balance, entry_size);
        self.open_positions.insert(address, pos);
    }

    async fn check_stale_positions(&mut self) {
        let mut to_close = Vec::new();
        for (addr, pos) in &self.open_positions {
            if pos.last_update.elapsed().as_secs() > 120 {
                to_close.push((addr.clone(), pos.latest_price));
            }
        }

        for (addr, price) in to_close {
            warn!("Stale price: {} — force close", addr);
            self.close_one_position(addr, price, "STALE").await;
        }
    }

    async fn check_exit_conditions(&mut self, address: String, current_price: f64) {
        let mut exit_type = None;
        if let Some(pos) = self.open_positions.get(&address) {
            let tp = pos.entry_price * 1.15;
            let sl = pos.entry_price * 0.92;
            if current_price >= tp { exit_type = Some("TP".to_string()); }
            else if current_price <= sl { exit_type = Some("SL".to_string()); }
        }

        if let Some(et) = exit_type {
            self.close_one_position(address, current_price, &et).await;
        }
    }

    async fn close_one_position(&mut self, address: String, current_price: f64, exit_type: &str) {
        if let Some(pos) = self.open_positions.remove(&address) {
            db::delete_open_position(&self.db, &address).await;
            let _ = self.tx.send(BotEvent::Unsubscribe(address.clone()));
            
            let pnl_percent = if pos.entry_price > 1e-12 {
                (current_price - pos.entry_price) / pos.entry_price * 100.0
            } else {
                error!("CRITICAL: entry_price adalah nol untuk {}. PNL tidak dapat dihitung.", address);
                0.0
            };
            let hold_time = (Utc::now() - pos.entry_time).num_seconds();

            self.closed_trades.push(pnl_percent);
            self.hold_times_secs.push(hold_time);
            
            let mut s = self.state.lock().await;
            s.active_positions = s.active_positions.saturating_sub(1);
            
            if exit_type == "TP" { 
                s.tp_hits += 1; 
                self.recent_outcomes.push_back(true);
            } else { 
                s.sl_hits += 1; 
                self.recent_outcomes.push_back(false);
            }
            if self.recent_outcomes.len() > 20 { self.recent_outcomes.pop_front(); }
            
            info!(
                "{} Posisi Ditutup ({}): Entry: {:.6} SOL, Exit: {:.6} SOL, PNL: {:.2}%",
                if pnl_percent >= 0.0 {"✅"} else {"🔻"},
                exit_type, pos.entry_price, current_price, pnl_percent
            );
			
            let pnl_sol = pos.entry_size_sol * (1.0 + pnl_percent / 100.0);
            s.virtual_balance += pnl_sol;
            
            // Sync total_pnl_pct dengan ROI baru
            let current_roi = s.total_roi_pct();
            s.total_pnl_pct = current_roi;
            
            if s.total_pnl_pct > self.peak_roi { self.peak_roi = s.total_pnl_pct; }
            drop(s);

            self.check_anomalies(current_roi).await;

            let record = TradeRecord {
                timestamp: Utc::now().to_rfc3339(),
                token_addr: address.clone(),
                entry_price: pos.entry_price,
                exit_price: current_price,
                pnl_pct: pnl_percent,
                exit_type: exit_type.to_string(),
                hold_secs: hold_time,
                volume_entry: pos.volume_at_entry,
                velocity_score: pos.velocity_score,
                buyers_count: pos.buyers_count,
                entry_score: pos.entry_score,
                hour_utc: Utc::now().hour(),
            };
            db::insert_trade(&self.db, &record).await;

            if let Some(notifier) = &self.notifier {
                notifier.send_trade_alert(&TradeResult {
                    token_addr: address,
                    pnl_pct: pnl_percent,
                    hold_secs: hold_time,
                    exit_type: exit_type.to_string(),
                    session_roi: current_roi,
                }).await;
            }
        }
    }

    async fn check_anomalies(&self, current_roi: f64) {
        if let Some(notifier) = &self.notifier {
            let sl_streak = self.recent_outcomes.iter().rev().take_while(|&&w| !w).count();
            if sl_streak >= 4 {
                notifier.send_generic_alert(format!("⚠️ *ANOMALI*: {} SL berturut-turut — market mungkin sedang dump.", sl_streak)).await;
            }

            if self.peak_roi > 10.0 {
                let drawdown = self.peak_roi - current_roi;
                if drawdown > 30.0 {
                    notifier.send_generic_alert(format!("⚠️ *DRAWDOWN TINGGI*: {:.1}% dari peak ROI {:.2}% — pertimbangkan pause manual.", drawdown, self.peak_roi)).await;
                }
            }
        }
    }

    async fn handle_session_end(&mut self) {
        info!("🛑 Menutup sesi... Menganalisis hasil perdagangan.");
        
        let mut floating_pnl = 0.0;
        let mut open_trade_count = 0;

        for (addr, pos) in self.open_positions.drain() {
            let current_price = pos.latest_price;
            let pnl = (current_price - pos.entry_price) / pos.entry_price * 100.0;
            tracing::warn!("Unclosed position: {} | entry: {:.10} | current: {:.10} | pnl: {:.2}%",
                addr, pos.entry_price, current_price, pnl
            );
            floating_pnl += pnl;
            open_trade_count += 1;
        }

        let s = self.state.lock().await;
        let total_pnl = s.total_pnl_pct + floating_pnl;
        let trade_count = s.total_trades + open_trade_count;
        let win_rate = if trade_count > 0 {
            (s.tp_hits as f64 / trade_count as f64) * 100.0
        } else {
            0.0
        };

        let avg_hold_time = if !self.hold_times_secs.is_empty() {
            self.hold_times_secs.iter().sum::<i64>() as f64 / self.hold_times_secs.len() as f64
        } else {
            0.0
        };

        info!("================ REPORT ================");
        info!("Market Events: {}", s.market_events);
        info!("Total Trades : {}", trade_count);
        info!("Win Rate     : {:.2}%", win_rate);
        info!("TP Hits      : {}", s.tp_hits);
        info!("SL Hits      : {}", s.sl_hits);
        info!("Avg Hold Time: {:.1}s", avg_hold_time);
        info!("Total ROI    : {:.2}%", total_pnl);
        info!("Balance      : {:.3} SOL", s.virtual_balance);
        info!("========================================");
    }
}
