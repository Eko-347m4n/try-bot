use crate::engine::rolling_stats::MarketSnapshot;
use crate::engine::market_context::{MarketContext, MarketRegime};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq)]
pub enum MarketMode {
    Hot,      // market sangat aktif
    Normal,   // kondisi market baik
    Strict,   // kondisi mulai buruk, perketat filter
    Pause,    // kondisi sangat buruk, stop trading
}

#[derive(Debug, Clone)]
pub struct DynamicConfig {
    pub mode:              MarketMode,
    pub velocity_thresh:   f64,
    pub max_velocity_thresh: f64, // Ceiling untuk menghindari extreme volatility
    pub volume_thresh:     f64,
    pub buyers_thresh:     u32,
    pub reason:            String,
}

impl DynamicConfig {
    pub fn normal() -> Self {
        Self {
            mode: MarketMode::Normal,
            velocity_thresh: 0.8, // naik dari 0.5
            max_velocity_thresh: 35.0, // Batas aman berdasarkan data historis
            volume_thresh: 3.0,
            buyers_thresh: 6,
            reason: "Mode Normal".into(),
        }
    }

    pub fn normal_relaxed() -> Self {
        Self {
            mode: MarketMode::Hot, // ubah dari Normal ke Hot
            velocity_thresh: 0.6, // naik dari 0.4
            max_velocity_thresh: 45.0, // Toleransi lebih tinggi di market Hot
            volume_thresh: 2.5,
            buyers_thresh: 5,
            reason: "Mode Relaxed (Hot Market)".into(),
        }
    }

    pub fn strict(reason: &str) -> Self {
        Self {
            mode: MarketMode::Strict,
            velocity_thresh: 1.2, // naik dari 1.0
            max_velocity_thresh: 30.0, // Perketat di market yang mulai tidak stabil
            volume_thresh: 5.0,
            buyers_thresh: 10,
            reason: format!("Mode Strict: {}", reason),
        }
    }

    pub fn pause(reason: &str) -> Self {
        Self {
            mode: MarketMode::Pause,
            velocity_thresh: 1.5,
            max_velocity_thresh: 20.0, // Sangat ketat saat pause
            volume_thresh: 5.0,
            buyers_thresh: 12,
            reason: format!("PAUSE: {}", reason),
        }
    }

    pub fn from_context(s: &MarketSnapshot, context: &MarketContext) -> Self {
        // 1. Cold market langsung pause
        if context.regime == MarketRegime::Cold {
            return Self::pause("Market regime: Cold");
        }

        // 2. Unknown -> Gunakan Mode Strict (Konservatif) saat data minim
        if context.regime == MarketRegime::Unknown {
            let mut cfg = Self::strict("Insufficient market data");
            cfg.reason = "Data minim (Using Strict mode for safety)".into();
            return cfg;
        }

        // 3. Kombinasi Regime + WR
        match (&context.regime, s.win_rate_30) {
            (MarketRegime::Hot, wr) if wr >= 0.38 => {
                let mut cfg = Self::normal_relaxed();
                cfg.reason = format!("Hot Market + WR {:.0}%", wr * 100.0);
                cfg
            },
            (MarketRegime::Hot, wr) => {
                let mut cfg = Self::normal();
                cfg.reason = format!("Hot Market but WR Low ({:.0}%)", wr * 100.0);
                cfg
            },
            (MarketRegime::Normal, wr) if wr >= 0.40 => {
                let mut cfg = Self::normal();
                cfg.reason = format!("Normal Market + WR {:.0}%", wr * 100.0);
                cfg
            },
            (MarketRegime::Normal, wr) => {
                let cfg = Self::strict(&format!("WR Low ({:.0}%)", wr * 100.0));
                cfg
            },
            (MarketRegime::Cooling, wr) if wr >= 0.38 => {
                let mut cfg = Self::strict("Market Cooling");
                cfg.reason = format!("Cooling but WR OK ({:.0}%)", wr * 100.0);
                cfg
            },
            (MarketRegime::Cooling, _) => {
                Self::pause("Cooling + Low WR")
            },
            _ => Self::pause("Regime tidak kondusif"),
        }
    }
}

pub type SharedConfig = Arc<RwLock<DynamicConfig>>;
