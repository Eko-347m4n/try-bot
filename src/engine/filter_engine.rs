#![allow(dead_code)]
use crate::config::StrategyParameters;
use crate::engine::dynamic_config::{MarketMode, SharedConfig};
use crate::queue::event_queue::{BotEvent, TokenData};
use crate::state::SharedState;
use crate::storage::db;
use crate::telegram::TelegramNotifier;
use chrono::{Timelike, Utc};
use dashmap::{DashMap, DashSet};
use sqlx::SqlitePool;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::{debug, info, warn};

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
    pub volume_score: f64,             // max 15 poin
    pub buyers_score: f64,             // max 25 poin
    pub late_velocity_score: f64,      // turun jadi 15 poin
    pub momentum_score: f64,           // max 15 poin
    pub pressure_score: f64,           // max 10 poin
    pub early_acceleration_score: f64, // BARU, 30 poin
}

#[derive(Debug)]
pub struct TokenScore {
    pub total: f64, // 0.0 - 100.0
    pub breakdown: ScoreBreakdown,
}

impl TokenScore {
    #[allow(clippy::too_many_arguments)]
    pub fn calculate(
        total_volume: f64,
        buyers: u32,
        late_volume: f64,
        early_volume: f64, // PARAMETER BARU
        momentum_pct: f64,
        buy_vol: f64,
        sell_vol: f64,
        thresh: &crate::engine::dynamic_config::DynamicConfig,
    ) -> Self {
        // Volume — semakin jauh di atas threshold, semakin tinggi skor
        let volume_ratio = if thresh.volume_thresh > 0.0 {
            total_volume / thresh.volume_thresh
        } else {
            1.0
        };
        let volume_score = match volume_ratio {
            r if r >= 3.0 => 15.0,
            r if r >= 2.0 => 12.0,
            r if r >= 1.5 => 9.0,
            r if r >= 1.0 => 5.0,
            _ => 0.0,
        };

        // Buyers — lebih banyak wallet unik = lebih organik
        let buyers_ratio = if thresh.buyers_thresh > 0 {
            buyers as f64 / thresh.buyers_thresh as f64
        } else {
            1.0
        };
        let buyers_score = match buyers_ratio {
            r if r >= 3.0 => 25.0,
            r if r >= 2.0 => 20.0,
            r if r >= 1.5 => 14.0,
            r if r >= 1.0 => 7.0,
            _ => 0.0,
        };

        // Late Velocity — kecepatan di paruh kedua window (bobot diturunkan)
        let late_velocity_ratio = if thresh.velocity_thresh > 0.0 {
            late_volume / thresh.velocity_thresh
        } else {
            1.0
        };
        let late_velocity_score = match late_velocity_ratio {
            r if r >= 2.5 => 15.0, // Maks 15 poin
            r if r >= 1.5 => 10.0,
            r if r >= 1.0 => 5.0,
            _ => 0.0,
        };

        // Momentum — seberapa kuat harga sudah naik (Price)
        // + Bonus untuk "Quiet Accumulation" (Volume Momentum)
        let mut momentum_score = match momentum_pct {
            m if m >= 20.0 => 15.0,
            m if m >= 12.0 => 12.0,
            m if m >= 8.0 => 8.0,
            m if m >= 3.0 => 3.0,
            _ => 0.0,
        };

        // Early Acceleration Score (BARU) — Volume di paruh pertama
        let early_accel_ratio = if thresh.volume_thresh > 0.0 {
            early_volume / thresh.volume_thresh
        } else {
            0.0
        };
        let early_acceleration_score = match early_accel_ratio {
            r if r >= 2.5 => 30.0, // Volume 2.5x threshold di 30 detik pertama
            r if r >= 1.5 => 20.0,
            r if r >= 0.7 => 10.0, // Bahkan volume < threshold pun dapat skor jika ada
            _ => 0.0,
        };

        // Quiet Accumulation Check: Jika price belum bergerak, tapi volume akselerasi sangat kuat di awal
        // Ini adalah kandidat entry terbaik (Early Entry)
        // Menggunakan early_acceleration_score sebagai pengganti dominasi akhir
        if momentum_score < 3.0 && early_acceleration_score >= 10.0 {
            momentum_score += early_acceleration_score * 0.5; // Bonus dari akselerasi awal
            if momentum_score > 15.0 {
                momentum_score = 15.0;
            } // Max 15 poin
        }

        // Pressure — rasio buy/sell
        let pressure_score = if sell_vol == 0.0 {
            10.0
        } else {
            let ratio = buy_vol / sell_vol;
            match ratio {
                r if r >= 3.0 => 10.0,
                r if r >= 2.0 => 8.0,
                r if r >= 1.5 => 5.0,
                r if r >= 1.2 => 2.0,
                _ => 0.0,
            }
        };

        // late_dominance_score dihapus

        let total = volume_score
            + buyers_score
            + late_velocity_score
            + momentum_score
            + pressure_score
            + early_acceleration_score; // Update total score

        Self {
            total,
            breakdown: ScoreBreakdown {
                volume_score,
                buyers_score,
                late_velocity_score,
                momentum_score,
                pressure_score,
                early_acceleration_score,
            },
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
                if !self.state.lock().await.is_running {
                    return;
                }
                if self.processed_tokens.contains(&token.address) {
                    return;
                }
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

                self.activity_monitor.insert(
                    token.address.clone(),
                    TokenActivity {
                        total_volume: 0.0,
                        buy_volume: 0.0,
                        sell_volume: 0.0,
                        half_volume: None,
                        unique_buyers: DashSet::new(),
                        latest_price: 0.0,
                    },
                );

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
                let is_passed = self.handle_token_matured(token).await;
                if !is_passed {
                    let _ = self.tx.send(BotEvent::Unsubscribe(addr));
                }
            }
            BotEvent::SessionEnd => {
                let s = self.state.lock().await;
                info!("========= FILTER REPORT =========");
                info!("Total Tokens Scanned : {}", s.tokens_scanned);
                info!("Rejected (Volume)    : {}", s.rejected_volume);
                info!("Rejected (Holders)   : {}", s.rejected_holders);
                info!("Rejected (Momentum)  : {}", s.rejected_momentum);
                info!("Rejected (Velocity)  : {}", s.rejected_velocity);
                info!("Rejected (Ext Vel)   : {}", s.rejected_extreme_velocity);
                info!("Rejected (Pressure)  : {}", s.rejected_pressure);
                info!("Rejected (Score)     : {}", s.rejected_score);
                info!(
                    "Rejected (Schedule)  : {} (H07: {}, H12: {}, H19: {})",
                    s.rejected_schedule, s.rejected_schedule_h07, s.rejected_schedule_h12, s.rejected_schedule_h19
                );
                info!("Rejected (Pump-Dump) : {}", s.rejected_pump);
                info!("Rejected (Liquidity) : {}", s.rejected_liquidity);
                info!("Rejected (Spike)     : {}", s.rejected_spike);
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
            info!(
                "🔍 Token {} ({}) terdeteksi (Umur {}s). Menunggu aktivitas (Target >3 SOL, >6 Pembeli, {}s)...",
                token.symbol, token.address, age, delay
            );

            let tx_clone = self.tx.clone();
            tokio::spawn(async move {
                sleep(Duration::from_secs(half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenHalfMatured(token.clone()));
                sleep(Duration::from_secs(delay - half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenMatured(token));
            });
        } else {
            // Jika token sudah "matang", langsung evaluasi
            let addr = token.address.clone();
            let is_passed = self.handle_token_matured(token).await;
            if !is_passed {
                let _ = self.tx.send(BotEvent::Unsubscribe(addr));
            }
        }
    }

    async fn handle_token_matured(&mut self, token: TokenData) -> bool {
        // 0. Filter Likuiditas Minimum
        if token.initial_liquidity < 10.0 {
            info!(
                "❌ {} Ditolak: Likuiditas awal {:.2} SOL < 10.0 SOL",
                token.symbol, token.initial_liquidity
            );
            let mut s = self.state.lock().await;
            s.rejected_liquidity += 1;
            self.activity_monitor.remove(&token.address);
            return false;
        }

        let activity_data = {
            let activity = self.activity_monitor.get(&token.address);
            if activity.is_none() {
                warn!("⚠️ Monitor data hilang untuk {}", token.symbol);
                // Tambahkan ke metrik penolakan agar tidak ada gap statistik
                let mut s = self.state.lock().await;
                s.rejected_volume += 1; // Anggap gagal volume karena tidak ada data
                return false;
            }
            let a = activity.unwrap();
            (
                a.unique_buyers.len(),
                a.total_volume,
                a.latest_price,
                a.half_volume,
                a.buy_volume,
                a.sell_volume,
            )
        };

        let (holder_count, vol, latest_price, half_volume, buy_vol, sell_vol) = activity_data;

        // 0. Update Market Context (SEBELUM FILTER)
        // Kita hitung metrik untuk "belajar" dari market, terlepas dari apakah kita akan trade token ini.
        let momentum_pct = if token.initial_price > 0.0 {
            ((latest_price / token.initial_price) - 1.0) * 100.0
        } else {
            0.0
        };
        let early_volume = half_volume.unwrap_or(0.0);
        let velocity = vol - early_volume;

        debug!(
            "[MATURED_DATA] {}: vol={:.2}, early={:.2}, velocity={:.2}, half_v_is_none={}",
            token.symbol,
            vol,
            early_volume,
            velocity,
            half_volume.is_none()
        );

        {
            let mut s = self.state.lock().await;
            s.market_ctx.add_velocity(velocity);
            s.market_ctx.add_momentum_result(momentum_pct >= 3.0);
        }

        // --- Filter Lonjakan Kecepatan ---
        let global_velocity = self.state.lock().await.market_ctx.global_velocity;
        if global_velocity > 0.5 && velocity > 3.0 * global_velocity {
            // global_velocity > 0.5 untuk menghindari false positive pada pasar yang sangat sepi
            info!(
                "❌ {} Ditolak: Lonjakan Kecepatan (Token {:.2} SOL/30s > 3x Global {:.2} SOL/30s)",
                token.symbol, velocity, global_velocity
            );
            let mut s = self.state.lock().await;
            s.rejected_spike += 1;
            self.activity_monitor.remove(&token.address);
            return false;
        }
        // --- Akhir Filter Lonjakan Kecepatan ---

        // 1. Confidence Score Check (Dihitung di awal agar bisa digunakan di filter)
        let score = {
            let cfg = self.config.read().await;
            TokenScore::calculate(
                vol,
                holder_count as u32,
                velocity,
                early_volume,
                momentum_pct,
                buy_vol,
                sell_vol,
                &cfg,
            )
        };

        // 2. AMBIL NILAI KONFIGURASI
        let (v_thresh, max_v_thresh, vol_thresh, buyers_thresh, is_paused, cfg_reason, cfg_mode) = {
            let cfg = self.config.read().await;
            (
                cfg.velocity_thresh,
                cfg.max_velocity_thresh,
                cfg.volume_thresh,
                cfg.buyers_thresh,
                cfg.mode == MarketMode::Pause,
                cfg.reason.clone(),
                cfg.mode.clone(),
            )
        };

        // --- Filter Eksklusif Mode Hot ---
        let mut fail_mode = false;
        if cfg_mode != MarketMode::Hot {
            info!("❌ {} Ditolak: Bukan Market Hot (Mode Saat Ini: {:?})", token.symbol, cfg_mode);
            fail_mode = true;
        }
        // --- Akhir Filter Eksklusif Mode Hot ---

        // 3. Filter Volume
        let mut fail_vol = false;
        let mut fail_holders = false;
        let mut fail_mom = false;
        let mut fail_vel = false;
        let mut fail_extreme_vel = false;
        let mut fail_press = false;
        let mut fail_score = false;
        let mut fail_schedule = false;
        let mut nearest_dead_hour: u32 = 0;

        // 3a. Time Filter: Blackout Window Check
        let now = Utc::now();
        let current_hour = now.hour();
        let current_min = now.minute();
        let current_total_min = current_hour * 60 + current_min;

        for &dead_hour in &self.params.blackout_hours {
            let dead_total_min = dead_hour * 60;
            // Hitung selisih menit (circular 24h)
            let diff = current_total_min.abs_diff(dead_total_min);

            // Handle cross-day (misal jam 23:55 vs jam 00:05)
            let diff = std::cmp::min(diff, 1440 - diff);

            if diff <= self.params.blackout_window_minutes {
                info!(
                    "❌ {} Ditolak: Blackout Window (current={:02}:{:02} UTC, nearest_dead_hour={:02}:00 UTC)",
                    token.symbol, current_hour, current_min, dead_hour
                );
                fail_schedule = true;
                nearest_dead_hour = dead_hour;
                break;
            }
        }

        if vol < vol_thresh {
            info!("❌ {} Ditolak: Vol {:.2} < {:.2} SOL", token.symbol, vol, vol_thresh);
            fail_vol = true;
        }

        // 4. Filter Unique Buyers
        if (holder_count as u32) < buyers_thresh {
            info!("❌ {} Ditolak: Buyers {} < {}", token.symbol, holder_count, buyers_thresh);
            fail_holders = true;
        }

        // 5. Momentum Check (Evaluasi Trading) - Diperbarui untuk fokus pada Early Acceleration
        // Lolos jika Skor Momentum (dari Price) >= 3.0 ATAU Early Acceleration sangat kuat
        if score.breakdown.momentum_score < 3.0 {
            let early_momentum_strong = score.breakdown.early_acceleration_score >= 20.0 && holder_count > 5;

            if !early_momentum_strong {
                info!(
                    "❌ {} Ditolak: Momentum Lemah (Price: {:.2}%, Score M: {:.0})",
                    token.symbol, momentum_pct, score.breakdown.momentum_score
                );
                fail_mom = true;
            } else {
                info!(
                    "⚡ {} Early Accumulation Terdeteksi (Price: {:.2}%, Early Accel Score: {:.1})",
                    token.symbol, momentum_pct, score.breakdown.early_acceleration_score
                );
            }
        }

        // 6. Filter Pump-and-Dump (Hold Time Minimum) -- DIKOMENTARI: Logika ini menghukum early acceleration
        // Hitung berapa cepat volume tumbuh di paruh PERTAMA vs KEDUA
        // let early_velocity = early_volume / 30.0;   // SOL/detik paruh 1
        // let late_velocity  = velocity   / 30.0;   // SOL/detik paruh 2

        // // Token sehat: late_velocity >= early_velocity (momentum masih tumbuh)
        // // Token pump: early_velocity >> late_velocity (sudah melambat)
        // if early_velocity > late_velocity * 1.5 {
        //     info!("❌ {} Ditolak: Pump-and-Dump terdeteksi (Early Vel: {:.2}, Late Vel: {:.2})", token.symbol, early_velocity, late_velocity);
        //     fail_pump = true;
        // }

        // 7. Velocity Check
        if velocity < v_thresh {
            info!("❌ {} Ditolak: Velocity {:.2} < {:.2}", token.symbol, velocity, v_thresh);
            fail_vel = true;
        }

        if velocity > max_v_thresh {
            info!(
                "❌ {} Ditolak: Volatilitas Ekstrem ({:.2} > {:.2})",
                token.symbol, velocity, max_v_thresh
            );
            fail_extreme_vel = true;
        }

        // 8. Buy/Sell Pressure Check
        let ratio = if sell_vol > 0.0 { buy_vol / sell_vol } else { 99.0 };
        info!(
            "🔍 Pressure Check: {} | Ratio: {:.2} (B: {:.2} S: {:.2})",
            token.symbol, ratio, buy_vol, sell_vol
        );

        if ratio < 1.2 {
            info!("❌ {} Ditolak: Pressure (Ratio {:.2})", token.symbol, ratio);
            fail_press = true;
        }

        // 9. Total Score Threshold Check
        let mut min_score = match cfg_mode {
            MarketMode::Hot => 60.0,
            MarketMode::Normal => 65.0,
            MarketMode::Strict => 70.0,
            MarketMode::Pause => 80.0,
        };

        // BOOTSTRAP MODE: Jika trade masih sedikit, longgarkan min_score
        const BOOTSTRAP_TRADES: u32 = 5;
        let total_trades = self.state.lock().await.total_trades;
        if total_trades < BOOTSTRAP_TRADES {
            min_score -= 10.0;
            info!(
                "🚀 BOOTSTRAP MODE ({}): Melonggarkan min_score ke {:.1} SOL untuk mengumpulkan data awal.",
                total_trades + 1,
                min_score
            );
        }

        if score.total < min_score {
            info!(
                "❌ {} Ditolak: Skor {:.1} < {:.1} (V:{:.0} B:{:.0} LateVel:{:.0} M:{:.0} P:{:.0} EarlyAccel:{:.0})",
                token.symbol,
                score.total,
                min_score,
                score.breakdown.volume_score,
                score.breakdown.buyers_score,
                score.breakdown.late_velocity_score, // Renamed
                score.breakdown.momentum_score,
                score.breakdown.pressure_score,
                score.breakdown.early_acceleration_score, // New field
            );
            fail_score = true;
        }

        {
            let mut s = self.state.lock().await;
            if fail_mode {
                s.rejected_market_mode += 1;
            }
            if fail_vol {
                s.rejected_volume += 1;
            }
            if fail_holders {
                s.rejected_holders += 1;
            }
            if fail_mom {
                s.rejected_momentum += 1;
            }
            if fail_vel {
                s.rejected_velocity += 1;
            }
            if fail_extreme_vel {
                s.rejected_extreme_velocity += 1;
            }
            if fail_press {
                s.rejected_pressure += 1;
            }
            if fail_score {
                s.rejected_score += 1;
            }

            if fail_schedule {
                s.rejected_schedule += 1;
                match nearest_dead_hour {
                    7 => s.rejected_schedule_h07 += 1,
                    12 => s.rejected_schedule_h12 += 1,
                    19 => s.rejected_schedule_h19 += 1,
                    _ => {} // Jam blackout kustom lainnya tidak dicatat granular saat ini
                }
            }
        }

        if fail_mode
            || fail_vol
            || fail_holders
            || fail_mom
            || fail_vel
            || fail_extreme_vel
            || fail_press
            || fail_score
            || fail_schedule
        {
            self.activity_monitor.remove(&token.address);
            return false;
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
            return false;
        }

        if latest_price <= 1e-12 {
            warn!("⚠️ {} Lolos tapi Signal di-skip karena harga belum valid (0.0)", token.symbol);
            self.activity_monitor.remove(&token.address);
            return false;
        }

        info!(
            "✅ BUY SIGNAL: {} | Skor: {:.1} | Vol: {:.2} SOL | Buyers: {}",
            token.symbol, score.total, vol, holder_count
        );

        if let Some(notifier) = &self.notifier {
            notifier
                .send_buy_alert("Legacy", &token.address, velocity, holder_count as u32, score.total)
                .await;
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

        true
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
                s.market_ctx.clone(),
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
                    notifier
                        .send_generic_alert(format!("🔄 *MODE BERUBAH*: {:?} — {}", new_cfg.mode, new_cfg.reason))
                        .await;
                }
            }
            *cfg_write = new_cfg;
        }

        // 2. Kirim alert jika regime berubah (Insight metrik)
        if regime != self.last_regime {
            if let Some(notifier) = &self.notifier {
                let msg = format!(
                    "🌐 *MARKET REGIME BERUBAH*: {:?} -> *{:?}*


                     📊 *Metrik Saat Ini:*

                     • Birth Rate: {:.1}/m

                     • Global Velocity: {:.2}

                     • Momentum Fail: {:.1}%

                     • TP Rate (1h): {:.1}%


                     Filter bot otomatis menyesuaikan.",
                    self.last_regime,
                    regime,
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
        let window_passed_rate = if window_scanned > 0 {
            window_passed as f64 / window_scanned as f64
        } else {
            0.0
        };
        let win_rate_30 = db::query_win_rate_last_n(&self.db, 30).await;

        // FIX 2: Gunakan Global Velocity dari MarketContext, bukan last_velocities yang cacat
        let avg_velocity = current_ctx.global_velocity;

        // SAFEGUARD: Detect possible velocity bug (Perketat threshold: 10 scanned)
        if window_scanned > 10 && avg_velocity == 0.0 {
            warn!(
                "🚨 ALERT: Velocity data missing early in window (Scanned: {}, Vel: 0.0)",
                window_scanned
            );
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
            &mode,
        )
        .await;

        info!(
            "📊 Window Stats: Scanned={}, Passed={}, Mode={}, Regime={}, GlobalVel={:.2}",
            window_scanned, window_passed, mode, regime_str, avg_velocity
        );

        info!("📈 Total Session: Scanned={}, Passed={}", total_scanned, total_passed);
    }
}
