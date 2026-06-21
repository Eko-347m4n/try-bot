use crate::core::types::ExitDecision;

pub mod fixed_tp_sl;
pub mod trailing_tp_sl;

// Position struct akan direfactor nanti, sementara kita asumsikan butuh harga entry dan current_price
pub trait ExitStrategy: Send + Sync {
    fn evaluate_exit(
        &self,
        entry_price: f64,
        current_price: f64,
        highest_price: f64,
        elapsed_secs: u64,
    ) -> Option<ExitDecision>;
    fn get_tp_sl(&self) -> (f64, f64);
}
