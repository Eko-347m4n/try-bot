use super::TokenFilter;
use crate::core::events::TokenActivity;
use crate::core::types::FilterResult;
use crate::engine::market_context::{MarketContext, MarketRegime};
use crate::queue::event_queue::TokenData;

pub struct RegimeFilter {
    pub required_regimes: Vec<MarketRegime>,
}

impl TokenFilter for RegimeFilter {
    fn name(&self) -> &'static str {
        "RegimeFilter"
    }

    fn evaluate(&self, _token: &TokenData, _activity: &TokenActivity, market_ctx: &MarketContext) -> FilterResult {
        if !self.required_regimes.contains(&market_ctx.regime) {
            FilterResult { passed: false, reason: format!("Regime {:?} tidak diizinkan", market_ctx.regime) }
        } else {
            FilterResult { passed: true, reason: "OK".to_string() }
        }
    }
}
