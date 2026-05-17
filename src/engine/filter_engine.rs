use crate::config::StrategyParameters;
use crate::queue::event_queue::{BotEvent, TokenData};
use crate::state::SharedState;
use crate::telegram::TelegramNotifier;
use crate::engine::dynamic_config::{SharedConfig, MarketMode};
use crate::storage::db;
use sqlx::SqlitePool;
use dashmap::{DashMap, DashSet};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{info, warn, debug};
use chrono::Utc;

#[derive(Debug)]
struct TokenActivity {
    pub total_volume: f64,
    pub buy_volume: f64,
    pub sell_volume: f64,
    pub half_volume: Option<f64>, // Snapshot pertengahan
    pub unique_buyers: DashSet<String>,
    pub latest_price: f64,
}

#[derive(Debug)]
pub struct ScoreBreakdown {
    pub volume_score:   f64,  // max 25 poin
    pub buyers_score:   f64,  // max 25 poin
    pub velocity_score: f64,  // max 25 poin
    pub momentum_score: f64,  // max 15 poin
    pub pressure_score: f64,  // max 10 poin
    pub late_dominance_score: f64, // max 30 poin
}

#[derive(Debug)]
pub struct TokenScore {
    pub total:    f64,   // 0.0 - 100.0
    pub breakdown: ScoreBreakdown,
}

impl TokenScore {
    pub fn calculate(
        total_volume: f64, 
        buyers: u32, 
        late_volume: f64, // velocity di prompt, diganti jadi late_volume
        momentum_pct: f64, 
        buy_vol: f64, 
        sell_vol: f64, 
        thresh: &crate::engine::dynamic_config::DynamicConfig
    ) -> Self {
        // Volume — semakin jauh di atas threshold, semakin tinggi skor
        let volume_ratio = if thresh.volume_thresh > 0.0 { total_volume / thresh.volume_thresh } else { 1.0 };
        let volume_score = match volume_ratio {
            r if r >= 3.0 => 15.0,   // 3× threshold → skor penuh (sebelumnya 25)
            r if r >= 2.0 => 12.0,   // (sebelumnya 20)
            r if r >= 1.5 => 9.0,    // (sebelumnya 15)
            r if r >= 1.0 => 5.0,    // (sebelumnya 8)
            _             => 0.0,
        };

        // Buyers — lebih banyak wallet unik = lebih organik
        let buyers_ratio = if thresh.buyers_thresh > 0 { buyers as f64 / thresh.buyers_thresh as f64 } else { 1.0 };
        let buyers_score = match buyers_ratio {
            r if r >= 3.0 => 25.0,
            r if r >= 2.0 => 20.0,
            r if r >= 1.5 => 14.0,
            r if r >= 1.0 => 7.0,
            _             => 0.0,
        };

        // Velocity — kecepatan di paruh kedua window
        let velocity_ratio = if thresh.velocity_thresh > 0.0 { late_volume / thresh.velocity_thresh } else { 1.0 };
        let velocity_score = match velocity_ratio {
            r if r >= 3.0 => 25.0,
            r if r >= 2.0 => 20.0,
            r if r >= 1.5 => 14.0,
            r if r >= 1.0 => 7.0,
            _             => 0.0,
        };

        // Momentum — seberapa kuat harga sudah naik
        let momentum_score = match momentum_pct {
            m if m >= 20.0 => 15.0,  // sangat kuat
            m if m >= 12.0 => 12.0,
            m if m >= 8.0  => 8.0,
            m if m >= 5.0  => 3.0,   // turunkan dari 4 ke 3
            m if m >= 3.0  => 0.0,   // 3% tidak dapat skor — hanya lolos filter minimum
            _              => 0.0,
        };

        // Pressure — rasio buy/sell
        let pressure_score = if sell_vol == 0.0 {
            10.0  // tidak ada sell → tekanan beli murni
        } else {
            let ratio = buy_vol / sell_vol;
            match ratio {
                r if r >= 3.0 => 10.0,
                r if r >= 2.0 => 8.0,
                r if r >= 1.5 => 5.0,
                r if r >= 1.2 => 2.0,
                _             => 0.0,
            }
        };

        // Tambah komponen baru: late_dominance (30 poin)
        // Seberapa dominan paruh kedua vs paruh pertama
        let late_ratio = if total_volume > 0.0 { late_volume / total_volume } else { 0.0 };
        let late_dominance_score = match late_ratio {
            r if r >= 0.65 => 30.0,  // 65%+ volume di paruh kedua → momentum masih kuat
            r if r >= 0.55 => 22.0,
            r if r >= 0.50 => 14.0,
            r if r >= 0.45 => 5.0,
            _              => 0.0,   // volume lebih banyak di paruh pertama → sudah lewat
        };

        let total = volume_score + buyers_score + velocity_score
                  + momentum_score + pressure_score + late_dominance_score;

        Self {
            total,
            breakdown: ScoreBreakdown {
                volume_score, buyers_score, velocity_score,
                momentum_score, pressure_score, late_dominance_score,
            }
        }
    }
}

