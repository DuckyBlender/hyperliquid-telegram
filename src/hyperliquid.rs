use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::{prelude::*, types::ParseMode};
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};

const HYPERLIQUID_API: &str = "https://api.hyperliquid.xyz/info";
const POLL_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub coin: String,
    pub szi: String,
    #[serde(rename = "entryPx")]
    pub entry_px: Option<String>,
    #[serde(rename = "positionValue")]
    pub position_value: String,
    #[serde(rename = "unrealizedPnl")]
    pub unrealized_pnl: String,
    pub leverage: Option<Leverage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leverage {
    #[serde(rename = "type")]
    pub leverage_type: String,
    pub value: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPosition {
    pub position: Position,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserState {
    pub asset_positions: Vec<AssetPosition>,
}

#[derive(Debug, Clone)]
pub struct PositionTracker {
    // wallet_address -> (coin -> position_size)
    pub positions: HashMap<String, HashMap<String, String>>,
}

impl PositionTracker {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
        }
    }
}

pub async fn monitor_positions(pool: SqlitePool, bot: Bot, state: Arc<RwLock<PositionTracker>>) {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    let mut interval = interval(Duration::from_secs(POLL_INTERVAL_SECS));

    log::info!("Position monitoring started");

    loop {
        interval.tick().await;

        let wallets = match crate::db::get_all_tracked_wallets(&pool).await {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to fetch tracked wallets: {}", e);
                continue;
            }
        };

        // Group wallets by user_id for notification batching
        let mut wallet_users: HashMap<String, Vec<i64>> = HashMap::new();
        for wallet in &wallets {
            wallet_users
                .entry(wallet.wallet_address.clone())
                .or_default()
                .push(wallet.user_id);
        }

        for (wallet_address, user_ids) in wallet_users {
            match fetch_positions(&client, &wallet_address).await {
                Ok(positions) => {
                    let changes =
                        detect_position_changes(&state, &wallet_address, &positions).await;

                    for change in changes {
                        for &user_id in &user_ids {
                            if let Err(e) =
                                send_position_notification(&bot, user_id, &wallet_address, &change)
                                    .await
                            {
                                log::error!("Failed to send notification to {}: {}", user_id, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("Failed to fetch positions for {}: {}", wallet_address, e);
                }
            }
        }
    }
}

async fn fetch_positions(client: &Client, wallet_address: &str) -> anyhow::Result<Vec<Position>> {
    let request_body = serde_json::json!({
        "type": "clearinghouseState",
        "user": wallet_address
    });

    let response = client
        .post(HYPERLIQUID_API)
        .json(&request_body)
        .send()
        .await?;

    let user_state: UserState = response.json().await?;

    Ok(user_state
        .asset_positions
        .into_iter()
        .map(|ap| ap.position)
        .collect())
}

#[derive(Debug)]
pub enum PositionChange {
    Opened {
        coin: String,
        size: String,
        entry_price: String,
        leverage: String,
    },
    Closed {
        coin: String,
    },
    Updated {
        coin: String,
        old_size: String,
        new_size: String,
        entry_price: String,
    },
}

async fn detect_position_changes(
    state: &Arc<RwLock<PositionTracker>>,
    wallet_address: &str,
    current_positions: &[Position],
) -> Vec<PositionChange> {
    let mut changes = Vec::new();
    let mut state = state.write().await;

    let old_positions = state
        .positions
        .entry(wallet_address.to_string())
        .or_default();

    let current_map: HashMap<String, &Position> = current_positions
        .iter()
        .filter(|p| p.szi.parse::<f64>().unwrap_or(0.0) != 0.0)
        .map(|p| (p.coin.clone(), p))
        .collect();

    // Check for closed positions
    let old_coins: Vec<String> = old_positions.keys().cloned().collect();
    for coin in old_coins {
        if !current_map.contains_key(&coin) {
            changes.push(PositionChange::Closed { coin: coin.clone() });
            old_positions.remove(&coin);
        }
    }

    // Check for new or updated positions
    for (coin, position) in &current_map {
        let size = &position.szi;

        if let Some(old_size) = old_positions.get(coin) {
            if old_size != size {
                changes.push(PositionChange::Updated {
                    coin: coin.clone(),
                    old_size: old_size.clone(),
                    new_size: size.clone(),
                    entry_price: position.entry_px.clone().unwrap_or_default(),
                });
            }
        } else {
            let leverage = position
                .leverage
                .as_ref()
                .map(|l| format!("{}x", l.value))
                .unwrap_or_else(|| "N/A".to_string());

            changes.push(PositionChange::Opened {
                coin: coin.clone(),
                size: size.clone(),
                entry_price: position.entry_px.clone().unwrap_or_default(),
                leverage,
            });
        }

        old_positions.insert(coin.clone(), size.clone());
    }

    changes
}

async fn send_position_notification(
    bot: &Bot,
    user_id: i64,
    wallet_address: &str,
    change: &PositionChange,
) -> anyhow::Result<()> {
    let short_wallet = format!(
        "{}...{}",
        &wallet_address[..6],
        &wallet_address[wallet_address.len() - 4..]
    );

    let message = match change {
        PositionChange::Opened {
            coin,
            size,
            entry_price,
            leverage,
        } => {
            let direction = if size.starts_with('-') {
                "🔴 SHORT"
            } else {
                "🟢 LONG"
            };
            let size_abs = size.trim_start_matches('-');

            format!(
                "<b>📈 Position Opened</b>\n\n\
                 <b>Wallet:</b> <code>{}</code>\n\
                 <b>Coin:</b> {}\n\
                 <b>Direction:</b> {}\n\
                 <b>Size:</b> {}\n\
                 <b>Entry:</b> ${}\n\
                 <b>Leverage:</b> {}",
                short_wallet, coin, direction, size_abs, entry_price, leverage
            )
        }
        PositionChange::Closed { coin } => {
            format!(
                "<b>📉 Position Closed</b>\n\n\
                 <b>Wallet:</b> <code>{}</code>\n\
                 <b>Coin:</b> {}",
                short_wallet, coin
            )
        }
        PositionChange::Updated {
            coin,
            old_size,
            new_size,
            entry_price,
        } => {
            let direction = if new_size.starts_with('-') {
                "🔴 SHORT"
            } else {
                "🟢 LONG"
            };

            format!(
                "<b>🔄 Position Updated</b>\n\n\
                 <b>Wallet:</b> <code>{}</code>\n\
                 <b>Coin:</b> {}\n\
                 <b>Direction:</b> {}\n\
                 <b>Size:</b> {} → {}\n\
                 <b>Entry:</b> ${}",
                short_wallet, coin, direction, old_size, new_size, entry_price
            )
        }
    };

    bot.send_message(ChatId(user_id), message)
        .parse_mode(ParseMode::Html)
        .await?;

    log::info!(
        "Sent notification to user {} for wallet {}",
        user_id,
        wallet_address
    );
    Ok(())
}
