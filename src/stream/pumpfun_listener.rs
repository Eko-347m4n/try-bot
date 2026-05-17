use crate::queue::event_queue::{BotEvent, TokenData};
use crate::telegram::TelegramNotifier;
use futures_util::{StreamExt, SinkExt};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message};
use tracing::{info, error, warn, debug};
use url::Url;
use chrono::Utc;
use anyhow::Result;
use serde_json::Value;
use tokio::time::{sleep, Duration, timeout};

pub struct PumpfunListener {
    ws_url: String,
    tx: mpsc::UnboundedSender<BotEvent>,
    notifier: Option<TelegramNotifier>,
}

impl PumpfunListener {
    pub fn new(ws_url: String, tx: mpsc::UnboundedSender<BotEvent>, notifier: Option<TelegramNotifier>) -> Self {
        Self { ws_url, tx, notifier }
    }

    pub async fn start(&self, mut cmd_rx: mpsc::UnboundedReceiver<BotEvent>) -> Result<()> {
        let url = Url::parse(&self.ws_url)?;
        let mut delay = Duration::from_secs(1);

        loop {
            info!("🔄 Menghubungkan ke WebSocket Pump.fun...");
            
            match connect_async(url.clone()).await {
                Ok((ws_stream, _)) => {
                    info!("✅ Terhubung ke WebSocket.");
                    delay = Duration::from_secs(1); // Reset delay on success
                    
                    let (mut write, mut read) = ws_stream.split();
                    let (tx_ws, mut rx_ws) = mpsc::unbounded_channel::<String>();

                    let _ = write.send(Message::Text(serde_json::json!({"method":"subscribeNewToken"}).to_string())).await;

                    loop {
                        tokio::select! {
                            res = timeout(Duration::from_secs(60), read.next()) => {
                                match res {
                                    Ok(Some(Ok(Message::Text(text)))) => {
                                        self.process_message(text, &tx_ws).await;
                                    }
                                    Ok(Some(Ok(Message::Ping(payload)))) => {
                                        let _ = write.send(Message::Pong(payload)).await;
                                    }
                                    Ok(None) | Ok(Some(Ok(Message::Close(_)))) => {
                                        warn!("⚠️ Koneksi ditutup oleh server.");
                                        break;
                                    }
                                    Err(_) => {
                                        error!("⌛ WebSocket Timeout (No data for 60s). Reconnecting...");
                                        break;
                                    }
                                    _ => {}
                                }
                            }
                            Some(cmd_text) = rx_ws.recv() => {
                                if let Err(e) = write.send(Message::Text(cmd_text)).await {
                                    error!("❌ Gagal kirim perintah: {}", e);
                                    break;
                                }
                            }
                            Some(event) = cmd_rx.recv() => {
                                if let BotEvent::Unsubscribe(mint) = event {
                                    debug!("🔌 Melepas langganan token: {}", mint);
                                    let msg = serde_json::json!({"method":"unsubscribeTokenTrade","keys":[mint]}).to_string();
                                    let _ = tx_ws.send(msg);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("❌ Gagal terhubung: {}. Retry dalam {}s...", e, delay.as_secs());
                    if let Some(n) = &self.notifier {
                        n.send_generic_alert(format!("⚠️ *WebSocket Putus*: {} — reconnect dalam {}s", e, delay.as_secs())).await;
                    }
                }
            }
            sleep(delay).await;
            delay = (delay * 2).min(Duration::from_secs(60));
        }
    }

    fn parse_f64(val: Option<&Value>) -> Option<f64> {
        val.and_then(|v| {
            if v.is_f64() { v.as_f64() }
            else if v.is_string() { v.as_str()?.parse::<f64>().ok() }
            else { None }
        })
    }

    async fn process_message(&self, text: String, tx_ws: &mpsc::UnboundedSender<String>) {
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => return,
        };

        let mint = match v.get("mint").and_then(|m| m.as_str()) {
            Some(m) => m.to_string(),
            None => {
                if v.get("method").is_none() && v.get("error").is_none() {
                    debug!("[WS_DEBUG] Pesan tanpa mint: {}", text);
                }
                return;
            }
        };

        let v_sol = Self::parse_f64(v.get("vSolInBondingCurve"));
        let v_tokens = Self::parse_f64(v.get("vTokensInBondingCurve"));
        
        // Fleksibel: Coba solAmount (pumpportal) dan sol_amount (beberapa rpc lain)
        let sol_amount = Self::parse_f64(v.get("solAmount"))
            .or_else(|| Self::parse_f64(v.get("sol_amount")))
            .unwrap_or(0.0);

        let trader = v.get("traderPublicKey")
            .and_then(|t| t.as_str())
            .or_else(|| v.get("trader").and_then(|t| t.as_str()))
            .unwrap_or("Unknown").to_string();

        let tx_type = v.get("txType")
            .and_then(|t| t.as_str())
            .or_else(|| v.get("tx_type").and_then(|t| t.as_str()))
            .unwrap_or("")
            .to_lowercase();

        // Hitung harga jika data bonding curve tersedia
        let actual_price = if let (Some(sol), Some(tokens)) = (v_sol, v_tokens) {
            if tokens > 0.0 { sol / tokens } else { 0.0 }
        } else {
            0.0
        };

        if tx_type == "buy" || tx_type == "sell" {
            debug!("[TRADE] {} | {:.2} SOL | {}", mint, sol_amount, tx_type);
            
            let _ = self.tx.send(BotEvent::PriceUpdate {
                token_address: mint,
                price: actual_price,
                volume: sol_amount,
                sender: trader,
                timestamp: Utc::now(),
                is_buy: tx_type == "buy",
            });
        } else if tx_type == "create" {
            // Selalu subscribe transaksi untuk koin baru agar volume bisa dipantau
            let _ = tx_ws.send(serde_json::json!({"method":"subscribeTokenTrade","keys":[mint]}).to_string());

            let token_data = TokenData {
                address: mint.clone(),
                name: v.get("name").and_then(|n| n.as_str()).unwrap_or("?").to_string(),
                symbol: v.get("symbol").and_then(|s| s.as_str()).unwrap_or("?").to_string(),
                created_at: Utc::now(),
                initial_price: actual_price,
            };
            
            // PENTING: Kirim NewToken DULU baru PriceUpdate
            let _ = self.tx.send(BotEvent::NewToken(token_data));

            let _ = self.tx.send(BotEvent::PriceUpdate {
                token_address: mint,
                price: actual_price,
                volume: sol_amount,
                sender: trader,
                timestamp: Utc::now(),
                is_buy: true,
            });
        }
    }
}
