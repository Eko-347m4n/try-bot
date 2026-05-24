use crate::engine::market_context::MarketContext;
use std::sync::Arc;
use tokio::sync::Mutex;
use std::time::Instant;

#[derive(Clone)]
pub struct SessionState {
    pub is_running: bool,
    pub market_events: u64,
    pub total_trades: u32,
    pub tp_hits: u32,
    pub sl_hits: u32,
    pub total_pnl_pct: f64,
    pub active_positions: u32,
    pub started_at: Option<Instant>,
    pub initial_balance: f64,
    pub virtual_balance: f64,
    pub last_ws_event: Instant,
    // market context
    pub market_ctx: MarketContext,
    // untuk insight market (Total Sesi)
    pub total_scanned: u32,
    pub total_passed: u32,
    // window reaktif (Direset setiap flush)
    pub window_scanned: u32,
    pub window_passed: u32,
    pub window_start: Instant,
    pub tokens_scanned: u32, // Deprecated, keeping for compatibility
    pub rejected_volume: u32,
    pub rejected_holders: u32,
    pub rejected_velocity: u32,
    pub rejected_extreme_velocity: u32,
    pub rejected_momentum: u32,
    pub rejected_pressure: u32,
    pub rejected_score: u32,
    pub rejected_schedule: u32,
    pub rejected_schedule_h07: u32,
    pub rejected_schedule_h12: u32,
    pub rejected_schedule_h19: u32,
    pub rejected_pump: u32,
    pub rejected_liquidity: u32, // New field for liquidity rejections
    pub rejected_spike: u32,
    pub passed_filter: u32,
    // parameters
    pub volume_threshold: f64,
    pub velocity_threshold: f64,
}

impl SessionState {
    pub fn uptime_minutes(&self) -> u64 {
        self.started_at.map(|s| s.elapsed().as_secs() / 60).unwrap_or(0)
    }

    pub fn win_rate(&self) -> f64 {
        let finished = self.tp_hits + self.sl_hits;
        if finished == 0 { 0.0 } else { self.tp_hits as f64 / finished as f64 }
    }

    pub fn total_roi_pct(&self) -> f64 {
        if self.initial_balance == 0.0 { return 0.0; }
        ((self.virtual_balance / self.initial_balance) - 1.0) * 100.0
    }
}

impl Default for SessionState {
    fn default() -> Self {
        Self {
            is_running: true,
            market_events: 0,
            total_trades: 0,
            tp_hits: 0,
            sl_hits: 0,
            total_pnl_pct: 0.0,
            active_positions: 0,
            started_at: Some(Instant::now()),
            initial_balance: 1.0, // Start with 1.0 SOL
            virtual_balance: 1.0, // Start with 1.0 SOL
            last_ws_event: Instant::now(),
            market_ctx: MarketContext::default(),
            total_scanned: 0,
            total_passed: 0,
            window_scanned: 0,
            window_passed: 0,
            window_start: Instant::now(),
            tokens_scanned: 0,
            rejected_volume: 0,
            rejected_holders: 0,
            rejected_velocity: 0,
            rejected_extreme_velocity: 0,
            rejected_momentum: 0,
            rejected_pressure: 0,
            rejected_score: 0,
            rejected_schedule: 0,
            rejected_schedule_h07: 0,
            rejected_schedule_h12: 0,
            rejected_schedule_h19: 0,
            rejected_pump: 0,
            rejected_liquidity: 0,
            rejected_spike: 0,
            passed_filter: 0,
            volume_threshold: 3.0,
            velocity_threshold: 0.5,
        }
    }
}

pub type SharedState = Arc<Mutex<SessionState>>;
