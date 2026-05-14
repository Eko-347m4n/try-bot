use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use teloxide::RequestError;
use crate::state::SharedState;
use crate::storage::db;
use sqlx::SqlitePool;
use tracing::warn;

#[derive(Clone)]
pub struct TelegramNotifier {
    bot: Bot,
    chat_id: ChatId,
}

pub struct TradeResult {
    pub token_addr: String,
    pub pnl_pct: f64,
    pub hold_secs: i64,
    pub exit_type: String,
    pub session_roi: f64,
}

impl TelegramNotifier {
    pub fn new(bot_token: String, chat_id: String) -> Self {
        let bot = Bot::new(bot_token);
        let chat_id = ChatId(chat_id.parse::<i64>().expect("Chat ID harus berupa angka"));
        Self { bot, chat_id }
    }

    pub async fn send_trade_alert(&self, trade: &TradeResult) {
        let emoji = if trade.pnl_pct > 0.0 { "✅" } else { "❌" };
        let msg = format!(
            "Trade selesai\n\
             Token: `{}`\n\
             Exit: {} {} {:.2}%\n\
             Hold: {}s\n\
             Session ROI: {:.2}%",
            if trade.token_addr.len() > 8 { &trade.token_addr[..8] } else { &trade.token_addr },
            trade.exit_type, emoji, trade.pnl_pct.abs(),
            trade.hold_secs,
            trade.session_roi
        );
        self.bot.send_message(self.chat_id, msg).await.ok();
    }

    pub async fn send_buy_alert(&self, token: &str, velocity: f64, buyers: u32, score: f64) {
        let msg = format!(
            "BUY signal\n\
             Token: `{}`\n\
             Skor: *{:.1}*\n\
             Velocity: {:.2} SOL/30s\n\
             Buyers: {}",
            if token.len() > 8 { &token[..8] } else { token },
            score, velocity, buyers
        );
        self.bot.send_message(self.chat_id, msg).disable_notification(true).await.ok();
    }

    pub async fn send_generic_alert(&self, msg: String) {
        let mut attempts = 0;
        loop {
            attempts += 1;
            match self.bot.send_message(self.chat_id, &msg).await {
                Ok(_) => return,
                Err(e) if attempts < 3 => {
                    warn!("Telegram send gagal ({}): {} — retry", attempts, e);
                    tokio::time::sleep(tokio::time::Duration::from_secs(attempts * 2)).await;
                }
                Err(e) => {
                    tracing::error!("Telegram send gagal permanen: {}", e);
                    return;
                }
            }
        }
    }
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Commands:")]
enum Command {
    #[command(description = "live metrics")]
    Status,
    #[command(description = "ringkasan sesi lengkap")]
    Report,
    #[command(description = "stats filter")]
    Filter,
    #[command(description = "hentikan sinyal baru")]
    Pause,
    #[command(description = "aktifkan sinyal baru")]
    Resume,
    #[command(description = "ubah volume min")]
    SetVolume(f64),
    #[command(description = "ubah velocity")]
    SetVelocity(f64),
    #[command(description = "10 trade terakhir dari SQLite")]
    History,
    #[command(description = "win rate rolling 20 trade terakhir")]
    Winrate,
}

pub async fn start_command_handler(bot_token: String, state: SharedState, db: SqlitePool) {
    let bot = Bot::new(bot_token);
    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(answer)
        )
        .branch(
            Update::filter_message()
                .endpoint(|_bot: Bot, _msg: Message| async move {
                    Ok::<_, RequestError>(())
                })
        );
    
    tokio::spawn(async move {
        Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![state, db])
            .build()
            .dispatch()
            .await;
    });
}

async fn answer(bot: Bot, msg: Message, cmd: Command, state: SharedState, db: SqlitePool) -> ResponseResult<()> {
    match cmd {
        Command::Status => {
            let s = state.lock().await;
            let uptime = if let Some(start) = s.started_at {
                let elapsed = start.elapsed().as_secs();
                format!("{}m {}s", elapsed / 60, elapsed % 60)
            } else {
                "Unknown".to_string()
            };
            let regime = format!("{:?}", s.market_ctx.regime);
            bot.send_message(msg.chat.id, format!(
                "🤖 *STATUS BOT*\n\n\
                 Running: {}\n\
                 Regime: *{}*\n\
                 Uptime: {}\n\
                 Balance: *{:.3} SOL*\n\n\
                 Trades: {}\n\
                 Win Rate: {:.1}%\n\
                 ROI: {:.2}%\n\
                 Active Pos: {}", 
                s.is_running, regime, uptime, s.virtual_balance,
                s.total_trades, 
                if s.total_trades > 0 { (s.tp_hits as f64 / s.total_trades as f64) * 100.0 } else { 0.0 }, 
                s.total_roi_pct(), s.active_positions)).await?;
        }
        Command::Report => {
            let s = state.lock().await;
            bot.send_message(msg.chat.id, format!("ROI: {:.2}%\nTP: {}, SL: {}\nTotal Trades: {}", s.total_roi_pct(), s.tp_hits, s.sl_hits, s.total_trades)).await?;
        }
        Command::Filter => {
            let s = state.lock().await;
            bot.send_message(msg.chat.id, format!("Scanned: {}\nRejected Vol: {}\nRejected Hold: {}\nRejected Mom: {}\nRejected Vel: {}\nRejected Pres: {}\nRejected Score: {}\nPassed: {}", 
                s.tokens_scanned, s.rejected_volume, s.rejected_holders, s.rejected_momentum, s.rejected_velocity, s.rejected_pressure, s.rejected_score, s.passed_filter)).await?;
        }
        Command::Pause => {
            state.lock().await.is_running = false;
            bot.send_message(msg.chat.id, "Bot paused.").await?;
        }
        Command::Resume => {
            state.lock().await.is_running = true;
            bot.send_message(msg.chat.id, "Bot resumed.").await?;
        }
        Command::SetVolume(val) => {
            state.lock().await.volume_threshold = val;
            bot.send_message(msg.chat.id, format!("Volume threshold set to {}", val)).await?;
        }
        Command::SetVelocity(val) => {
            state.lock().await.velocity_threshold = val;
            bot.send_message(msg.chat.id, format!("Velocity threshold set to {}", val)).await?;
        }
        Command::History => {
            let rows = sqlx::query("SELECT token_addr, exit_type, pnl_pct FROM trades ORDER BY id DESC LIMIT 10")
                .fetch_all(&db)
                .await;

            if let Ok(rows) = rows {
                let mut results = Vec::new();
                for row in rows {
                    use sqlx::Row;
                    let addr: String = row.get("token_addr");
                    let exit_type: String = row.get("exit_type");
                    let pnl: f64 = row.get("pnl_pct");
                    results.push(format!("{}: {} ({:.2}%)", if addr.len() > 8 { &addr[..8] } else { &addr }, exit_type, pnl));
                }
                let msg_text = if results.is_empty() { "No trades yet.".to_string() } else { results.join("\n") };
                bot.send_message(msg.chat.id, msg_text).await?;
            } else {
                bot.send_message(msg.chat.id, "Failed to get history.").await?;
            }
        }
        Command::Winrate => {
            let wr = db::query_win_rate_last_n(&db, 20).await;
            bot.send_message(msg.chat.id, format!("Rolling Winrate (last 20): {:.1}%", wr * 100.0)).await?;
        }
    }
    Ok(())
}
