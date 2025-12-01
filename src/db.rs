use log::info;
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

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
    #[allow(dead_code)]
    pub id: i64,
    pub user_id: i64,
    pub wallet_address: String,
    pub note: Option<String>,
}

pub async fn add_wallet(
    pool: &SqlitePool,
    user_id: i64,
    wallet_address: &str,
    note: Option<&str>,
) -> anyhow::Result<bool> {
    let wallet_lower = wallet_address.to_lowercase();
    let result = sqlx::query!(
        "INSERT OR IGNORE INTO tracked_wallets (user_id, wallet_address, note) VALUES (?, ?, ?)",
        user_id,
        wallet_lower,
        note
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
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
        r#"SELECT id as "id!: i64", user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets WHERE user_id = ?"#,
        user_id
    )
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}

pub async fn get_all_tracked_wallets(pool: &SqlitePool) -> anyhow::Result<Vec<TrackedWallet>> {
    let wallets = sqlx::query_as!(
        TrackedWallet,
        r#"SELECT id as "id!: i64", user_id as "user_id!: i64", wallet_address, note FROM tracked_wallets"#
    )
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}
