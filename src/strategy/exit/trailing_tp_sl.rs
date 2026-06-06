use crate::core::types::ExitDecision;
use super::ExitStrategy;

pub struct TrailingTpSlExit {
    pub activation_multiplier: f64, // Titik aktif trailing (misal 1.15 untuk +15%)
    pub trailing_percent: f64,      // Penurunan dari high untuk exit (misal 0.025 untuk 2.5%)
    pub sl_multiplier: f64,         // Stop loss multiplier (misal 0.92 untuk -8%)
    pub timeout_secs: u64,          // Waktu maksimal hold
}

impl ExitStrategy for TrailingTpSlExit {
    fn evaluate_exit(&self, entry_price: f64, current_price: f64, highest_price: f64, elapsed_secs: u64) -> Option<ExitDecision> {
        let sl_target = entry_price * self.sl_multiplier;
        let activation_target = entry_price * self.activation_multiplier;

        // Jika harga sudah pernah menyentuh target aktivasi, gunakan trailing stop
        if highest_price >= activation_target {
            let trailing_stop_price = highest_price * (1.0 - self.trailing_percent);
            
            if current_price <= trailing_stop_price {
                Some(ExitDecision::TakeProfit)
            } else if elapsed_secs > self.timeout_secs {
                Some(ExitDecision::TimeoutStale)
            } else {
                None
            }
        } else {
            // Sebelum aktivasi, gunakan SL standar dan timeout
            if current_price <= sl_target {
                Some(ExitDecision::StopLoss)
            } else if elapsed_secs > self.timeout_secs {
                Some(ExitDecision::TimeoutStale)
            } else {
                None
            }
        }
    }

    fn get_tp_sl(&self) -> (f64, f64) {
        (self.activation_multiplier, self.sl_multiplier)
    }
}
