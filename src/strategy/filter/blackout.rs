use chrono::{Timelike, Utc};
use std::cmp;
use crate::queue::event_queue::TokenData;
use crate::engine::market_context::MarketContext;
use crate::core::types::FilterResult;
use crate::core::events::TokenActivity;
use super::TokenFilter;

pub struct BlackoutFilter {
    pub blackout_hours: Vec<u32>,
    pub blackout_window_minutes: u32,
}

impl TokenFilter for BlackoutFilter {
    fn name(&self) -> &'static str {
        "BlackoutFilter"
    }

    fn evaluate(&self, _token: &TokenData, _activity: &TokenActivity, _market_ctx: &MarketContext) -> FilterResult {
        if self.blackout_hours.is_empty() {
            return FilterResult { passed: true, reason: "OK".to_string() };
        }

        let now = Utc::now();
        let current_hour = now.hour();
        let current_min = now.minute();
        let current_total_min = current_hour * 60 + current_min;

        for &dead_hour in &self.blackout_hours {
            let dead_total_min = dead_hour * 60;
            // Hitung selisih menit (circular 24h)
            let diff = if current_total_min > dead_total_min {
                current_total_min - dead_total_min
            } else {
                dead_total_min - current_total_min
            };
            
            // Handle cross-day
            let diff = cmp::min(diff, 1440 - diff);

            if diff <= self.blackout_window_minutes {
                return FilterResult {
                    passed: false,
                    reason: format!("Blackout Window (near {:02}:00 UTC)", dead_hour),
                };
            }
        }

        FilterResult { passed: true, reason: "OK".to_string() }
    }
}
