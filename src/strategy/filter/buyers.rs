use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;
use super::TokenFilter;

pub struct BuyersFilter {
    pub min_buyers: usize,
}

impl TokenFilter for BuyersFilter {
    fn name(&self) -> &'static str {
        "BuyersFilter"
    }

    fn evaluate(&self, _token: &TokenData, activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        if activity.unique_buyers < self.min_buyers {
            FilterResult {
                passed: false,
                reason: format!("Buyers {} < {}", activity.unique_buyers, self.min_buyers),
            }
        } else {
            FilterResult {
                passed: true,
                reason: "OK".to_string(),
            }
        }
    }
}
