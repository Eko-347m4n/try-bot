use crate::state::SharedState;
use crate::storage::db;
use sqlx::SqlitePool;
use teloxide::prelude::*;
use teloxide::utils::command::BotCommands;
use teloxide::RequestError;
use tracing::warn;

#[derive(Clone)]
pub struct TelegramNotifier {
    bot: Bot,
    chat_id: ChatId,
}

#[allow(dead_code)]
pub struct TradeResult {
    pub strategy_id: String,
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

    #[allow(dead_code)]
    pub async fn send_trade_alert(&self, trade: &TradeResult) {
        let emoji = if trade.pnl_pct > 0.0 { "✅" } else { "🔻" };
        let msg = format!(
            "📦 *TRADE CLOSED [{}]*\n\n\
             Token: `{}`\n\
             Result: *{} {} {:.2}%*\n\
             Hold Time: *{}s*\n\n\
             📈 *Strategy ROI:* `{:.2}%` bersih",
            trade.strategy_id,
            trade.token_addr,
            trade.exit_type,
            emoji,
            trade.pnl_pct,
            trade.hold_secs,
            trade.session_roi
        );
        self.bot
            .send_message(self.chat_id, msg)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .await
            .ok();
    }

    #[allow(dead_code)]
    pub async fn send_buy_alert(&self, strategy_id: &str, token: &str, velocity: f64, buyers: u32, score: f64) {
        let msg = format!(
            "🚀 *VIRTUAL BUY [{}]*\n\n\
             Token: `{}`\n\
             Score: *{:.1}/100*\n\
             Velocity: `{:.2}` SOL/30s\n\
             Buyers: `{}`\n\n\
             [Pump.fun](https://pump.fun/{}) | [DexS](https://dexscreener.com/solana/{})",
            strategy_id, token, score, velocity, buyers, token, token
        );
        self.bot
            .send_message(self.chat_id, msg)
            .parse_mode(teloxide::types::ParseMode::MarkdownV2)
            .disable_notification(true)
            .await
            .ok();
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
        .branch(Update::filter_message().filter_command::<Command>().endpoint(answer))
        .branch(Update::filter_message().endpoint(|_bot: Bot, _msg: Message| async move { Ok::<_, RequestError>(()) }));

    tokio::spawn(async move {
        teloxide::dispatching::Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![state, db])
            .error_handler(teloxide::error_handlers::LoggingErrorHandler::with_custom_text(
                "Teloxide error (mengabaikan TerminatedByOtherGetUpdates jika ada instance ganda yang berjalan)",
            ))
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

            let mut response = format!(
                "<b>🤖 STATUS BOT (MULTI-STRATEGY)</b>\n\n\
                 Status: <b>{}</b>\n\
                 Regime: <code>{:?}</code>\n\
                 Uptime: <code>{}</code>\n\n",
                status_emoji, s.market_ctx.regime, uptime
            );

            if s.strategy_statuses.is_empty() {
                response.push_str("<i>Belum ada strategi yang aktif.</i>");
            } else {
                for stat in &s.strategy_statuses {
                    let short_id = if stat.id.len() > 15 { &stat.id[..15] } else { &stat.id };
                    response.push_str(&format!(
                        "📌 <b>{}</b>\n\
                         • Equity: <b>{:.3} SOL</b>\n\
                         • Bal: <code>{:.2} SOL</code> | ROI: <code>{:.1}%</code>\n\
                         • WR: <code>{:.1}%</code> | Trades: <code>{}</code>\n\
                         • Pos: <code>{} active</code>\n\
                         • Exit: <code>+{:.0}% / -{:.0}%</code>\n\n",
                        short_id,
                        stat.total_equity,
                        stat.balance,
                        (stat.realized_pnl / 1.0) * 100.0,
                        stat.win_rate,
                        stat.trade_count,
                        stat.active_positions,
                        (stat.tp_multiplier - 1.0) * 100.0,
                        (1.0 - stat.sl_multiplier) * 100.0
                    ));
                }
            }

            bot.send_message(msg.chat.id, response)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::Report => {
            let s = state.lock().await;
            let mut response = "📊 <b>SUMMARY LAPORAN MULTI-STRATEGY</b>\n\n".to_string();

            if s.strategy_statuses.is_empty() {
                response.push_str("<i>Tidak ada data untuk dilaporkan.</i>");
            } else {
                for stat in &s.strategy_statuses {
                    response.push_str(&format!(
                        "📂 <b>{}</b>\n\
                         • Equity: <b>{:.3} SOL</b>\n\
                         • ROI: <b>{:.2}%</b>\n\
                         • Win Rate: <b>{:.1}%</b>\n\
                         • Total Trades: <code>{}</code>\n\
                         • Cash: <code>{:.3} SOL</code>\n\n",
                        stat.id,
                        stat.total_equity,
                        (stat.realized_pnl / 1.0) * 100.0,
                        stat.win_rate,
                        stat.trade_count,
                        stat.balance
                    ));
                }
            }

            bot.send_message(msg.chat.id, response)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::Filter => {
            use sqlx::Row;
            let s = state.lock().await;
            let active_ids: Vec<String> = s.strategy_statuses.iter().map(|stat| stat.id.clone()).collect();
            let session_start = s.session_start_utc.to_rfc3339();
            drop(s);

            let rows = sqlx::query(
                r#"SELECT strategy_id, 
                          COUNT(*) as total, 
                          SUM(CASE WHEN final_decision = 'BUY' THEN 1 ELSE 0 END) as buys 
                   FROM decision_traces 
                   WHERE timestamp >= ?
                   GROUP BY strategy_id"#,
            )
            .bind(session_start)
            .fetch_all(&db)
            .await;

            let mut response = "🔍 <b>STATISTIK FILTER PER STRATEGI</b>\n\n".to_string();

            if let Ok(rows) = rows {
                let mut has_data = false;
                for row in rows {
                    let id: String = row.get(0);
                    if !active_ids.contains(&id) {
                        continue;
                    }
                    has_data = true;
                    let total: i64 = row.get(1);
                    let buys: i64 = row.get(2);
                    let rejected = total - buys;
                    let pass_rate = if total > 0 {
                        (buys as f64 / total as f64) * 100.0
                    } else {
                        0.0
                    };

                    response.push_str(&format!(
                        "🛡️ <b>{}</b>\n\
                         • Scanned: <code>{}</code>\n\
                         • Passed: <b>{}</b> (<code>{:.1}%</code>)\n\
                         • Rejected: <code>{}</code>\n\n",
                        id, total, buys, pass_rate, rejected
                    ));
                }
                if !has_data {
                    response.push_str("<i>Belum ada data evaluasi token untuk strategi aktif.</i>");
                }
            } else {
                response.push_str("❌ Gagal mengambil data filter dari database.");
            }

            bot.send_message(msg.chat.id, response)
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::Pause => {
            state.lock().await.is_running = false;
            bot.send_message(msg.chat.id, "⏸️ <b>Bot di-pause.</b> Sinyal baru tidak akan diproses.")
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::Resume => {
            state.lock().await.is_running = true;
            bot.send_message(msg.chat.id, "▶️ <b>Bot dilanjutkan.</b> Memulai pencarian sinyal baru...")
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::SetVolume(val) => {
            state.lock().await.volume_threshold = val;
            bot.send_message(msg.chat.id, format!("✅ Volume threshold diubah ke <b>{} SOL</b>", val))
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
        }
        Command::SetVelocity(val) => {
            state.lock().await.velocity_threshold = val;
            bot.send_message(msg.chat.id, format!("✅ Velocity threshold diubah ke <b>{}</b>", val))
                .parse_mode(teloxide::types::ParseMode::Html)
                .await?;
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
                    results.push(format!(
                        "{} <code>{}</code>: <b>{:.2}%</b> ({})",
                        emoji,
                        if addr.len() > 8 { &addr[..8] } else { &addr },
                        pnl,
                        exit_type
                    ));
                }
                let msg_text = if results.len() == 1 {
                    "Belum ada riwayat perdagangan.".to_string()
                } else {
                    results.join("\n")
                };
                bot.send_message(msg.chat.id, msg_text)
                    .parse_mode(teloxide::types::ParseMode::Html)
                    .await?;
            } else {
                bot.send_message(msg.chat.id, "❌ Gagal mengambil riwayat dari database.")
                    .await?;
            }
        }
        Command::Winrate => {
            let wr = db::query_win_rate_last_n(&db, 20).await;
            bot.send_message(
                msg.chat.id,
                format!(
                    "🏆 <b>Rolling Winrate (20 trade terakhir):</b>\n\nDasar: <b>{:.1}%</b>",
                    wr * 100.0
                ),
            )
            .parse_mode(teloxide::types::ParseMode::Html)
            .await?;
        }
    }
    Ok(())
}
