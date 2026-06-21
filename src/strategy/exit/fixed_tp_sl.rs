use super::ExitStrategy;
use crate::core::types::ExitDecision;

pub struct FixedTpSlExit {
    pub tp_multiplier: f64, // e.g., 1.30 for +30%
    pub sl_multiplier: f64, // e.g., 0.92 for -8%
    pub timeout_secs: u64,  // e.g., 120s for stale timeout
}

impl ExitStrategy for FixedTpSlExit {
    fn evaluate_exit(
        &self,
        entry_price: f64,
        current_price: f64,
        _highest_price: f64,
        elapsed_secs: u64,
    ) -> Option<ExitDecision> {
        let tp_target = entry_price * self.tp_multiplier;
        let sl_target = entry_price * self.sl_multiplier;

        if current_price >= tp_target {
            Some(ExitDecision::TakeProfit)
        } else if current_price <= sl_target {
            Some(ExitDecision::StopLoss)
        } else if elapsed_secs > self.timeout_secs {
            Some(ExitDecision::TimeoutStale)
        } else {
            None
        }
    }

    fn get_tp_sl(&self) -> (f64, f64) {
        (self.tp_multiplier, self.sl_multiplier)
    }
}
