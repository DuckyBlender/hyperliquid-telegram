use sqlx::SqlitePool;
use teloxide::{
    prelude::*,
    types::{Message, ParseMode},
    utils::command::BotCommands,
};

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Available commands:")]
pub enum Command {
    #[command(description = "Display this help message")]
    Help,
    #[command(description = "Start the bot")]
    Start,
    #[command(description = "Add a wallet to track: /add <wallet_address>")]
    Add(String),
    #[command(description = "Remove a tracked wallet: /remove <wallet_address>")]
    Remove(String),
    #[command(description = "List all tracked wallets")]
    List,
}

pub async fn run(bot: Bot, pool: SqlitePool) {
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
            let help_text = Command::descriptions().to_string();
            bot.send_message(
                msg.chat.id,
                format!("<b>📚 Help</b>\n\n<code>{}</code>", help_text),
            )
            .parse_mode(ParseMode::Html)
            .await?;
        }
        Command::Start => {
            let welcome = r#"<b>👋 Welcome to Hyperliquid Position Tracker!</b>

I'll notify you when wallets you're tracking open or close positions on Hyperliquid.

<b>Commands:</b>
• /add &lt;wallet&gt; - Add a wallet to track
• /remove &lt;wallet&gt; - Remove a tracked wallet
• /list - Show your tracked wallets
• /help - Show help message

<i>Start by adding a wallet address to track!</i>"#;

            bot.send_message(msg.chat.id, welcome)
                .parse_mode(ParseMode::Html)
                .await?;
        }
        Command::Add(wallet) => {
            let wallet = wallet.trim();
            if wallet.is_empty() {
                bot.send_message(
                    msg.chat.id,
                    "❌ Please provide a wallet address.\n\nUsage: <code>/add 0x...</code>",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            if !is_valid_address(wallet) {
                bot.send_message(
                    msg.chat.id,
                    "❌ Invalid wallet address format. Please provide a valid Ethereum address.",
                )
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            match crate::db::add_wallet(&pool, user_id, wallet).await {
                Ok(true) => {
                    log::info!("User {} added wallet {}", user_id, wallet);
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Now tracking wallet:\n<code>{}</code>",
                            wallet.to_lowercase()
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Ok(false) => {
                    bot.send_message(msg.chat.id, "⚠️ This wallet is already being tracked.")
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    log::error!("Failed to add wallet: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to add wallet. Please try again.")
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
                .parse_mode(ParseMode::Html)
                .await?;
                return Ok(());
            }

            match crate::db::remove_wallet(&pool, user_id, wallet).await {
                Ok(true) => {
                    log::info!("User {} removed wallet {}", user_id, wallet);
                    bot.send_message(
                        msg.chat.id,
                        format!(
                            "✅ Stopped tracking wallet:\n<code>{}</code>",
                            wallet.to_lowercase()
                        ),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
                Ok(false) => {
                    bot.send_message(msg.chat.id, "⚠️ This wallet was not being tracked.")
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
                Err(e) => {
                    log::error!("Failed to remove wallet: {}", e);
                    bot.send_message(msg.chat.id, "❌ Failed to remove wallet. Please try again.")
                        .parse_mode(ParseMode::Html)
                        .await?;
                }
            }
        }
        Command::List => match crate::db::get_user_wallets(&pool, user_id).await {
            Ok(wallets) => {
                if wallets.is_empty() {
                    bot.send_message(
                            msg.chat.id,
                            "📋 You're not tracking any wallets yet.\n\nUse <code>/add &lt;wallet&gt;</code> to start tracking.",
                        )
                        .parse_mode(ParseMode::Html)
                        .await?;
                } else {
                    let wallet_list: String = wallets
                        .iter()
                        .enumerate()
                        .map(|(i, w)| format!("{}. <code>{}</code>", i + 1, w.wallet_address))
                        .collect::<Vec<_>>()
                        .join("\n");

                    bot.send_message(
                        msg.chat.id,
                        format!("<b>📋 Your tracked wallets:</b>\n\n{}", wallet_list),
                    )
                    .parse_mode(ParseMode::Html)
                    .await?;
                }
            }
            Err(e) => {
                log::error!("Failed to list wallets: {}", e);
                bot.send_message(
                    msg.chat.id,
                    "❌ Failed to retrieve wallets. Please try again.",
                )
                .parse_mode(ParseMode::Html)
                .await?;
            }
        },
    }

    Ok(())
}

fn is_valid_address(address: &str) -> bool {
    address.starts_with("0x")
        && address.len() == 42
        && address[2..].chars().all(|c| c.is_ascii_hexdigit())
}
