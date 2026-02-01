use log::{error, info, warn};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::Arc;
use teloxide::{prelude::*, types::ParseMode};
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};

use crate::bot::format_wallet_display;
use crate::db;

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
pub struct CachedPosition {
    pub size: String,
    pub entry_px: String,
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

    /// Load positions from database into memory
    pub fn from_db_positions(
        db_positions: HashMap<String, HashMap<String, db::ActivePosition>>,
    ) -> Self {
        let positions = db_positions
            .into_iter()
            .map(|(wallet, coins)| {
                let cached_coins = coins
                    .into_iter()
                    .map(|(coin, pos)| {
                        (
                            coin,
                            CachedPosition {
                                size: pos.size,
                                entry_px: pos.entry_px,
                                unrealized_pnl: pos.unrealized_pnl,
                                leverage: pos.leverage as u32,
                            },
                        )
                    })
                    .collect();
                (wallet, cached_coins)
            })
            .collect();

        Self { positions }
    }
}

pub async fn monitor_positions(pool: SqlitePool, bot: Bot, state: Arc<RwLock<PositionTracker>>) {
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("Failed to create HTTP client");

    // Load existing positions from DB into tracker on startup
    match db::get_all_active_positions(&pool).await {
        Ok(db_positions) => {
            let position_count: usize = db_positions.values().map(|v| v.len()).sum();
            let mut tracker = state.write().await;
            *tracker = PositionTracker::from_db_positions(db_positions);
            info!(
                "Loaded {} positions for {} wallets from database",
                position_count,
                tracker.positions.len()
            );
        }
        Err(e) => {
            error!("Failed to load positions from database: {}", e);
        }
    }

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

        let mut wallet_users: HashMap<String, Vec<(i64, Option<String>)>> = HashMap::new();
        for wallet in &wallets {
            wallet_users
                .entry(wallet.wallet_address.clone())
                .or_default()
                .push((wallet.user_id, wallet.note.clone()));
        }

        for (wallet_address, user_infos) in wallet_users {
            match fetch_user_state(&client, &wallet_address).await {
                Ok(user_state) => {
                    let changes =
                        detect_position_changes(&pool, &state, &wallet_address, &user_state).await;

                    for change in changes {
                        for (user_id, note) in &user_infos {
                            if let Err(e) = send_position_notification(
                                &bot,
                                *user_id,
                                &wallet_address,
                                note.as_deref(),
                                &change,
                            )
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

pub async fn fetch_user_state(client: &Client, wallet_address: &str) -> anyhow::Result<UserState> {
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
        size: f64,
    },
    Increased {
        coin: String,
        old_size: f64,
        new_size: f64,
        entry_price: f64,
        leverage: u32,
        is_long: bool,
        unrealized_pnl: f64,
        position_value: f64,
    },
    Decreased {
        coin: String,
        old_size: f64,
        new_size: f64,
        entry_price: f64,
        leverage: u32,
        is_long: bool,
        unrealized_pnl: f64,
        position_value: f64,
    },
}

async fn detect_position_changes(
    pool: &SqlitePool,
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

    // Check for closed positions
    let old_coins: Vec<String> = old_positions.keys().cloned().collect();
    for coin in old_coins {
        if !current_map.contains_key(&coin)
            && let Some(old_pos) = old_positions.remove(&coin)
        {
            let was_long = !old_pos.size.starts_with('-');
            let unrealized_pnl: f64 = old_pos.unrealized_pnl.parse().unwrap_or(0.0);
            let entry_price: f64 = old_pos.entry_px.parse().unwrap_or(0.0);
            let size: f64 = old_pos.size.parse::<f64>().unwrap_or(0.0).abs();

            // Delete from DB
            if let Err(e) = db::delete_position(pool, wallet_address, &coin).await {
                error!("Failed to delete position from DB: {}", e);
            }

            changes.push(PositionChange::Closed {
                coin,
                realized_pnl: unrealized_pnl,
                entry_price,
                was_long,
                leverage: old_pos.leverage,
                size,
            });
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
        let position_value: f64 = position.position_value.parse().unwrap_or(0.0);
        let leverage = position.leverage.as_ref().map(|l| l.value).unwrap_or(1);
        let entry_px_str = position.entry_px.clone().unwrap_or_default();

        let has_changed = if let Some(old_pos) = old_positions.get(coin) {
            let old_size: f64 = old_pos.size.parse().unwrap_or(0.0);

            // Check for size changes
            let size_diff = (new_size.abs() - old_size.abs()).abs();
            if size_diff > 0.0001 {
                if new_size.abs() > old_size.abs() {
                    let unrealized_pnl: f64 = position.unrealized_pnl.parse().unwrap_or(0.0);
                    changes.push(PositionChange::Increased {
                        coin: coin.clone(),
                        old_size: old_size.abs(),
                        new_size: new_size.abs(),
                        entry_price,
                        leverage,
                        is_long,
                        unrealized_pnl,
                        position_value,
                    });
                } else {
                    let unrealized_pnl: f64 = position.unrealized_pnl.parse().unwrap_or(0.0);

                    changes.push(PositionChange::Decreased {
                        coin: coin.clone(),
                        old_size: old_size.abs(),
                        new_size: new_size.abs(),
                        entry_price,
                        leverage,
                        is_long,
                        unrealized_pnl,
                        position_value,
                    });
                }
                true
            } else {
                false
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
            true
        };

        // Update in-memory cache
        old_positions.insert(
            coin.clone(),
            CachedPosition {
                size: position.szi.clone(),
                entry_px: entry_px_str.clone(),
                unrealized_pnl: position.unrealized_pnl.clone(),
                leverage,
            },
        );

        // Only persist to DB when position has meaningful changes
        if has_changed
            && let Err(e) = db::upsert_position(
                pool,
                wallet_address,
                coin,
                &position.szi,
                &entry_px_str,
                &position.unrealized_pnl,
                leverage as i64,
            )
            .await
        {
            error!("Failed to upsert position to DB: {}", e);
        }
    }

    changes
}

fn format_pnl(pnl: f64) -> String {
    if pnl >= 0.0 {
        format!("+${:.2}", pnl)
    } else {
        format!("-${:.2}", pnl.abs())
    }
}

fn format_price(price: f64) -> String {
    let s = format!("${}", price);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn format_size(size: f64) -> String {
    let s = format!("{}", size);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn direction_str(is_long: bool) -> &'static str {
    if is_long { "Long" } else { "Short" }
}

fn calculate_current_price_info(entry_price: f64, position_value: f64, size: f64) -> (f64, String) {
    let current_price = if size > 0.0 {
        position_value / size
    } else {
        0.0
    };
    let current_price_rounded = (current_price * 10000000000.0).round() / 10000000000.0;
    let price_diff = current_price_rounded - entry_price;
    let price_diff_str = if price_diff >= 0.0 {
        format!("+${:.2}", price_diff)
    } else {
        format!("-${:.2}", price_diff.abs())
    };
    (current_price_rounded, price_diff_str)
}

fn calculate_pnl_percent(entry_price: f64, size: f64, unrealized_pnl: f64) -> String {
    let entry_value = entry_price * size;
    let pnl_pct = if entry_value > 0.0 {
        (unrealized_pnl / entry_value) * 100.0
    } else {
        0.0
    };
    if pnl_pct >= 0.0 {
        format!("+{:.2}%", pnl_pct)
    } else {
        format!("{:.2}%", pnl_pct)
    }
}

async fn send_position_notification(
    bot: &Bot,
    user_id: i64,
    wallet_address: &str,
    note: Option<&str>,
    change: &PositionChange,
) -> anyhow::Result<()> {
    let wallet_display = format_wallet_display(wallet_address, note, false);

    let hyperdash_link = format!(
        "🌐 <a href=\"https://app.coinmarketman.com/hypertracker/wallet/{}\">Hypertracker</a>",
        wallet_address
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
                "<b>📈 {}x {} {} Opened</b>\n\n\
                 👛 Wallet: {}\n\
                 📊 Size: {} {} (${:.2})\n\
                 💰 Entry: {}\n\
                 {}",
                leverage,
                coin,
                direction_str(*is_long),
                wallet_display,
                format_size(*size),
                coin,
                position_value,
                format_price(*entry_price),
                hyperdash_link
            )
        }
        PositionChange::Closed {
            coin,
            realized_pnl,
            entry_price,
            was_long,
            leverage,
            size,
        } => {
            // Calculate exit price from PnL
            // For longs: exit_price = entry_price + (pnl / size)
            // For shorts: exit_price = entry_price - (pnl / size)
            let exit_price = if *size > 0.0 {
                if *was_long {
                    entry_price + (realized_pnl / size)
                } else {
                    entry_price - (realized_pnl / size)
                }
            } else {
                *entry_price
            };
            let exit_price_rounded = (exit_price * 10000000000.0).round() / 10000000000.0;
            let price_diff = exit_price_rounded - entry_price;
            let price_diff_str = if price_diff >= 0.0 {
                format!("+${:.2}", price_diff)
            } else {
                format!("-${:.2}", price_diff.abs())
            };

            format!(
                "<b>📉 {}x {} {} Closed</b>\n\n\
                 👛 Wallet: {}\n\
                 💰 Entry: {}\n\
                 📍 Exit: {} ({})\n\
                 💵 PnL: {}\n\
                 {}",
                leverage,
                coin,
                direction_str(*was_long),
                wallet_display,
                format_price(*entry_price),
                format_price(exit_price_rounded),
                price_diff_str,
                format_pnl(*realized_pnl),
                hyperdash_link
            )
        }
        PositionChange::Increased {
            coin,
            old_size,
            new_size,
            entry_price,
            leverage,
            is_long,
            unrealized_pnl,
            position_value,
        } => {
            let (current_price_rounded, price_diff_str) =
                calculate_current_price_info(*entry_price, *position_value, *new_size);
            let pnl_pct_str = calculate_pnl_percent(*entry_price, *new_size, *unrealized_pnl);
            let size_change_pct = if *old_size > 0.0 {
                ((new_size - old_size) / old_size) * 100.0
            } else {
                0.0
            };
            format!(
                "<b>⬆️ {}x {} {} Increased</b>\n\n\
                 👛 Wallet: {}\n\
                 📊 Size: {} → {} {} (+{:.2}%)\n\
                 💰 Entry: {}\n\
                 📍 Current: {} ({})\n\
                 💵 PnL: {} ({})\n\
                 {}",
                leverage,
                coin,
                direction_str(*is_long),
                wallet_display,
                format_size(*old_size),
                format_size(*new_size),
                coin,
                size_change_pct,
                format_price(*entry_price),
                format_price(current_price_rounded),
                price_diff_str,
                format_pnl(*unrealized_pnl),
                pnl_pct_str,
                hyperdash_link
            )
        }
        PositionChange::Decreased {
            coin,
            old_size,
            new_size,
            entry_price,
            leverage,
            is_long,
            unrealized_pnl,
            position_value,
        } => {
            let (current_price_rounded, price_diff_str) =
                calculate_current_price_info(*entry_price, *position_value, *new_size);
            let pnl_pct_str = calculate_pnl_percent(*entry_price, *new_size, *unrealized_pnl);
            let size_change_pct = if *old_size > 0.0 {
                ((old_size - new_size) / old_size) * 100.0
            } else {
                0.0
            };
            format!(
                "<b>⬇️ {}x {} {} Decreased</b>\n\n\
                 👛 Wallet: {}\n\
                 📊 Size: {} → {} {} (-{:.2}%)\n\
                 💰 Entry: {}\n\
                 📍 Current: {} ({})\n\
                 💵 PnL: {} ({})\n\
                 {}",
                leverage,
                coin,
                direction_str(*is_long),
                wallet_display,
                format_size(*old_size),
                format_size(*new_size),
                coin,
                size_change_pct,
                format_price(*entry_price),
                format_price(current_price_rounded),
                price_diff_str,
                format_pnl(*unrealized_pnl),
                pnl_pct_str,
                hyperdash_link
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
