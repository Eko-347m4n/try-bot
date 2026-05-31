use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;
use super::TokenFilter;

pub struct LiquidityFilter {
    pub min_liquidity_sol: f64,
}

impl TokenFilter for LiquidityFilter {
    fn name(&self) -> &'static str {
        "LiquidityFilter"
    }

    fn evaluate(&self, token: &TokenData, _activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        if token.initial_liquidity < self.min_liquidity_sol {
            FilterResult {
                passed: false,
                reason: format!("Likuiditas awal {:.2} SOL < {:.2} SOL", token.initial_liquidity, self.min_liquidity_sol),
            }
        } else {
            FilterResult {
                passed: true,
                reason: "OK".to_string(),
            }
        }
    }
}
