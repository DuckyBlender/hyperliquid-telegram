use log::{error, info};
use reqwest::Client;
use sqlx::SqlitePool;
use std::time::Duration;
use teloxide::{
    prelude::*,
    sugar::request::RequestReplyExt,
    types::{Message, ParseMode},
    utils::{command::BotCommands, html},
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

            // Validate note is not a reserved number (1-10)
            if let Some(n) = note {
                if is_reserved_note(n) {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Notes cannot be numbers 1-10 as these are reserved for wallet indexing.",
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }

                // Check if note already exists (case-insensitive) for another wallet
                match db::note_exists_for_user(&pool, user_id, n, Some(wallet)).await {
                    Ok(true) => {
                        bot.send_message(
                            msg.chat.id,
                            "❌ You already have a wallet with this note. Please use a different note.",
                        )
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                        return Ok(());
                    }
                    Err(e) => {
                        error!("Failed to check note existence: {}", e);
                        bot.send_message(msg.chat.id, "❌ Failed to add wallet. Please try again.")
                            .reply_to(msg.id)
                            .parse_mode(ParseMode::Html)
                            .await?;
                        return Ok(());
                    }
                    _ => {}
                }
            }

            // Check wallet limit (only for new wallets)
            let existing_count = db::get_user_wallet_count(&pool, user_id).await.unwrap_or(0);
            let wallet_lower = wallet.to_lowercase();
            let wallet_exists = db::get_user_wallets(&pool, user_id)
                .await
                .map(|wallets| wallets.iter().any(|w| w.wallet_address == wallet_lower))
                .unwrap_or(false);

            if !wallet_exists && existing_count >= db::MAX_WALLETS_PER_USER {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "❌ You've reached the maximum limit of {} tracked wallets.\n\nUse <code>/remove &lt;wallet&gt;</code> to remove a wallet first.",
                        db::MAX_WALLETS_PER_USER
                    ),
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            match db::add_wallet(&pool, user_id, wallet, note).await {
                Ok(db::AddWalletResult::Added) => {
                    info!("User {} added wallet {}", user_id, wallet);
                    let note_text = note
                        .map(|n| format!(" ({})", html::escape(n)))
                        .unwrap_or_default();
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
                Ok(db::AddWalletResult::Updated) => {
                    info!("User {} updated note for wallet {}", user_id, wallet);
                    let note_text = note
                        .map(|n| format!(" to '{}'", html::escape(n)))
                        .unwrap_or_else(|| " (removed)".to_string());
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Updated note{}:\n<code>{}</code>",
                            note_text,
                            wallet.to_lowercase()
                        ),
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Ok(db::AddWalletResult::AlreadyExistsNoChange) => {
                    bot.send_message(
                        msg.chat.id,
                        "⚠️ This wallet is already being tracked with the same note.",
                    )
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
        Command::Remove(identifier) => {
            let identifier = identifier.trim();
            if identifier.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address, index (1-10), or note.\n\nUsage: <code>/remove &lt;address|index|note&gt;</code>",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            // Resolve the identifier to a wallet address
            let resolved = match resolve_wallet_identifier(&pool, user_id, identifier).await {
                Ok(Some((addr, _))) => addr,
                Ok(None) => {
                    bot.send_message(
                        msg.chat.id,
                        "❌ Wallet not found. Use <code>/list</code> to see your tracked wallets.",
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }
                Err(e) => {
                    error!("Failed to resolve wallet identifier: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to remove wallet. Please try again.")
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                    return Ok(());
                }
            };

            match db::remove_wallet(&pool, user_id, &resolved).await {
                Ok(true) => {
                    info!("User {} removed wallet {}", user_id, resolved);
                    bot.send_message(
                        msg.chat.id,
                        format!("✅ Stopped tracking wallet:\n<code>{}</code>", resolved),
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
                            let display =
                                format_wallet_display(&w.wallet_address, w.note.as_deref(), true);
                            format!("{}. {}", i + 1, display)
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
        Command::Positions(identifier) => {
            let identifier = identifier.trim();
            if identifier.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address, index (1-10), or note.\n\nUsage: <code>/positions &lt;address|index|note&gt;</code>",
                )
                .reply_to(msg.id)
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            // Resolve the identifier to a wallet address
            let (wallet, note) = match resolve_wallet_identifier(&pool, user_id, identifier).await {
                Ok(Some((addr, note))) => (addr, note),
                Ok(None) => {
                    // If not found in user's wallets but looks like a valid address, use it directly
                    if is_valid_address(identifier) {
                        (identifier.to_lowercase(), None)
                    } else {
                        bot.send_message(
                            msg.chat.id,
                            "❌ Wallet not found. Provide a valid address, index (1-10), or note.",
                        )
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                        return Ok(());
                    }
                }
                Err(e) => {
                    error!("Failed to resolve wallet identifier: {}", e);
                    bot.send_message(
                        msg.chat.id,
                        "❌ Failed to fetch positions. Please try again.",
                    )
                    .reply_to(msg.id)
                    .parse_mode(ParseMode::Html)
                    .await?;
                    return Ok(());
                }
            };

            let client = Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client");

            let wallet_display = format_wallet_display(&wallet, note.as_deref(), false);

            let hyperdash_link = format!(
                "<a href=\"https://legacy.hyperdash.com/trader/{}\">Hyperdash</a>",
                wallet
            );

            match hyperliquid::fetch_user_state(&client, &wallet).await {
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
                                "<b>📊 Open Positions</b>\n\n\
                                 👛 Wallet: {}\n\n\
                                 <i>No open positions</i>\n\n\
                                 {}",
                                wallet_display, hyperdash_link
                            ),
                        )
                        .reply_to(msg.id)
                        .parse_mode(ParseMode::Html)
                        .await?;
                    } else {
                        let mut message = format!(
                            "<b>📊 Open Positions</b>\n\n\
                             👛 Wallet: {}\n",
                            wallet_display
                        );
                        for ap in positions {
                            let pos = &ap.position;
                            let size: f64 = pos.szi.parse().unwrap_or(0.0);
                            let is_long = size > 0.0;
                            let entry_price: f64 = pos
                                .entry_px
                                .as_ref()
                                .and_then(|p| p.parse().ok())
                                .unwrap_or(0.0);
                            let position_value: f64 = pos.position_value.parse().unwrap_or(0.0);
                            let current_price = if size.abs() > 0.0 {
                                position_value / size.abs()
                            } else {
                                0.0
                            };
                            let unrealized_pnl: f64 = pos.unrealized_pnl.parse().unwrap_or(0.0);
                            let leverage = pos.leverage.as_ref().map(|l| l.value).unwrap_or(1);
                            let direction_str = if is_long { "Long" } else { "Short" };
                            let direction_emoji = if is_long { "🟢" } else { "🔴" };
                            let pnl_str = if unrealized_pnl >= 0.0 {
                                format!("<b>+${:.2}</b>", unrealized_pnl)
                            } else {
                                format!("<b>-${:.2}</b>", unrealized_pnl.abs())
                            };
                            // Calculate PnL percentage (based on entry value)
                            let entry_value = entry_price * size.abs();
                            let pnl_pct = if entry_value > 0.0 {
                                (unrealized_pnl / entry_value) * 100.0
                            } else {
                                0.0
                            };
                            let pnl_pct_str = if pnl_pct >= 0.0 {
                                format!("+{:.2}%", pnl_pct)
                            } else {
                                format!("{:.2}%", pnl_pct)
                            };
                            // Round to avoid floating point artifacts like 2744.7999999999997
                            let current_price_rounded =
                                (current_price * 10000000000.0).round() / 10000000000.0; // 10 decimal places
                            let entry_str = format!("{}", entry_price)
                                .trim_end_matches('0')
                                .trim_end_matches('.')
                                .to_string();
                            let current_str = format!("{}", current_price_rounded)
                                .trim_end_matches('0')
                                .trim_end_matches('.')
                                .to_string();
                            let size_str = format!("{}", size.abs())
                                .trim_end_matches('0')
                                .trim_end_matches('.')
                                .to_string();

                            // Calculate price difference
                            let price_diff = current_price_rounded - entry_price;
                            let price_diff_str = if price_diff >= 0.0 {
                                format!("+${:.2}", price_diff)
                            } else {
                                format!("-${:.2}", price_diff.abs())
                            };

                            message.push_str(&format!(
                                "\n{} <b>{}x {} {}</b>\n\
                                 📊 Size: {} {} (${:.2})\n\
                                 💰 Entry: ${}\n\
                                 📍 Current: ${} ({})\n\
                                 💵 PnL: {} ({})\n",
                                direction_emoji,
                                leverage,
                                pos.coin,
                                direction_str,
                                size_str,
                                pos.coin,
                                position_value,
                                entry_str,
                                current_str,
                                price_diff_str,
                                pnl_str,
                                pnl_pct_str
                            ));
                        }
                        message.push_str(&format!("\n{}", hyperdash_link));
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

fn is_reserved_note(note: &str) -> bool {
    // Notes cannot be numbers 1-10
    if let Ok(n) = note.parse::<u32>() {
        (1..=10).contains(&n)
    } else {
        false
    }
}

/// Resolve a wallet identifier which can be:
/// - An index (1-10) referring to the user's wallet list
/// - A note name (case-insensitive)
/// - A wallet address
/// 
/// Returns (wallet_address, note) if found
async fn resolve_wallet_identifier(
    pool: &SqlitePool,
    user_id: i64,
    identifier: &str,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    // First, try parsing as index (1-10)
    if let Ok(index) = identifier.parse::<usize>()
        && (1..=10).contains(&index)
            && let Some(wallet) = db::get_wallet_by_index(pool, user_id, index).await? {
                return Ok(Some((wallet.wallet_address, wallet.note)));
            }

    // Second, try finding by note (case-insensitive)
    if let Some(wallet) = db::get_wallet_by_note(pool, user_id, identifier).await? {
        return Ok(Some((wallet.wallet_address, wallet.note)));
    }

    // Finally, if it looks like an address, return it as-is
    if is_valid_address(identifier) {
        let note = db::get_wallet_note(pool, identifier).await.ok().flatten();
        return Ok(Some((identifier.to_lowercase(), note)));
    }

    Ok(None)
}

pub fn format_wallet_display(wallet_address: &str, note: Option<&str>, full: bool) -> String {
    let addr = if full {
        wallet_address.to_string()
    } else {
        format!(
            "{}...{}",
            &wallet_address[..6],
            &wallet_address[wallet_address.len() - 4..]
        )
    };
    match note {
        Some(n) => format!("<code>{}</code> ({})", addr, html::escape(n)),
        None => format!("<code>{}</code>", addr),
    }
}
