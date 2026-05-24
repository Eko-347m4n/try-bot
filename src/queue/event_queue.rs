use serde::{Deserialize, Serialize};
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenData {
    pub address: String,
    pub name: String,
    pub symbol: String,
    pub created_at: DateTime<Utc>,
    pub initial_price: f64,
    pub initial_liquidity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeData {
    pub token_address: String,
    pub price: f64,
    pub volume: f64,
    pub timestamp: DateTime<Utc>,
    pub is_buy: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum BotEvent {
    NewToken(TokenData),
    TokenHalfMatured(TokenData), // Ditambahkan untuk snapshot volume pertengahan
    TokenMatured(TokenData),
    Trade(TradeData),
    PriceUpdate {
        token_address: String,
        price: f64,
        volume: f64,
        sender: String, // Tambahan: Alamat wallet yang melakukan trade
        timestamp: DateTime<Utc>,
        is_buy: bool,
    },
    BuySignal {
        token_address: String,
        price: f64,
        volume_at_entry: f64,
        velocity_score: f64,
        buyers_count: u32,
        entry_score: f64,
    },
    SellSignal(String),
    Unsubscribe(String), // Tambahan: Untuk melepas langganan WebSocket
    Heartbeat, // Tambahan: Untuk menandakan koneksi WS masih hidup
    SessionEnd, // Event untuk menutup sesi dan menghitung ROI
}

#[allow(dead_code)]
pub struct EventQueue {
    pub id: Uuid,
}

impl EventQueue {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
        }
    }
}
