use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;
use super::TokenFilter;

#[allow(dead_code)]
#[derive(Debug)]
pub struct ScoreBreakdown {
    pub volume_score:   f64,
    pub buyers_score:   f64,
    pub late_velocity_score: f64,
    pub momentum_score: f64,
    pub pressure_score: f64,
    pub early_acceleration_score: f64,
}

#[derive(Debug)]
pub struct TokenScore {
    pub total:    f64,
    pub breakdown: ScoreBreakdown,
}

impl TokenScore {
    pub fn calculate(
        total_volume: f64, 
        buyers: u32, 
        late_volume: f64,
        early_volume: f64,
        momentum_pct: f64, 
        buy_vol: f64, 
        sell_vol: f64,
        // config thresholds
        volume_thresh: f64,
        buyers_thresh: u32,
        velocity_thresh: f64,
    ) -> Self {
        let volume_ratio = if volume_thresh > 0.0 { total_volume / volume_thresh } else { 1.0 };
        let volume_score = match volume_ratio {
            r if r >= 3.0 => 15.0,
            r if r >= 2.0 => 12.0,
            r if r >= 1.5 => 9.0,
            r if r >= 1.0 => 5.0,
            _             => 0.0,
        };

        let buyers_ratio = if buyers_thresh > 0 { buyers as f64 / buyers_thresh as f64 } else { 1.0 };
        let buyers_score = match buyers_ratio {
            r if r >= 3.0 => 25.0,
            r if r >= 2.0 => 20.0,
            r if r >= 1.5 => 14.0,
            r if r >= 1.0 => 7.0,
            _             => 0.0,
        };

        let late_velocity_ratio = if velocity_thresh > 0.0 { late_volume / velocity_thresh } else { 1.0 };
        let late_velocity_score = match late_velocity_ratio {
            r if r >= 2.5 => 15.0,
            r if r >= 1.5 => 10.0,
            r if r >= 1.0 => 5.0,
            _             => 0.0,
        };

        let mut momentum_score = match momentum_pct {
            m if m >= 20.0 => 15.0,
            m if m >= 12.0 => 12.0,
            m if m >= 8.0  => 8.0,
            m if m >= 3.0  => 3.0,
            _              => 0.0,
        };

        let early_accel_ratio = if volume_thresh > 0.0 { early_volume / volume_thresh } else { 0.0 };
        let early_acceleration_score = match early_accel_ratio {
            r if r >= 2.5 => 30.0,
            r if r >= 1.5 => 20.0,
            r if r >= 0.7 => 10.0,
            _             => 0.0,
        };

        if momentum_score < 3.0 && early_acceleration_score >= 10.0 {
            momentum_score += early_acceleration_score * 0.5;
            if momentum_score > 15.0 { momentum_score = 15.0; }
        }

        let pressure_score = if sell_vol == 0.0 {
            10.0
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

        let total = volume_score + buyers_score + late_velocity_score
                  + momentum_score + pressure_score + early_acceleration_score;

        Self {
            total,
            breakdown: ScoreBreakdown {
                volume_score, buyers_score, late_velocity_score,
                momentum_score, pressure_score, early_acceleration_score,
            }
        }
    }
}

pub struct ScoreConfig {
    pub volume_thresh: f64,
    pub buyers_thresh: u32,
    pub velocity_thresh: f64,
    pub min_score: f64,
    pub enable_early_acceleration_bias: bool,
}

pub struct ScoreFilter {
    pub config: ScoreConfig,
}

impl TokenFilter for ScoreFilter {
    fn name(&self) -> &'static str {
        "ScoreFilter"
    }

    fn evaluate(&self, token: &TokenData, activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        let momentum_pct = if token.initial_price > 0.0 {
            ((activity.latest_price / token.initial_price) - 1.0) * 100.0
        } else {
            0.0
        };

        let early_volume = activity.half_volume.unwrap_or(0.0);
        let velocity = activity.total_volume - early_volume; // late_volume equivalent in the old code

        let score = TokenScore::calculate(
            activity.total_volume,
            activity.unique_buyers as u32,
            velocity,
            early_volume,
            momentum_pct,
            activity.buy_volume,
            activity.sell_volume,
            self.config.volume_thresh,
            self.config.buyers_thresh,
            self.config.velocity_thresh
        );

        let actual_min_score = self.config.min_score;

        // Validasi bias early acceleration (hanya untuk strategi 20260517 ke atas)
        if self.config.enable_early_acceleration_bias {
            if score.breakdown.momentum_score < 3.0 {
                let early_momentum_strong = score.breakdown.early_acceleration_score >= 20.0 && activity.unique_buyers > 5;
                if !early_momentum_strong {
                    return FilterResult {
                        passed: false,
                        reason: format!("Momentum Lemah (Score M: {:.0})", score.breakdown.momentum_score),
                    };
                }
            }
        }

        if score.total < actual_min_score {
            FilterResult {
                passed: false,
                reason: format!("Skor {:.1} < {:.1}", score.total, actual_min_score),
            }
        } else {
            FilterResult {
                passed: true,
                reason: format!("Skor {:.1} >= {:.1}", score.total, actual_min_score),
            }
        }
    }
}