pub struct FilterEngine {
    params: StrategyParameters,
    processed_tokens: Arc<DashSet<String>>, 
    activity_monitor: Arc<DashMap<String, TokenActivity>>,
    tx: mpsc::UnboundedSender<BotEvent>,
    state: SharedState,
    notifier: Option<TelegramNotifier>,
    config: SharedConfig,
    db: SqlitePool,
    last_velocities: Vec<f64>, // Untuk menghitung avg_velocity window
    last_regime: crate::engine::market_context::MarketRegime,
}

impl FilterEngine {
    pub fn new(
        params: StrategyParameters,
        tx: mpsc::UnboundedSender<BotEvent>,
        state: SharedState,
        notifier: Option<TelegramNotifier>,
        config: SharedConfig,
        db: SqlitePool,
    ) -> Self {
        Self {
            params,
            processed_tokens: Arc::new(DashSet::new()),
            activity_monitor: Arc::new(DashMap::new()),
            tx,
            state,
            notifier,
            config,
            db,
            last_velocities: Vec::new(),
            last_regime: crate::engine::market_context::MarketRegime::Unknown,
        }
    }

    pub async fn process_event(&mut self, event: BotEvent) {
        match event {
            BotEvent::NewToken(token) => {
                if !self.state.lock().await.is_running { return; }
                if self.processed_tokens.contains(&token.address) { return; }
                self.processed_tokens.insert(token.address.clone());
                
                debug!("[NEW_TOKEN] {} ({}) masuk.", token.symbol, token.address);

                let (window_scanned, window_age) = {
                    let mut s = self.state.lock().await;
                    s.tokens_scanned += 1;
                    s.total_scanned += 1;
                    s.window_scanned += 1;
                    s.market_ctx.add_token_birth();
                    (s.window_scanned, s.window_start.elapsed().as_secs())
                };

                self.activity_monitor.insert(token.address.clone(), TokenActivity {
                    total_volume: 0.0,
                    buy_volume: 0.0,
                    sell_volume: 0.0,
                    half_volume: None,
                    unique_buyers: DashSet::new(),
                    latest_price: 0.0,
                });

                // Flush stats setiap 100 token scanned ATAU setiap 10 menit
                if window_scanned >= 100 || window_age >= 600 {
                    self.flush_window_stats().await;
                }

                self.handle_new_token(token).await;
            }
            BotEvent::PriceUpdate { token_address, volume, sender, price, is_buy, .. } => {
                if let Some(mut activity) = self.activity_monitor.get_mut(&token_address) {
                    activity.total_volume += volume;
                    if is_buy {
                        activity.buy_volume += volume;
                        if sender != "Unknown" {
                            activity.unique_buyers.insert(sender);
                        }
                    } else {
                        activity.sell_volume += volume;
                    }
                    activity.latest_price = price;
                }
            }
            BotEvent::TokenHalfMatured(token) => {
                if let Some(mut activity) = self.activity_monitor.get_mut(&token.address) {
                    activity.half_volume = Some(activity.total_volume);
                }
            }
            BotEvent::TokenMatured(token) => {
                debug!("[EVENT_MATURED] Menerima event matured untuk {}.", token.symbol);
                let addr = token.address.clone();
                self.handle_token_matured(token).await;
                let _ = self.tx.send(BotEvent::Unsubscribe(addr));
            }
            BotEvent::SessionEnd => {
                let s = self.state.lock().await;
                info!("========= FILTER REPORT =========");
                info!("Total Tokens Scanned : {}", s.tokens_scanned);
                info!("Rejected (Volume)    : {}", s.rejected_volume);
                info!("Rejected (Holders)   : {}", s.rejected_holders);
                info!("Rejected (Momentum)  : {}", s.rejected_momentum);
                info!("Rejected (Velocity)  : {}", s.rejected_velocity);
                info!("Rejected (Pressure)  : {}", s.rejected_pressure);
                info!("Rejected (Score)     : {}", s.rejected_score);
                info!("Rejected (Schedule)  : {}", s.rejected_schedule);
                info!("Rejected (Pump-Dump) : {}", s.rejected_pump);
                info!("Passed Filter        : {}", s.passed_filter);
                info!("=================================");
            }
            _ => {}
        }
    }

