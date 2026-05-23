use serde::{Deserialize, Serialize};

pub mod settings;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenAgeRange {
    pub min: u64,
    pub max: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct VolumeThresholds {
    #[serde(rename = "30s")]
    pub v30s: f64,
    #[serde(rename = "60s")]
    pub v60s: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HolderGrowthThresholds {
    pub min_holder: u64,
    pub growth_per_30s: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LiquidityThresholds {
    pub min: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DistributionThresholds {
    pub max_top_holder: f64,
    pub max_top5: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RiskParameters {
    pub entry_size: f64,
    pub take_profit: f64,
    pub stop_loss: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StrategyParameters {
    pub token_age_seconds: TokenAgeRange,
    pub volume: VolumeThresholds,
    pub holder_growth: HolderGrowthThresholds,
    pub liquidity: LiquidityThresholds,
    pub distribution: DistributionThresholds,
    pub risk: RiskParameters,
    pub blackout_hours: Vec<u32>,        // Jam UTC untuk skip trading
    pub blackout_window_minutes: u32,  // Menit sebelum/sesudah blackout_hours
}

impl StrategyParameters {
    pub fn default() -> Self {
        Self {
            token_age_seconds: TokenAgeRange { min: 60, max: 120 },
            volume: VolumeThresholds { v30s: 3.0, v60s: 6.0 }, // Turunkan sedikit target volume dasar
            holder_growth: HolderGrowthThresholds { min_holder: 50, growth_per_30s: 20 },
            liquidity: LiquidityThresholds { min: 8.0 },
            distribution: DistributionThresholds { max_top_holder: 15.0, max_top5: 60.0 },
            risk: RiskParameters { entry_size: 0.1, take_profit: 15.0, stop_loss: 8.0 }, // TP +15%, SL -8%
            blackout_hours: vec![7, 12, 19],
            blackout_window_minutes: 15,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BotConfig {
    pub project_name: String,
    pub paper_trading: bool,
    pub websocket_url: String,
}

impl BotConfig {
    pub fn new() -> Self {
        Self {
            project_name: "pumpfun-quant-bot".to_string(),
            paper_trading: true,
            websocket_url: std::env::var("WEBSOCKET_URL")
                .unwrap_or_else(|_| "wss://pumpdev.io/ws".to_string()),
        }
    }
}

impl Default for BotConfig {
    fn default() -> Self {
        Self::new()
    }
}
