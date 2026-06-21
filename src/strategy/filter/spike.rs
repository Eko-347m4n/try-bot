use super::TokenFilter;
use crate::core::events::TokenActivity;
use crate::core::types::FilterResult;
use crate::engine::market_context::MarketContext;
use crate::queue::event_queue::TokenData;

pub struct SpikeFilter {
    pub max_multiplier: f64,
}

impl TokenFilter for SpikeFilter {
    fn name(&self) -> &'static str {
        "SpikeFilter"
    }

    fn evaluate(&self, _token: &TokenData, activity: &TokenActivity, market_ctx: &MarketContext) -> FilterResult {
        let early_volume = activity.half_volume.unwrap_or(0.0);
        let velocity = activity.total_volume - early_volume;
        let global_velocity = market_ctx.global_velocity;

        if global_velocity > 0.5 && velocity > self.max_multiplier * global_velocity {
            FilterResult {
                passed: false,
                reason: format!(
                    "Spike: {:.2} > {:.1}x Global ({:.2})",
                    velocity, self.max_multiplier, global_velocity
                ),
            }
        } else {
            FilterResult { passed: true, reason: "OK".to_string() }
        }
    }
}
