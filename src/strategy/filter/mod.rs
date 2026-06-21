use crate::core::events::TokenActivity;
use crate::core::types::FilterResult;
use crate::engine::market_context::MarketContext;
use crate::queue::event_queue::TokenData;

pub mod blackout;
pub mod buyers;
pub mod liquidity;
pub mod regime;
pub mod score;
pub mod spike;
pub mod velocity;
pub mod volume;

pub trait TokenFilter: Send + Sync {
    fn name(&self) -> &'static str;

    // Konfigurasi dinamis bisa dilempar sebagai argumen tambahan jika perlu,
    // namun untuk simplifikasi, setiap Filter yang di-instantiate bisa membawa parameternya sendiri
    // di dalam field struct-nya (misal VolumeFilter { threshold: 10.0 }).
    fn evaluate(&self, token: &TokenData, activity: &TokenActivity, market_ctx: &MarketContext) -> FilterResult;
}
