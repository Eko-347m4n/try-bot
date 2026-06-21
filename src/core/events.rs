#[derive(Debug, Clone)]
pub struct TokenActivity {
    pub total_volume: f64,
    pub buy_volume: f64,
    pub sell_volume: f64,
    pub half_volume: Option<f64>, // Snapshot pertengahan
    pub latest_price: f64,
    pub unique_buyers: usize, // Simpan sebagai count aja biar gampang dikirim
}

impl Default for TokenActivity {
    fn default() -> Self {
        Self {
            total_volume: 0.0,
            buy_volume: 0.0,
            sell_volume: 0.0,
            half_volume: None,
            latest_price: 0.0,
            unique_buyers: 0,
        }
    }
}
