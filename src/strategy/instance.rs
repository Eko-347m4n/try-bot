use tracing::info;
use tokio::sync::mpsc;
use crate::queue::event_queue::{BotEvent, TokenData};
use crate::engine::market_context::MarketContext;
use crate::core::types::{TraceRecord, DecisionTrace, FilterExecution, ExitDecision, TradeTrace};
use crate::core::events::TokenActivity;
use crate::telegram::{TelegramNotifier, TradeResult};
use super::filter::TokenFilter;
use super::exit::ExitStrategy;
use crate::broker::simulator::Broker;
use chrono::Utc;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct StrategyStatus {
    pub id: String,
    pub balance: f64,
    pub total_equity: f64,
    pub realized_pnl: f64,
    pub trade_count: u32,
    pub win_rate: f64,
    pub active_positions: usize,
    pub tp_multiplier: f64,
    pub sl_multiplier: f64,
}

pub trait Strategy: Send + Sync {
    fn id(&self) -> &str;
    fn process_event(&mut self, event: &BotEvent, activity: Option<&TokenActivity>, ctx: &MarketContext, trace_tx: &mpsc::Sender<TraceRecord>);
    fn get_status(&self) -> StrategyStatus;
    fn check_timeouts(&mut self, activities: &std::collections::HashMap<String, TokenActivity>, trace_tx: &mpsc::Sender<TraceRecord>);
}

// Struct sederhana untuk mencatat Wallet Snapshot, ini bisa diekstrak ke wallet/mod.rs nanti
pub struct VirtualWallet {
    pub balance: f64,
    pub realized_pnl: f64,
    pub trade_count: u32,
    pub tp_hits: u32,
    pub sl_hits: u32,
}

impl Default for VirtualWallet {
    fn default() -> Self {
        Self { balance: 1.0, realized_pnl: 0.0, trade_count: 0, tp_hits: 0, sl_hits: 0 }
    }
}

impl VirtualWallet {
    pub fn win_rate(&self) -> f64 {
        let finished = self.tp_hits + self.sl_hits;
        if finished == 0 { 0.0 } else { (self.tp_hits as f64 / finished as f64) * 100.0 }
    }
}

pub struct StrategyInstance {
    pub strategy_id: String,
    pub filters: Vec<Box<dyn TokenFilter>>,
    pub broker: Box<dyn Broker>,
    pub exit: Box<dyn ExitStrategy>,
    pub wallet: VirtualWallet,
    pub notifier: Option<TelegramNotifier>,
    
    // Simplifikasi simulasi posisi: token_addr -> (entry_price, size_sol, entry_time, highest_price)
    pub open_positions: std::collections::HashMap<String, (f64, f64, u64, f64)>,
}

impl Strategy for StrategyInstance {
    fn id(&self) -> &str {
        &self.strategy_id
    }

    fn process_event(&mut self, event: &BotEvent, activity_opt: Option<&TokenActivity>, ctx: &MarketContext, trace_tx: &mpsc::Sender<TraceRecord>) {
        match event {
            BotEvent::TokenMatured(token) => {
                if let Some(activity) = activity_opt {
                    self.evaluate_buy_signal(token, activity, ctx, trace_tx);
                }
            }
            BotEvent::PriceUpdate { token_address, price, .. } => {
                self.evaluate_open_positions(token_address, *price, trace_tx);
            }
            _ => {}
        }
    }

    fn get_status(&self) -> StrategyStatus {
        let (tp, sl) = self.exit.get_tp_sl();
        let open_positions_value: f64 = self.open_positions.values().map(|&(_, size, _, _)| size).sum();
        let total_equity = self.wallet.balance + open_positions_value;

        StrategyStatus {
            id: self.strategy_id.clone(),
            balance: self.wallet.balance,
            total_equity,
            realized_pnl: self.wallet.realized_pnl,
            trade_count: self.wallet.trade_count,
            win_rate: self.wallet.win_rate(),
            active_positions: self.open_positions.len(),
            tp_multiplier: tp,
            sl_multiplier: sl,
        }
    }

    fn check_timeouts(&mut self, activities: &std::collections::HashMap<String, TokenActivity>, trace_tx: &mpsc::Sender<TraceRecord>) {
        let mut to_close = Vec::new();
        for (addr, (entry_price, _size_sol, entry_time, highest_price)) in self.open_positions.iter_mut() {
            let elapsed = (Utc::now().timestamp() as u64).saturating_sub(*entry_time);
            
            // Ambil harga terbaru dari activities map jika ada, jika tidak gunakan entry_price (asumsi harga tetap)
            let current_price = activities.get(addr).map(|a| a.latest_price).unwrap_or(*entry_price);
            
            if current_price > *highest_price {
                *highest_price = current_price;
            }

            if let Some(decision) = self.exit.evaluate_exit(*entry_price, current_price, *highest_price, elapsed) {
                to_close.push((addr.clone(), current_price, decision));
            }
        }

        for (addr, price, decision) in to_close {
            info!("[{}] 🔴 TIMEOUT/EXIT DETECTED for {}: {:?} | Price: {:.10}", self.id(), addr, decision, price);
            self.close_position(&addr, price, decision, trace_tx);
        }
    }
}

