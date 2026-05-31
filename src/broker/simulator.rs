use crate::core::types::ExitDecision;

pub trait Broker: Send + Sync {
    /// Menghitung total biaya masuk, termasuk ukuran posisi, slippage (jika ada), dan fee transaksi.
    /// Mengembalikan tuple (effective_entry_price, total_cost_sol)
    fn calculate_entry(&self, entry_price: f64, size_sol: f64) -> (f64, f64);

    /// Menghitung PNL bersih (Net Return) setelah posisi ditutup, dengan memperhitungkan fee dan slippage.
    /// Mengembalikan nilai net_return dalam SOL.
    fn calculate_net_return(&self, exit_type: &ExitDecision, entry_price: f64, current_price: f64, size_sol: f64) -> f64;
}

pub struct RealisticBroker {
    pub trading_fee_rate: f64,
    pub priority_fee_sol: f64,
    pub slippage_tp: f64,
    pub slippage_sl: f64,
    pub net_roi_enabled: bool,
}

impl Broker for RealisticBroker {
    fn calculate_entry(&self, entry_price: f64, size_sol: f64) -> (f64, f64) {
        if !self.net_roi_enabled {
            return (entry_price, size_sol);
        }

        let fee_buy = size_sol * self.trading_fee_rate;
        let total_buy_cost = size_sol + fee_buy + self.priority_fee_sol;
        
        // Asumsi slippage entry belum diterapkan secara historis di kode lama,
        // jadi effective entry price tetap
        (entry_price, total_buy_cost)
    }

    fn calculate_net_return(&self, exit_type: &ExitDecision, entry_price: f64, current_price: f64, size_sol: f64) -> f64 {
        if !self.net_roi_enabled {
            // Pengembalian kotor
            return size_sol * (current_price / entry_price);
        }

        let effective_exit_price = match exit_type {
            ExitDecision::TakeProfit => current_price * (1.0 - self.slippage_tp),
            ExitDecision::StopLoss => current_price * (1.0 - self.slippage_sl),
            _ => current_price * (1.0 - self.slippage_sl), // Asumsi slippage konservatif untuk exit lainnya
        };

        let gross_return = size_sol * (effective_exit_price / entry_price);
        let fee_sell = gross_return * self.trading_fee_rate;
        
        gross_return - fee_sell - self.priority_fee_sol
    }
}

