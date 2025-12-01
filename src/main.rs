mod bot;
mod db;
mod hyperliquid;
mod logging;

use log::info;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio::sync::RwLock;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    logging::setup_logging()?;

    info!("Starting Hyperliquid Telegram Bot...");

    let database_url =
        std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:bot.db".to_string());
    let pool = db::init_db(&database_url).await?;

    let bot = Bot::from_env();

    let state = Arc::new(RwLock::new(hyperliquid::PositionTracker::new()));

    // Spawn position monitoring task
    let monitor_pool = pool.clone();
    let monitor_bot = bot.clone();
    let monitor_state = state.clone();
    tokio::spawn(async move {
        hyperliquid::monitor_positions(monitor_pool, monitor_bot, monitor_state).await;
    });

    // Start the bot
    bot::run(bot, pool).await;

    Ok(())
}
