use log::{error, info, warn};
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
    #[serde(rename = "liquidationPx")]
    pub liquidation_px: Option<String>,
    #[serde(rename = "marginUsed")]
    pub margin_used: String,
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
    pub margin_summary: MarginSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MarginSummary {
    pub account_value: String,
    pub total_margin_used: String,
}

#[derive(Debug, Clone)]
pub struct CachedPosition {
    pub size: String,
    pub entry_px: String,
    pub margin_used: String,
    pub unrealized_pnl: String,
    pub leverage: u32,
}

#[derive(Debug, Clone)]
pub struct PositionTracker {
    pub positions: HashMap<String, HashMap<String, CachedPosition>>,
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

    info!("Position monitoring started");

    loop {
        interval.tick().await;

        let wallets = match crate::db::get_all_tracked_wallets(&pool).await {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to fetch tracked wallets: {}", e);
                continue;
            }
        };

        let mut wallet_users: HashMap<String, Vec<i64>> = HashMap::new();
        for wallet in &wallets {
            wallet_users
                .entry(wallet.wallet_address.clone())
                .or_default()
                .push(wallet.user_id);
        }

        for (wallet_address, user_ids) in wallet_users {
            match fetch_user_state(&client, &wallet_address).await {
                Ok(user_state) => {
                    let changes =
                        detect_position_changes(&state, &wallet_address, &user_state).await;

                    for change in changes {
                        for &user_id in &user_ids {
                            if let Err(e) =
                                send_position_notification(&bot, user_id, &wallet_address, &change)
                                    .await
                            {
                                error!("Failed to send notification to {}: {}", user_id, e);
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to fetch positions for {}: {}", wallet_address, e);
                }
            }
        }
    }
}

async fn fetch_user_state(client: &Client, wallet_address: &str) -> anyhow::Result<UserState> {
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
    Ok(user_state)
}

#[derive(Debug)]
pub enum PositionChange {
    Opened {
        coin: String,
        size: f64,
        entry_price: f64,
        leverage: u32,
        position_value: f64,
        is_long: bool,
    },
    Closed {
        coin: String,
        realized_pnl: f64,
        entry_price: f64,
        was_long: bool,
        leverage: u32,
    },
    Increased {
        coin: String,
        old_size: f64,
        new_size: f64,
        entry_price: f64,
        leverage: u32,
        is_long: bool,
    },
    Decreased {
        coin: String,
        old_size: f64,
        new_size: f64,
        entry_price: f64,
        realized_pnl: f64,
        leverage: u32,
        is_long: bool,
    },
    MarginAdded {
        coin: String,
        old_margin: f64,
        new_margin: f64,
        leverage: u32,
        is_long: bool,
    },
    MarginRemoved {
        coin: String,
        old_margin: f64,
        new_margin: f64,
        leverage: u32,
        is_long: bool,
    },
    Liquidated {
        coin: String,
        lost_margin: f64,
        was_long: bool,
        leverage: u32,
    },
}

async fn detect_position_changes(
    state: &Arc<RwLock<PositionTracker>>,
    wallet_address: &str,
    user_state: &UserState,
) -> Vec<PositionChange> {
    let mut changes = Vec::new();
    let mut state = state.write().await;

    let old_positions = state
        .positions
        .entry(wallet_address.to_string())
        .or_default();

    let current_map: HashMap<String, &Position> = user_state
        .asset_positions
        .iter()
        .filter(|ap| ap.position.szi.parse::<f64>().unwrap_or(0.0) != 0.0)
        .map(|ap| (ap.position.coin.clone(), &ap.position))
        .collect();

    // Check for closed/liquidated positions
    let old_coins: Vec<String> = old_positions.keys().cloned().collect();
    for coin in old_coins {
        if !current_map.contains_key(&coin)
            && let Some(old_pos) = old_positions.remove(&coin)
        {
            let was_long = !old_pos.size.starts_with('-');
            let old_size: f64 = old_pos.size.parse::<f64>().unwrap_or(0.0).abs();
            let margin: f64 = old_pos.margin_used.parse().unwrap_or(0.0);
            let unrealized_pnl: f64 = old_pos.unrealized_pnl.parse().unwrap_or(0.0);

            // If margin was significant and PnL is very negative (close to -margin), likely liquidated
            let is_liquidated =
                margin > 0.0 && unrealized_pnl < 0.0 && (unrealized_pnl.abs() / margin) > 0.9;

            let entry_price: f64 = old_pos.entry_px.parse().unwrap_or(0.0);

            if is_liquidated && old_size > 0.0 {
                changes.push(PositionChange::Liquidated {
                    coin,
                    lost_margin: margin,
                    was_long,
                    leverage: old_pos.leverage,
                });
            } else {
                changes.push(PositionChange::Closed {
                    coin,
                    realized_pnl: unrealized_pnl,
                    entry_price,
                    was_long,
                    leverage: old_pos.leverage,
                });
            }
        }
    }

    // Check for new or updated positions
    for (coin, position) in &current_map {
        let new_size: f64 = position.szi.parse().unwrap_or(0.0);
        let is_long = new_size > 0.0;
        let entry_price: f64 = position
            .entry_px
            .as_ref()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0.0);
        let new_margin: f64 = position.margin_used.parse().unwrap_or(0.0);
        let position_value: f64 = position.position_value.parse().unwrap_or(0.0);
        let leverage = position.leverage.as_ref().map(|l| l.value).unwrap_or(1);

        if let Some(old_pos) = old_positions.get(coin) {
            let old_size: f64 = old_pos.size.parse().unwrap_or(0.0);
            let old_margin: f64 = old_pos.margin_used.parse().unwrap_or(0.0);
            let old_pnl: f64 = old_pos.unrealized_pnl.parse().unwrap_or(0.0);

            // Check for size changes
            let size_diff = (new_size.abs() - old_size.abs()).abs();
            if size_diff > 0.0001 {
                if new_size.abs() > old_size.abs() {
                    changes.push(PositionChange::Increased {
                        coin: coin.clone(),
                        old_size: old_size.abs(),
                        new_size: new_size.abs(),
                        entry_price,
                        leverage,
                        is_long,
                    });
                } else {
                    // When decreasing, estimate realized PnL based on proportion closed
                    let closed_ratio = (old_size.abs() - new_size.abs()) / old_size.abs();
                    let realized_pnl = old_pnl * closed_ratio;

                    changes.push(PositionChange::Decreased {
                        coin: coin.clone(),
                        old_size: old_size.abs(),
                        new_size: new_size.abs(),
                        entry_price,
                        realized_pnl,
                        leverage,
                        is_long,
                    });
                }
            }
            // Check for margin changes (if size didn't change significantly)
            else {
                let margin_diff = (new_margin - old_margin).abs();
                if margin_diff > 0.01 {
                    if new_margin > old_margin {
                        changes.push(PositionChange::MarginAdded {
                            coin: coin.clone(),
                            old_margin,
                            new_margin,
                            leverage,
                            is_long,
                        });
                    } else {
                        changes.push(PositionChange::MarginRemoved {
                            coin: coin.clone(),
                            old_margin,
                            new_margin,
                            leverage,
                            is_long,
                        });
                    }
                }
            }
        } else {
            changes.push(PositionChange::Opened {
                coin: coin.clone(),
                size: new_size.abs(),
                entry_price,
                leverage,
                position_value,
                is_long,
            });
        }

        old_positions.insert(
            coin.clone(),
            CachedPosition {
                size: position.szi.clone(),
                entry_px: position.entry_px.clone().unwrap_or_default(),
                margin_used: position.margin_used.clone(),
                unrealized_pnl: position.unrealized_pnl.clone(),
                leverage,
            },
        );
    }

    changes
}

fn format_pnl(pnl: f64) -> String {
    if pnl >= 0.0 {
        format!("🟢 +${:.2}", pnl)
    } else {
        format!("🔴 -${:.2}", pnl.abs())
    }
}

fn direction_emoji(is_long: bool) -> &'static str {
    if is_long { "🟢" } else { "🔴" }
}

