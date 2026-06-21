use super::TokenFilter;
use crate::core::events::TokenActivity;
use crate::core::types::FilterResult;
use crate::engine::market_context::MarketContext;
use crate::queue::event_queue::TokenData;

pub struct VolumeFilter {
    pub min_volume_sol: f64,
}

impl TokenFilter for VolumeFilter {
    fn name(&self) -> &'static str {
        "VolumeFilter"
    }

    fn evaluate(&self, _token: &TokenData, activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        if activity.total_volume < self.min_volume_sol {
            FilterResult {
                passed: false,
                reason: format!("Vol {:.2} < {:.2} SOL", activity.total_volume, self.min_volume_sol),
            }
        } else {
            FilterResult { passed: true, reason: "OK".to_string() }
        }
    }
}