impl StrategyInstance {
    fn evaluate_buy_signal(&mut self, token: &TokenData, activity: &TokenActivity, ctx: &MarketContext, trace_tx: &mpsc::Sender<TraceRecord>) {
        if self.open_positions.contains_key(&token.address) { return; }
        
        let mut traces = Vec::new();
        let mut passed_all = true;

        for filter in &self.filters {
            let result = filter.evaluate(token, activity, ctx);
            traces.push(FilterExecution { 
                name: filter.name().to_string(), 
                result: if result.passed { "PASS".to_string() } else { "FAIL".to_string() },
                details: result.reason.clone() 
            });
            
            if !result.passed {
                passed_all = false;
                break;
            }
        }

        let trace_record = DecisionTrace {
            trace_id: Uuid::new_v4(),
            strategy_id: self.strategy_id.clone(),
            token_address: token.address.clone(),
            timestamp: Utc::now(),
            filters: traces,
            final_decision: if passed_all { "BUY".to_string() } else { "REJECT".to_string() },
        };
        
        let _ = trace_tx.try_send(TraceRecord::Decision(trace_record));

        if passed_all {
            let current_price = activity.latest_price;
            let early_volume = activity.half_volume.unwrap_or(0.0);
            let velocity = activity.total_volume - early_volume;
            let buyers = activity.unique_buyers as u32;

            if self.open_positions.len() >= 5 {
                info!("[{}] ⚠️ Buy di-skip: Max positions tercapai (5) untuk {}", self.id(), token.address);
                return;
            }

            if let Some(n) = &self.notifier {
                let n_clone = n.clone();
                let sid = self.strategy_id.clone();
                let taddr = token.address.clone();
                let score = 0.0; // TODO: get score from score filter if available
                tokio::spawn(async move {
                    n_clone.send_buy_alert(&sid, &taddr, velocity, buyers, score).await;
                });
            }

            self.execute_buy(token, current_price);
        }
    }

    fn execute_buy(&mut self, token: &TokenData, current_price: f64) {
        // Terapkan ukuran posisi statis 0.05 SOL untuk memperpanjang daya tahan (Risk of Ruin)
        let size_sol = 0.05;

        let (effective_entry, total_cost) = self.broker.calculate_entry(current_price, size_sol);
        
        // Hard stop: Jika saldo tidak cukup untuk biaya transaksi, batalkan eksekusi
        if self.wallet.balance < total_cost {
            tracing::warn!("[{}] Saldo tidak mencukupi untuk buy {}. Balance: {:.3}, Cost: {:.3}", self.id(), token.address, self.wallet.balance, total_cost);
            return;
        }

        self.wallet.balance -= total_cost;
        self.wallet.trade_count += 1;
        
        info!("[{}] 🟢 VIRTUAL BUY: {} | Balance: {:.3} SOL", self.id(), token.address, self.wallet.balance);
        self.open_positions.insert(token.address.clone(), (effective_entry, size_sol, Utc::now().timestamp() as u64, current_price));
    }

    fn evaluate_open_positions(&mut self, token_address: &str, current_price: f64, trace_tx: &mpsc::Sender<TraceRecord>) {
        let exit_decision = if let Some((entry_price, _, entry_time, highest_price)) = self.open_positions.get_mut(token_address) {
            if current_price > *highest_price {
                *highest_price = current_price;
            }
            let elapsed = (Utc::now().timestamp() as u64).saturating_sub(*entry_time);
            let dec = self.exit.evaluate_exit(*entry_price, current_price, *highest_price, elapsed);
            if dec.is_some() {
                info!("[{}] 🔴 EXIT DETECTED for {}: {:?} | Price: {:.10} | Elapsed: {}s", self.id(), token_address, dec, current_price, elapsed);
            }
            dec
        } else {
            None
        };

        if let Some(decision) = exit_decision {
            self.close_position(token_address, current_price, decision, trace_tx);
        }
    }

    fn close_position(&mut self, token_address: &str, current_price: f64, decision: ExitDecision, trace_tx: &mpsc::Sender<TraceRecord>) {
        if let Some((entry_price, size_sol, entry_time, _highest)) = self.open_positions.remove(token_address) {
            let net_return = self.broker.calculate_net_return(&decision, entry_price, current_price, size_sol);
            self.wallet.balance += net_return;
            
            let (cost, _) = self.broker.calculate_entry(entry_price, size_sol);
            let pnl = net_return - cost;
            let pnl_pct = (pnl / cost) * 100.0;
            self.wallet.realized_pnl += pnl;

            let hold_secs = (Utc::now().timestamp() as u64).saturating_sub(entry_time) as i64;

            match decision {
                ExitDecision::TakeProfit => self.wallet.tp_hits += 1,
                ExitDecision::StopLoss => self.wallet.sl_hits += 1,
                _ => {}
            }

            let trade_record = TradeTrace {
                strategy_id: self.strategy_id.clone(),
                token_addr: token_address.to_string(),
                entry_price,
                exit_price: current_price,
                pnl_pct,
                exit_type: format!("{:?}", decision),
                hold_secs,
            };
            let _ = trace_tx.try_send(TraceRecord::Trade(trade_record));

            if let Some(n) = &self.notifier {
                let n_clone = n.clone();
                let trade_res = TradeResult {
                    strategy_id: self.strategy_id.clone(),
                    token_addr: token_address.to_string(),
                    pnl_pct,
                    hold_secs,
                    exit_type: format!("{:?}", decision),
                    session_roi: (self.wallet.realized_pnl / 1.0) * 100.0, // initial 1.0
                };
                tokio::spawn(async move {
                    n_clone.send_trade_alert(&trade_res).await;
                });
            }
        }
    }
}