    async fn handle_new_token(&mut self, token: TokenData) {
        let now = Utc::now();
        let age = (now - token.created_at).num_seconds() as u64;

        if age < self.params.token_age_seconds.min {
            let delay = self.params.token_age_seconds.min - age;
            let half_delay = delay / 2;
            info!("🔍 Token {} ({}) terdeteksi (Umur {}s). Menunggu aktivitas (Target >3 SOL, >6 Pembeli, {}s)...", 
                token.symbol, token.address, age, delay);

            let tx_clone = self.tx.clone();
            tokio::spawn(async move {
                sleep(Duration::from_secs(half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenHalfMatured(token.clone()));
                sleep(Duration::from_secs(delay - half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenMatured(token));
            });
        } else {
            // Jika token sudah "matang", langsung evaluasi
            self.handle_token_matured(token).await;
        }
    }

    async fn handle_token_matured(&mut self, token: TokenData) {
        let activity_data = {
            let activity = self.activity_monitor.get(&token.address);
            if activity.is_none() {
                warn!("⚠️ Monitor data hilang untuk {}", token.symbol);
                // Tambahkan ke metrik penolakan agar tidak ada gap statistik
                let mut s = self.state.lock().await;
                s.rejected_volume += 1; // Anggap gagal volume karena tidak ada data
                return;
            }
            let a = activity.unwrap();
            (a.unique_buyers.len(), a.total_volume, a.latest_price, a.half_volume, a.buy_volume, a.sell_volume)
        };

        let (holder_count, vol, latest_price, half_volume, buy_vol, sell_vol) = activity_data;
        
        // 0. Update Market Context (SEBELUM FILTER)
        // Kita hitung metrik untuk "belajar" dari market, terlepas dari apakah kita akan trade token ini.
        let momentum_pct = ((latest_price / token.initial_price) - 1.0) * 100.0;
        let early_volume = half_volume.unwrap_or(0.0);
        let velocity = vol - early_volume;

        debug!(
            "[MATURED_DATA] {}: vol={:.2}, early={:.2}, velocity={:.2}, half_v_is_none={}",
            token.symbol, vol, early_volume, velocity, half_volume.is_none()
        );

        {
            let mut s = self.state.lock().await;
            s.market_ctx.add_velocity(velocity);
            s.market_ctx.add_momentum_result(momentum_pct >= 3.0);
        }

        // AMBIL NILAI KONFIGURASI DULU, LALU LEPAS LOCK
        let (v_thresh, vol_thresh, buyers_thresh, is_paused, cfg_reason, cfg_mode) = {
            let cfg = self.config.read().await;
            (cfg.velocity_thresh, cfg.volume_thresh, cfg.buyers_thresh, cfg.mode == MarketMode::Pause, cfg.reason.clone(), cfg.mode.clone())
        };

        // 1. Filter Volume
        let mut fail_vol = false;
        let mut fail_holders = false;
        let mut fail_mom = false;
        let mut fail_pump = false;
        let mut fail_vel = false;
        let mut fail_press = false;
        let mut fail_score = false;

        if vol < vol_thresh {
            info!("❌ {} Ditolak: Vol {:.2} < {:.2} SOL", token.symbol, vol, vol_thresh);
            fail_vol = true;
        }

        // 2. Filter Unique Buyers
        if (holder_count as u32) < buyers_thresh {
            info!("❌ {} Ditolak: Buyers {} < {}", token.symbol, holder_count, buyers_thresh);
            fail_holders = true;
        }

        // 3. Price Momentum Check (Evaluasi Trading)
        if momentum_pct < 3.0 {
            info!("❌ {} Ditolak: Momentum Lemah ({:.2}%)", token.symbol, momentum_pct);
            fail_mom = true;
        }

        // 4. Filter Pump-and-Dump (Hold Time Minimum)
        // Hitung berapa cepat volume tumbuh di paruh PERTAMA vs KEDUA
        let early_velocity = early_volume / 30.0;   // SOL/detik paruh 1
        let late_velocity  = velocity   / 30.0;   // SOL/detik paruh 2

        // Token sehat: late_velocity >= early_velocity (momentum masih tumbuh)
        // Token pump: early_velocity >> late_velocity (sudah melambat)
        if early_velocity > late_velocity * 1.5 {
            info!("❌ {} Ditolak: Pump-and-Dump terdeteksi (Early Vel: {:.2}, Late Vel: {:.2})", token.symbol, early_velocity, late_velocity);
            fail_pump = true;
        }

        // 5. Velocity Check (sebelumnya 4)
        if velocity < v_thresh {
            info!("❌ {} Ditolak: Velocity {:.2} < {:.2}", token.symbol, velocity, v_thresh);
            fail_vel = true;
        }

        // 5. Buy/Sell Pressure Check
        let ratio = if sell_vol > 0.0 { buy_vol / sell_vol } else { 99.0 };
        info!("🔍 Pressure Check: {} | Ratio: {:.2} (B: {:.2} S: {:.2})", token.symbol, ratio, buy_vol, sell_vol);
        
        if ratio < 1.2 {
            info!("❌ {} Ditolak: Pressure (Ratio {:.2})", token.symbol, ratio);
            fail_press = true;
        }

        // 6. Confidence Score Check
        let score = {
            let cfg = self.config.read().await;
            TokenScore::calculate(vol, holder_count as u32, velocity, momentum_pct, buy_vol, sell_vol, &cfg)
        };

        let min_score = match cfg_mode {
            MarketMode::Hot     => 62.0, // turunkan sedikit
            MarketMode::Normal  => 68.0, // turunkan dari 72
            MarketMode::Strict  => 75.0, // turunkan dari 80
            MarketMode::Pause   => 85.0, // turunkan dari 88
        };

        if score.total < min_score {
            info!("❌ {} Ditolak: Skor {:.1} < {:.1} (V:{:.0} B:{:.0} Vel:{:.0} M:{:.0} P:{:.0} L:{:.0})",
                token.symbol, score.total, min_score,
                score.breakdown.volume_score,
                score.breakdown.buyers_score,
                score.breakdown.velocity_score,
                score.breakdown.momentum_score,
                score.breakdown.pressure_score,
                score.breakdown.late_dominance_score,
            );
            fail_score = true;
        }

        // Hard minimum per komponen (Aksi 4)
        if score.breakdown.velocity_score < 7.0 {
            info!("❌ {} Ditolak: Velocity score terlalu rendah ({:.0})", token.symbol, score.breakdown.velocity_score);
            fail_score = true;
        }

        if score.breakdown.momentum_score < 3.0 { // Tadi di Aksi 2, momentum >= 5% dapat 3.0. Jadi < 3.0 berarti < 5% momentum.
            info!("❌ {} Ditolak: Momentum score terlalu rendah ({:.0})", token.symbol, score.breakdown.momentum_score);
            fail_score = true;
        }

        // UPDATE METRICS (Satu kali lock untuk semua penolakan)
        {
            let mut s = self.state.lock().await;
            if fail_vol     { s.rejected_volume += 1; }
            if fail_holders { s.rejected_holders += 1; }
            if fail_mom     { s.rejected_momentum += 1; }
            if fail_pump    { s.rejected_pump += 1; }
            if fail_vel     { s.rejected_velocity += 1; }
            if fail_press   { s.rejected_pressure += 1; }
            if fail_score   { s.rejected_score += 1; }
        }

        if fail_vol || fail_holders || fail_mom || fail_pump || fail_vel || fail_press || fail_score {
            self.activity_monitor.remove(&token.address);
            return;
        }

        // LOLOS SEMUA FILTER
        {
            let mut s = self.state.lock().await;
            s.passed_filter += 1;
            s.total_passed += 1;
            s.window_passed += 1;
        }
        
        // PENTING: last_velocities dihapus, gunakan global_velocity dari MarketContext
        // self.last_velocities.push(velocity);
        // if self.last_velocities.len() > 100 { self.last_velocities.remove(0); }

        if is_paused {
            info!("⏸️ {} Lolos tapi Signal di-skip (Pause: {})", token.symbol, cfg_reason);
            self.activity_monitor.remove(&token.address);
            return;
        }

        info!("✅ BUY SIGNAL: {} | Skor: {:.1} | Vol: {:.2} SOL | Buyers: {}", token.symbol, score.total, vol, holder_count);
            
        if let Some(notifier) = &self.notifier {
            notifier.send_buy_alert(&token.address, velocity, holder_count as u32, score.total).await;
        }
            
        self.activity_monitor.remove(&token.address);
        
        if let Err(e) = self.tx.send(BotEvent::BuySignal { 
            token_address: token.address.clone(), 
            price: latest_price,
            volume_at_entry: vol,
            velocity_score: velocity,
            buyers_count: holder_count as u32,
            entry_score: score.total,
        }) {
            warn!("Gagal mengirim BuySignal: {}", e);
        }
    }

    async fn flush_window_stats(&mut self) {
        let tp_rate_1h = db::query_tp_rate_last_hour(&self.db).await;
        
        let (window_scanned, window_passed, total_scanned, total_passed, regime, current_ctx) = {
            let mut s = self.state.lock().await;
            s.market_ctx.update_metrics(tp_rate_1h);
            
            let res = (
                s.window_scanned, 
                s.window_passed, 
                s.total_scanned,
                s.total_passed,
                s.market_ctx.regime.clone(), 
                s.market_ctx.clone()
            );
            
            // RESET WINDOW
            s.window_scanned = 0;
            s.window_passed = 0;
            s.window_start = std::time::Instant::now();
            
            res
        };

        // 1. UPDATE KONFIGURASI SECARA INSTAN
        let snapshot = crate::engine::rolling_stats::MarketSnapshot::compute(&self.db).await;
        let new_cfg = crate::engine::dynamic_config::DynamicConfig::from_context(&snapshot, &current_ctx);
        
        {
            let mut cfg_write = self.config.write().await;
            if new_cfg.mode != cfg_write.mode {
                if let Some(notifier) = &self.notifier {
                    notifier.send_generic_alert(format!("🔄 *MODE BERUBAH*: {:?} — {}", new_cfg.mode, new_cfg.reason)).await;
                }
            }
            *cfg_write = new_cfg;
        }

        // 2. Kirim alert jika regime berubah (Insight metrik)
        if regime != self.last_regime {
            if let Some(notifier) = &self.notifier {
                let msg = format!(
                    "🌐 *MARKET REGIME BERUBAH*: {:?} -> *{:?}*\n\n\
                     📊 *Metrik Saat Ini:*\n\
                     • Birth Rate: {:.1}/m\n\
                     • Global Velocity: {:.2}\n\
                     • Momentum Fail: {:.1}%\n\
                     • TP Rate (1h): {:.1}%\n\n\
                     Filter bot otomatis menyesuaikan.",
                    self.last_regime, regime,
                    current_ctx.birth_rate_5m,
                    current_ctx.global_velocity,
                    current_ctx.momentum_fail_rate * 100.0,
                    current_ctx.tp_rate_1h * 100.0
                );
                notifier.send_generic_alert(msg).await;
            }
            self.last_regime = regime.clone();
        }
        
        let regime_str = format!("{:?}", regime);
        let window_passed_rate = if window_scanned > 0 { window_passed as f64 / window_scanned as f64 } else { 0.0 };
        let win_rate_30 = db::query_win_rate_last_n(&self.db, 30).await;
        
        // FIX 2: Gunakan Global Velocity dari MarketContext, bukan last_velocities yang cacat
        let avg_velocity = current_ctx.global_velocity;

        // SAFEGUARD: Detect possible velocity bug (Perketat threshold: 10 scanned)
        if window_scanned > 10 && avg_velocity == 0.0 {
            warn!("🚨 ALERT: Velocity data missing early in window (Scanned: {}, Vel: 0.0)", window_scanned);
        }

        let mode = format!("{:?}", self.config.read().await.mode);

        warn!(
            "[FLUSH] avg_velocity={:.4}, global_vel={:.4}, last_vel_len={}, mode={}",
            avg_velocity,
            current_ctx.global_velocity,
            self.last_velocities.len(),
            mode
        );

        db::insert_window_stats(
            &self.db,
            window_scanned as i32,
            window_passed as i32,
            window_passed_rate,
            win_rate_30,
            avg_velocity,
            &mode
        ).await;
        
        info!("📊 Window Stats: Scanned={}, Passed={}, Mode={}, Regime={}, GlobalVel={:.2}", 
            window_scanned, window_passed, mode, regime_str, avg_velocity);

        info!("📈 Total Session: Scanned={}, Passed={}", total_scanned, total_passed);
    }
}
