use std::collections::VecDeque;
use std::time::{Instant, Duration};

#[derive(Debug, Clone, PartialEq)]
pub enum MarketRegime {
    Hot,      // Sangat aktif, momentum kuat
    Normal,   // Kondisi standar
    Cooling,  // Mulai lesu
    Cold,     // Tidak kondusif, pause
    Unknown,  // Data belum cukup
}

#[derive(Debug, Clone)]
pub struct MarketContext {
    // Data mentah - rolling window
    token_births:      VecDeque<Instant>,
    all_velocities:    VecDeque<f64>,
    momentum_results:  VecDeque<bool>, // true=lolos, false=rejected

    // Metrik terhitung
    pub birth_rate_5m:      f64,
    pub global_velocity:    f64,
    pub momentum_fail_rate: f64,
    pub tp_rate_1h:         f64,
    pub regime:             MarketRegime,
}

impl Default for MarketContext {
    fn default() -> Self {
        Self {
            token_births: VecDeque::with_capacity(200),
            all_velocities: VecDeque::with_capacity(100),
            momentum_results: VecDeque::with_capacity(100),
            birth_rate_5m: 0.0,
            global_velocity: 0.0,
            momentum_fail_rate: 0.0,
            tp_rate_1h: 0.0,
            regime: MarketRegime::Unknown,
        }
    }
}

impl MarketContext {
    pub fn add_token_birth(&mut self) {
        self.token_births.push_back(Instant::now());
        self.cleanup_births();
    }

    pub fn add_velocity(&mut self, velocity: f64) {
        self.all_velocities.push_back(velocity);
        if self.all_velocities.len() > 100 {
            self.all_velocities.pop_front();
        }
    }

    pub fn add_momentum_result(&mut self, success: bool) {
        self.momentum_results.push_back(success);
        if self.momentum_results.len() > 100 {
            self.momentum_results.pop_front();
        }
    }

    fn cleanup_births(&mut self) {
        let now = Instant::now();
        let five_min = Duration::from_secs(300);
        while let Some(t) = self.token_births.front() {
            if now.duration_since(*t) > five_min {
                self.token_births.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn update_metrics(&mut self, tp_rate: f64) {
        self.cleanup_births();
        
        // Birth rate per menit (5m window)
        self.birth_rate_5m = self.token_births.len() as f64 / 5.0;

        // Global velocity average
        if !self.all_velocities.is_empty() {
            self.global_velocity = self.all_velocities.iter().sum::<f64>() / self.all_velocities.len() as f64;
        }

        // Momentum fail rate
        if !self.momentum_results.is_empty() {
            let fails = self.momentum_results.iter().filter(|&&r| !r).count();
            self.momentum_fail_rate = fails as f64 / self.momentum_results.len() as f64;
        }

        self.tp_rate_1h = tp_rate;
        self.update_regime();
    }

    fn update_regime(&mut self) {
        // FIX 3: Safeguard & Mechanism Reset
        // Butuh minimal data sebelum bisa menilai secara akurat. 
        // Jika data sangat sedikit, gunakan Unknown (yang akan men-trigger Normal_Relaxed di config)
        if self.token_births.len() < 10 || self.momentum_results.len() < 5 {
            self.regime = MarketRegime::Unknown;
            return;
        }

        let mut score: i32 = 0;
        let mut data_points = 0;

        // Birth rate scoring
        match self.birth_rate_5m {
            r if r > 40.0 => score += 2,
            r if r > 20.0 => score += 1,
            r if r > 5.0  => { score += 0; data_points += 1; },
            _             => score -= 2,
        }

        // Global velocity scoring
        match self.global_velocity {
            v if v > 1.2 => score += 2,
            v if v > 0.7 => score += 1,
            v if v > 0.3 => { score += 0; data_points += 1; },
            _            => score -= 2,
        }

        // Momentum fail rate scoring
        match self.momentum_fail_rate {
            r if r < 0.3 => score += 2,
            r if r < 0.5 => score += 1,
            r if r < 0.7 => { score -= 1; data_points += 1; },
            _            => score -= 3,
        }

        // TP Rate scoring (hanya jika ada data trade)
        if self.tp_rate_1h > 0.0 {
            match self.tp_rate_1h {
                r if r > 0.45 => score += 2,
                r if r > 0.35 => score += 1,
                r if r > 0.25 => score -= 1,
                _             => score -= 2,
            }
        }

        // Final Regime Decision dengan bantalan (padding)
        self.regime = match score {
            s if s >= 4  => MarketRegime::Hot,
            s if s >= 1  => MarketRegime::Normal,
            s if s >= -1 => MarketRegime::Cooling,
            _            => MarketRegime::Cold,
        };
    }
}
