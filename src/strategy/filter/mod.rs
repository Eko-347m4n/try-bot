use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;

pub mod volume;
pub mod liquidity;
pub mod blackout;
pub mod spike;
pub mod regime;
pub mod buyers;
pub mod score;
pub mod velocity;

pub trait TokenFilter: Send + Sync {
    fn name(&self) -> &'static str;
    
    // Konfigurasi dinamis bisa dilempar sebagai argumen tambahan jika perlu,
    // namun untuk simplifikasi, setiap Filter yang di-instantiate bisa membawa parameternya sendiri
    // di dalam field struct-nya (misal VolumeFilter { threshold: 10.0 }).
    fn evaluate(&self, token: &TokenData, activity: &TokenActivity, market_ctx: &MarketContext) -> FilterResult;
}