fn direction_str(is_long: bool) -> &'static str {
    if is_long { "Long" } else { "Short" }
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
            position_value,
            is_long,
        } => {
            format!(
                "<b>{} {}x {} {} Opened</b>\n\n\
                 <code>{}</code>\n\
                 Size: {} | ${:.2}\n\
                 Entry: ${:.4}",
                direction_emoji(*is_long),
                leverage,
                coin,
                direction_str(*is_long),
                short_wallet,
                size,
                position_value,
                entry_price
            )
        }
        PositionChange::Closed {
            coin,
            realized_pnl,
            entry_price,
            was_long,
            leverage,
        } => {
            format!(
                "<b>{} {}x {} {} Closed</b>\n\n\
                 <code>{}</code>\n\
                 Entry: ${:.4}\n\
                 PnL: {}",
                direction_emoji(*was_long),
                leverage,
                coin,
                direction_str(*was_long),
                short_wallet,
                entry_price,
                format_pnl(*realized_pnl)
            )
        }
        PositionChange::Increased {
            coin,
            old_size,
            new_size,
            entry_price,
            leverage,
            is_long,
        } => {
            format!(
                "<b>{} {}x {} {} Increased</b>\n\n\
                 <code>{}</code>\n\
                 Size: {:.4} → {:.4}\n\
                 Entry: ${:.4}",
                direction_emoji(*is_long),
                leverage,
                coin,
                direction_str(*is_long),
                short_wallet,
                old_size,
                new_size,
                entry_price
            )
        }
        PositionChange::Decreased {
            coin,
            old_size,
            new_size,
            entry_price,
            realized_pnl,
            leverage,
            is_long,
        } => {
            format!(
                "<b>{} {}x {} {} Decreased</b>\n\n\
                 <code>{}</code>\n\
                 Size: {:.4} → {:.4}\n\
                 Entry: ${:.4}\n\
                 PnL: {}",
                direction_emoji(*is_long),
                leverage,
                coin,
                direction_str(*is_long),
                short_wallet,
                old_size,
                new_size,
                entry_price,
                format_pnl(*realized_pnl)
            )
        }
        PositionChange::MarginAdded {
            coin,
            old_margin,
            new_margin,
            leverage,
            is_long,
        } => {
            format!(
                "<b>➕ {}x {} {} Margin Added</b>\n\n\
                 <code>{}</code>\n\
                 Margin: ${:.2} → ${:.2} (+${:.2})",
                leverage,
                coin,
                direction_str(*is_long),
                short_wallet,
                old_margin,
                new_margin,
                new_margin - old_margin
            )
        }
        PositionChange::MarginRemoved {
            coin,
            old_margin,
            new_margin,
            leverage,
            is_long,
        } => {
            format!(
                "<b>➖ {}x {} {} Margin Removed</b>\n\n\
                 <code>{}</code>\n\
                 Margin: ${:.2} → ${:.2} (-${:.2})",
                leverage,
                coin,
                direction_str(*is_long),
                short_wallet,
                old_margin,
                new_margin,
                old_margin - new_margin
            )
        }
        PositionChange::Liquidated {
            coin,
            lost_margin,
            was_long,
            leverage,
        } => {
            format!(
                "<b>💀 {}x {} {} Liquidated</b>\n\n\
                 <code>{}</code>\n\
                 Lost: 🔴 -${:.2}",
                leverage,
                coin,
                direction_str(*was_long),
                short_wallet,
                lost_margin
            )
        }
    };

    bot.send_message(ChatId(user_id), message)
        .parse_mode(ParseMode::Html)
        .await?;

    info!(
        "Sent notification to user {} for wallet {}",
        user_id, wallet_address
    );
    Ok(())
}
