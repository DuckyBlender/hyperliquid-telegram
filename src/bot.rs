use log::{error, info};
use reqwest::Client;
use sqlx::SqlitePool;
use std::time::Duration;
use teloxide::{
    prelude::*, sugar::request::RequestReplyExt, types::{Message, ParseMode}, utils::command::BotCommands
};

use crate::db;
use crate::hyperliquid;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase")]
pub enum Command {
    #[command(description = "Display this help message")]
    Help,
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Add a wallet to track")]
    Add(String),
    #[command(description = "Remove a tracked wallet")]
    Remove(String),
    #[command(description = "List all tracked wallets")]
    List,
    #[command(description = "Show open positions for a wallet")]
    Positions(String),
}

pub async fn run(bot: Bot, pool: SqlitePool) {
    // Register commands with Telegram
    if let Err(e) = bot.set_my_commands(Command::bot_commands()).await {
        error!("Failed to register commands: {}", e);
    } else {
        info!("Bot commands registered successfully");
    }

    Command::repl(bot, move |bot: Bot, msg: Message, cmd: Command| {
        let pool = pool.clone();
        async move {
            // Only respond to private messages (DMs)
            if !msg.chat.is_private() {
                return Ok(());
            }

            handle_command(bot, msg, cmd, pool).await
        }
    })
    .await;
}

