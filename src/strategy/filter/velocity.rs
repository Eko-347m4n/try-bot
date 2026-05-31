use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;
use super::TokenFilter;

pub struct VelocityFilter {
    pub min_velocity_sol: f64,
}

impl TokenFilter for VelocityFilter {
    fn name(&self) -> &'static str {
        "VelocityFilter"
    }

    fn evaluate(&self, _token: &TokenData, activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        let early_volume = activity.half_volume.unwrap_or(0.0);
        let velocity = activity.total_volume - early_volume;

        if velocity < self.min_velocity_sol {
            FilterResult {
                passed: false,
                reason: format!("Velocity {:.2} < {:.2}", velocity, self.min_velocity_sol),
            }
        } else {
            FilterResult {
                passed: true,
                reason: "OK".to_string(),
            }
        }
    }
}
