use log::info;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use std::collections::HashMap;

pub const MAX_WALLETS_PER_USER: i64 = 10;

pub async fn init_db(database_url: &str) -> anyhow::Result<SqlitePool> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    sqlx::migrate!().run(&pool).await?;

    info!("Database initialized successfully");
    Ok(pool)
}

#[derive(Debug, Clone)]
pub struct TrackedWallet {
    pub user_id: i64,
    pub wallet_address: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ActivePosition {
    pub wallet_address: String,
    pub coin: String,
    pub size: String,
    pub entry_px: String,
    pub unrealized_pnl: String,
    pub leverage: i64,
}

pub async fn get_user_wallet_count(pool: &SqlitePool, user_id: i64) -> anyhow::Result<i64> {
    let count = sqlx::query_scalar!(
        "SELECT COUNT(*) as count FROM tracked_wallets WHERE user_id = ?",
        user_id
    )
    .fetch_one(pool)
    .await?;

    Ok(count as i64)
}

#[derive(Debug, Clone, PartialEq)]
pub enum AddWalletResult {
    Added,
    Updated,
    AlreadyExistsNoChange,
}

pub async fn add_wallet(
    pool: &SqlitePool,
    user_id: i64,
    wallet_address: &str,
    note: Option<&str>,
) -> anyhow::Result<AddWalletResult> {
    let wallet_lower = wallet_address.to_lowercase();

    // Check if wallet already exists and get current note
    let existing = sqlx::query_scalar!(
        "SELECT note FROM tracked_wallets WHERE user_id = ? AND wallet_address = ?",
        user_id,
        wallet_lower
    )
    .fetch_optional(pool)
    .await?;

    match existing {
        Some(existing_note) => {
            // Wallet exists, check if note is different
            if existing_note.as_deref() == note {
                Ok(AddWalletResult::AlreadyExistsNoChange)
            } else {
                // Update the note
                sqlx::query!(
                    "UPDATE tracked_wallets SET note = ? WHERE user_id = ? AND wallet_address = ?",
                    note,
                    user_id,
                    wallet_lower
                )
                .execute(pool)
                .await?;
                Ok(AddWalletResult::Updated)
            }
        }
        None => {
            // Insert new wallet
            sqlx::query!(
                "INSERT INTO tracked_wallets (user_id, wallet_address, note) VALUES (?, ?, ?)",
                user_id,
                wallet_lower,
                note
            )
            .execute(pool)
            .await?;
            Ok(AddWalletResult::Added)
        }
    }
}

pub async fn remove_wallet(
    pool: &SqlitePool,
    user_id: i64,
    wallet_address: &str,
) -> anyhow::Result<bool> {
    let wallet_lower = wallet_address.to_lowercase();
    let result = sqlx::query!(
        "DELETE FROM tracked_wallets WHERE user_id = ? AND wallet_address = ?",
        user_id,
        wallet_lower
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_user_wallets(
    pool: &SqlitePool,
    user_id: i64,
) -> anyhow::Result<Vec<TrackedWallet>> {
    let wallets = sqlx::query_as!(
        TrackedWallet,
        r#"SELECT user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets WHERE user_id = ? ORDER BY id"#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}

pub async fn get_all_tracked_wallets(pool: &SqlitePool) -> anyhow::Result<Vec<TrackedWallet>> {
    let wallets = sqlx::query_as!(
        TrackedWallet,
        r#"SELECT user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets"#
    )
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}

/// Load all active positions from DB, grouped by wallet address
pub async fn get_all_active_positions(
    pool: &SqlitePool,
) -> anyhow::Result<HashMap<String, HashMap<String, ActivePosition>>> {
    let positions = sqlx::query_as!(
        ActivePosition,
        r#"SELECT wallet_address, coin, size, entry_px, unrealized_pnl, leverage as "leverage!: i64" FROM active_positions"#
    )
    .fetch_all(pool)
    .await?;

    let mut result: HashMap<String, HashMap<String, ActivePosition>> = HashMap::new();
    for pos in positions {
        result
            .entry(pos.wallet_address.clone())
            .or_default()
            .insert(pos.coin.clone(), pos);
    }

    Ok(result)
}

/// Save or update an active position
pub async fn upsert_position(
    pool: &SqlitePool,
    wallet_address: &str,
    coin: &str,
    size: &str,
    entry_px: &str,
    unrealized_pnl: &str,
    leverage: i64,
) -> anyhow::Result<()> {
    let wallet_lower = wallet_address.to_lowercase();
    sqlx::query!(
        r#"INSERT INTO active_positions (wallet_address, coin, size, entry_px, unrealized_pnl, leverage, updated_at)
           VALUES (?, ?, ?, ?, ?, ?, CURRENT_TIMESTAMP)
           ON CONFLICT(wallet_address, coin) DO UPDATE SET
             size = excluded.size,
             entry_px = excluded.entry_px,
             unrealized_pnl = excluded.unrealized_pnl,
             leverage = excluded.leverage,
             updated_at = CURRENT_TIMESTAMP"#,
        wallet_lower,
        coin,
        size,
        entry_px,
        unrealized_pnl,
        leverage
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Delete a closed position
pub async fn delete_position(
    pool: &SqlitePool,
    wallet_address: &str,
    coin: &str,
) -> anyhow::Result<()> {
    let wallet_lower = wallet_address.to_lowercase();
    sqlx::query!(
        "DELETE FROM active_positions WHERE wallet_address = ? AND coin = ?",
        wallet_lower,
        coin
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Get the note for a wallet address (for any user)
pub async fn get_wallet_note(
    pool: &SqlitePool,
    wallet_address: &str,
) -> anyhow::Result<Option<String>> {
    let wallet_lower = wallet_address.to_lowercase();
    let result = sqlx::query_scalar!(
        "SELECT note FROM tracked_wallets WHERE wallet_address = ? LIMIT 1",
        wallet_lower
    )
    .fetch_optional(pool)
    .await?;

    Ok(result.flatten())
}

/// Check if a note already exists for a user (case-insensitive)
pub async fn note_exists_for_user(
    pool: &SqlitePool,
    user_id: i64,
    note: &str,
    exclude_wallet: Option<&str>,
) -> anyhow::Result<bool> {
    let note_lower = note.to_lowercase();
    let result = match exclude_wallet {
        Some(wallet) => {
            let wallet_lower = wallet.to_lowercase();
            sqlx::query_scalar!(
                "SELECT COUNT(*) as count FROM tracked_wallets WHERE user_id = ? AND LOWER(note) = ? AND wallet_address != ?",
                user_id,
                note_lower,
                wallet_lower
            )
            .fetch_one(pool)
            .await?
        }
        None => sqlx::query_scalar!(
            "SELECT COUNT(*) as count FROM tracked_wallets WHERE user_id = ? AND LOWER(note) = ?",
            user_id,
            note_lower
        )
        .fetch_one(pool)
        .await?,
    };

    Ok(result > 0)
}

/// Get wallet by index (1-based) for a user, ordered by id
pub async fn get_wallet_by_index(
    pool: &SqlitePool,
    user_id: i64,
    index: usize,
) -> anyhow::Result<Option<TrackedWallet>> {
    if index == 0 {
        return Ok(None);
    }
    let offset = (index - 1) as i64;
    let wallet = sqlx::query_as!(
        TrackedWallet,
        r#"SELECT user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets WHERE user_id = ? ORDER BY id LIMIT 1 OFFSET ?"#,
        user_id,
        offset
    )
    .fetch_optional(pool)
    .await?;

    Ok(wallet)
}

/// Get wallet by note (case-insensitive) for a user
pub async fn get_wallet_by_note(
    pool: &SqlitePool,
    user_id: i64,
    note: &str,
) -> anyhow::Result<Option<TrackedWallet>> {
    let note_lower = note.to_lowercase();
    let wallet = sqlx::query_as!(
        TrackedWallet,
        r#"SELECT user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets WHERE user_id = ? AND LOWER(note) = ?"#,
        user_id,
        note_lower
    )
    .fetch_optional(pool)
    .await?;

    Ok(wallet)
}