async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    pool: SqlitePool,
) -> ResponseResult<()> {
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);

    match cmd {
        Command::Help => {
            bot.send_message(
                msg.chat.id,
                format!("<b>📚 Help</b>\n{}", Command::descriptions()),
            )
            .reply_to(msg.id)
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Command::Start => {
            let welcome = format!(
                "<b>👋 Welcome to Hyperliquid Position Tracker!</b>\n\n\
                 I'll notify you when wallets you're tracking open or close positions on Hyperliquid.\n\
                 {}\n\n\
                 <i>Start by adding a wallet address to track!</i>",
                Command::descriptions()
            );

            bot.send_message(msg.chat.id, welcome)
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Command::Add(args) => {
            let args = args.trim();
            if args.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address.\n\nUsage: <code>/add 0x... [note]</code>",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            // Parse wallet and optional note
            let parts: Vec<&str> = args.splitn(2, ' ').collect();
            let wallet = parts[0];
            let note = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());

            if !is_valid_address(wallet) {
                bot.send_message(
                    msg.chat.id,
                    "❌ Invalid wallet address format. Please provide a valid Ethereum address.",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            match db::add_wallet(&pool, user_id, wallet, note).await {
                Ok(true) => {
                    info!("User {} added wallet {}", user_id, wallet);
                    let note_text = note.map(|n| format!(" ({})", n)).unwrap_or_default();
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Now tracking wallet{}:\n<code>{}</code>",
                            note_text,
                            wallet.to_lowercase()
                        ),
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Ok(false) => {
                    bot.send_message(msg.chat.id, "⚠️ This wallet is already being tracked.")
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    error!("Failed to add wallet: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to add wallet. Please try again.")
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        Command::Remove(wallet) => {
            let wallet = wallet.trim();
            if wallet.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address.\n\nUsage: <code>/remove 0x...</code>",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            match db::remove_wallet(&pool, user_id, wallet).await {
                Ok(true) => {
                    info!("User {} removed wallet {}", user_id, wallet);
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Stopped tracking wallet:\n<code>{}</code>",
                            wallet.to_lowercase()
                        ),
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Ok(false) => {
                    bot.send_message(msg.chat.id, "⚠️ This wallet was not being tracked.")
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    error!("Failed to remove wallet: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to remove wallet. Please try again.")
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        Command::List => match db::get_user_wallets(&pool, user_id).await {
            Ok(wallets) => {
                if wallets.is_empty() {
                    bot.send_message(
                            msg.chat.id,
                            "📋 You're not tracking any wallets yet.\n\nUse <code>/add &lt;wallet&gt; [note]</code> to start tracking.",
                        )
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                } else {
                    let wallet_list: String = wallets
                        .iter()
                        .enumerate()
                        .map(|(i, w)| {
                            let note_text = w.note.as_ref().map(|n| format!(" - {}", n)).unwrap_or_default();
                            format!("{}. <code>{}</code>{}", i + 1, w.wallet_address, note_text)
                        })
                        .collect::<Vec<_>>()
                        .join("\n");

                    bot.send_message(
                        msg.chat.id,
                        format!("<b>📋 Your tracked wallets:</b>\n\n{}", wallet_list),
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
            }
            Err(e) => {
                error!("Failed to list wallets: {}", e);
                bot.send_message(
                    msg.chat.id,
                    "❌ Failed to retrieve wallets. Please try again.",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
            }
        },
        Command::Positions(wallet) => {
            let wallet = wallet.trim();
            if wallet.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address.\n\nUsage: <code>/positions 0x...</code>",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if !is_valid_address(wallet) {
                bot.send_message(
                    msg.chat.id,
                    "❌ Invalid wallet address format. Please provide a valid Ethereum address.",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            let client = Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client");

            let wallet_display = format!(
                "{}...{}",
                &wallet[..6],
                &wallet[wallet.len() - 4..]
            );

            let hyperdash_link = format!(
                "<a href=\"https://legacy.hyperdash.com/trader/{}\">Hyperdash</a>",
                wallet
            );

            match hyperliquid::fetch_user_state(&client, wallet).await {
                Ok(user_state) => {
                    let positions: Vec<_> = user_state
                        .asset_positions
                        .iter()
                        .filter(|ap| ap.position.szi.parse::<f64>().unwrap_or(0.0) != 0.0)
                        .collect();

                    if positions.is_empty() {
                        bot.send_message(
                            msg.chat.id,
                            format!(
                                "<b>📊 Open Positions</b>\n\n<code>{}</code>\n\n<i>No open positions</i>\n{}",
                                wallet_display,
                                hyperdash_link
                            ),
                        )
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                    } else {
                        let mut message = format!("<b>📊 Open Positions</b>\n\n<code>{}</code>\n", wallet_display);
                        for ap in positions {
                            let pos = &ap.position;
                            let size: f64 = pos.szi.parse().unwrap_or(0.0);
                            let is_long = size > 0.0;
                            let entry_price: f64 = pos.entry_px.as_ref().and_then(|p| p.parse().ok()).unwrap_or(0.0);
                            let unrealized_pnl: f64 = pos.unrealized_pnl.parse().unwrap_or(0.0);
                            let leverage = pos.leverage.as_ref().map(|l| l.value).unwrap_or(1);
                            let direction = if is_long { "🟢" } else { "🔴" };
                            let direction_str = if is_long { "Long" } else { "Short" };
                            let pnl_str = if unrealized_pnl >= 0.0 {
                                format!("+${:.2}", unrealized_pnl)
                            } else {
                                format!("-${:.2}", unrealized_pnl.abs())
                            };
                            let entry_str = format!("{}", entry_price).trim_end_matches('0').trim_end_matches('.').to_string();
                            let size_str = format!("{}", size.abs()).trim_end_matches('0').trim_end_matches('.').to_string();

                            message.push_str(&format!(
                                "\n{} <b>{}x {} {}</b>\n   📊 {} {}\n   💰 ${}\n   📈 {}\n",
                                direction,
                                leverage,
                                pos.coin,
                                direction_str,
                                size_str,
                                pos.coin,
                                entry_str,
                                pnl_str
                            ));
                        }
                        message.push_str(&hyperdash_link);
                        bot.send_message(msg.chat.id, message)
                            .reply_to(msg.id)
                            .parse_mode(ParseMode::Html)
                            .await?;
                    }
                }
                Err(e) => {
                    error!("Failed to fetch positions for {}: {}", wallet, e);
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to fetch positions. Please try again.",
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
            }
        }
    }

    Ok(())
}

fn is_valid_address(address: &str) -> bool {
    address.starts_with("0x")
        && address.len() == 42
        && address[2..].chars().all(|c| c.is_ascii_hexdigit())
}
