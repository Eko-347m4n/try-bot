use crate::config::StrategyParameters;
use crate::core::events::TokenActivity;
use crate::core::types::TraceRecord;
use crate::engine::market_context::MarketContext;
use crate::queue::event_queue::BotEvent;
use crate::strategy::instance::{Strategy, StrategyStatus};
use chrono::Utc;
use std::collections::HashMap;
use tokio::sync::mpsc;
use tokio::time::{sleep, Duration};
use tracing::info;

pub struct Dispatcher {
    strategies: Vec<Box<dyn Strategy>>,
    trace_tx: mpsc::Sender<TraceRecord>,
    event_tx: mpsc::UnboundedSender<BotEvent>,
    params: StrategyParameters,
    market_ctx: MarketContext,
    activities: HashMap<String, TokenActivity>,
}

impl Dispatcher {
    pub fn new(
        strategies: Vec<Box<dyn Strategy>>,
        trace_tx: mpsc::Sender<TraceRecord>,
        event_tx: mpsc::UnboundedSender<BotEvent>,
        params: StrategyParameters,
    ) -> Self {
        Self {
            strategies,
            trace_tx,
            event_tx,
            params,
            market_ctx: MarketContext::default(),
            activities: HashMap::new(),
        }
    }

    pub fn process_event(&mut self, event: BotEvent) {
        // Pre-processing: update activity map
        match &event {
            BotEvent::NewToken(token) => {
                self.activities.insert(
                    token.address.clone(),
                    TokenActivity { latest_price: token.initial_price, ..Default::default() },
                );
                self.handle_new_token(token.clone());
            }
            BotEvent::PriceUpdate { token_address, price, volume, is_buy, .. } => {
                if let Some(activity) = self.activities.get_mut(token_address) {
                    activity.latest_price = *price;
                    activity.total_volume += volume;
                    if *is_buy {
                        activity.buy_volume += volume;
                        activity.unique_buyers += 1; // Simplifikasi: setiap buy dihitung sbg unique buyer utk demo
                    } else {
                        activity.sell_volume += volume;
                    }
                }
            }
            BotEvent::TokenHalfMatured(token) => {
                if let Some(activity) = self.activities.get_mut(&token.address) {
                    activity.half_volume = Some(activity.total_volume);
                }
            }
            _ => {}
        }

        // Ambil activity context jika event berkaitan dengan token tertentu
        let token_addr = match &event {
            BotEvent::TokenMatured(t) => Some(t.address.clone()),
            BotEvent::PriceUpdate { token_address, .. } => Some(token_address.clone()),
            _ => None,
        };

        // Hindari mutably borrowing `self` dua kali dengan clone context dan meminjam values
        let activity_opt = token_addr.and_then(|addr| self.activities.get(&addr).cloned());

        for strategy in &mut self.strategies {
            strategy.process_event(&event, activity_opt.as_ref(), &self.market_ctx, &self.trace_tx);

            // GLOBAL STALE CHECK:
            // Jalankan pemeriksaan timeout untuk semua posisi di strategi ini
            strategy.check_timeouts(&self.activities, &self.trace_tx);
        }
    }

    pub fn get_strategy_statuses(&self) -> Vec<StrategyStatus> {
        self.strategies.iter().map(|s| s.get_status()).collect()
    }

    fn handle_new_token(&self, token: crate::queue::event_queue::TokenData) {
        let now = Utc::now();
        let age = (now - token.created_at).num_seconds() as u64;

        if age < self.params.token_age_seconds.min {
            let delay = self.params.token_age_seconds.min - age;
            let half_delay = delay / 2;
            info!(
                "🔍 Token {} ({}) terdeteksi. Menunggu aktivitas {}s...",
                token.symbol, token.address, delay
            );

            let tx_clone = self.event_tx.clone();
            tokio::spawn(async move {
                sleep(Duration::from_secs(half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenHalfMatured(token.clone()));
                sleep(Duration::from_secs(delay - half_delay)).await;
                let _ = tx_clone.send(BotEvent::TokenMatured(token));
            });
        } else {
            let _ = self.event_tx.send(BotEvent::TokenMatured(token));
        }
    }
}
