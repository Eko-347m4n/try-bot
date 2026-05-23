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
        let emoji = if trade.pnl_pct > 0.0 { "✅" } else { "🔻" };
        let msg = format!(
            "📦 *TRADE CLOSED*\n\n\
             Token: `{}`\n\
             Result: *{} {} {:.2}%*\n\
             Hold Time: *{}s*\n\n\
             📈 *Session ROI:* `{:.2}%` bersih",
            trade.token_addr,
            trade.exit_type, emoji, trade.pnl_pct,
            trade.hold_secs,
            trade.session_roi
        );
        self.bot.send_message(self.chat_id, msg)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await.ok();
    }

    pub async fn send_buy_alert(&self, token: &str, velocity: f64, buyers: u32, score: f64) {
        let msg = format!(
            "🚀 *VIRTUAL BUY SIGNAL*\n\n\
             Token: `{}`\n\
             Score: *{:.1}/10*\n\
             Velocity: `{:.2}` SOL/30s\n\
             Buyers: `{}`\n\n\
             [Pump.fun](https://pump.fun/{}) | [DexS](https://dexscreener.com/solana/{})",
            token, score, velocity, buyers, token, token
        );
        self.bot.send_message(self.chat_id, msg)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .disable_notification(true)
            .await.ok();
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
            let status_emoji = if s.is_running { "🟢 RUNNING" } else { "🔴 PAUSED" };
            let uptime = if let Some(start) = s.started_at {
                let elapsed = start.elapsed().as_secs();
                format!("{}m {}s", elapsed / 60, elapsed % 60)
            } else {
                "Unknown".to_string()
            };
            
            let response = format!(
                "<b>🤖 STATUS BOT</b>\n\n\
                 Status: <b>{}</b>\n\
                 Regime: <code>{:?}</code>\n\
                 Uptime: <code>{}</code>\n\n\
                 💰 <b>Portfolio (Net):</b>\n\
                 Balance: <code>{:.3} SOL</code>\n\
                 Bersih: <code>{:.2}% ROI</code>\n\n\
                 📊 <b>Performance:</b>\n\
                 Trades: <code>{}</code>\n\
                 Win Rate: <code>{:.1}%</code>\n\
                 Active: <code>{} pos</code>", 
                status_emoji, s.market_ctx.regime, uptime,
                s.virtual_balance, s.total_roi_pct(),
                s.total_trades, 
                if s.total_trades > 0 { (s.tp_hits as f64 / s.total_trades as f64) * 100.0 } else { 0.0 }, 
                s.active_positions
            );
            bot.send_message(msg.chat.id, response).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::Report => {
            let s = state.lock().await;
            let wr = if s.total_trades > 0 { (s.tp_hits as f64 / s.total_trades as f64) * 100.0 } else { 0.0 };
            let response = format!(
                "📊 <b>SUMMARY LAPORAN SESI</b>\n\n\
                 ROI Bersih: <b>{:.2}%</b>\n\
                 Win Rate: <b>{:.1}%</b>\n\n\
                 ✅ TP Hits: <code>{}</code>\n\
                 🔻 SL Hits: <code>{}</code>\n\
                 🔄 Total Trade: <code>{}</code>",
                s.total_roi_pct(), wr, s.tp_hits, s.sl_hits, s.total_trades
            );
            bot.send_message(msg.chat.id, response).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::Filter => {
            let s = state.lock().await;
            let response = format!(
                "🔍 <b>STATISTIK FILTER</b>\n\n\
                 Scanned: <code>{}</code>\n\
                 Passed: <b>{}</b>\n\n\
                 <b>Rejection Reasons:</b>\n\
                 - Volume: <code>{}</code>\n\
                 - Holders: <code>{}</code>\n\
                 - Momentum: <code>{}</code>\n\
                 - Velocity: <code>{}</code>\n\
                 - Score: <code>{}</code>",
                s.tokens_scanned, s.passed_filter,
                s.rejected_volume, s.rejected_holders,
                s.rejected_momentum, s.rejected_velocity,
                s.rejected_score
            );
            bot.send_message(msg.chat.id, response).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::Pause => {
            state.lock().await.is_running = false;
            bot.send_message(msg.chat.id, "⏸️ <b>Bot di-pause.</b> Sinyal baru tidak akan diproses.").parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::Resume => {
            state.lock().await.is_running = true;
            bot.send_message(msg.chat.id, "▶️ <b>Bot dilanjutkan.</b> Memulai pencarian sinyal baru...").parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::SetVolume(val) => {
            state.lock().await.volume_threshold = val;
            bot.send_message(msg.chat.id, format!("✅ Volume threshold diubah ke <b>{} SOL</b>", val)).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::SetVelocity(val) => {
            state.lock().await.velocity_threshold = val;
            bot.send_message(msg.chat.id, format!("✅ Velocity threshold diubah ke <b>{}</b>", val)).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
        Command::History => {
            let rows = sqlx::query("SELECT token_addr, exit_type, pnl_pct FROM trades ORDER BY id DESC LIMIT 10")
                .fetch_all(&db)
                .await;

            if let Ok(rows) = rows {
                let mut results = vec!["📜 <b>10 TRADE TERAKHIR:</b>\n".to_string()];
                for row in rows {
                    use sqlx::Row;
                    let addr: String = row.get("token_addr");
                    let exit_type: String = row.get("exit_type");
                    let pnl: f64 = row.get("pnl_pct");
                    let emoji = if pnl >= 0.0 { "✅" } else { "🔻" };
                    results.push(format!("{} <code>{}</code>: <b>{:.2}%</b> ({})", emoji, if addr.len() > 8 { &addr[..8] } else { &addr }, pnl, exit_type));
                }
                let msg_text = if results.len() == 1 { "Belum ada riwayat perdagangan.".to_string() } else { results.join("\n") };
                bot.send_message(msg.chat.id, msg_text).parse_mode(teloxide::types::ParseMode::Html).await?;
            } else {
                bot.send_message(msg.chat.id, "❌ Gagal mengambil riwayat dari database.").await?;
            }
        }
        Command::Winrate => {
            let wr = db::query_win_rate_last_n(&db, 20).await;
            bot.send_message(msg.chat.id, format!("🏆 <b>Rolling Winrate (20 trade terakhir):</b>\n\nDasar: <b>{:.1}%</b>", wr * 100.0)).parse_mode(teloxide::types::ParseMode::Html).await?;
        }
    }
    Ok(())
}
