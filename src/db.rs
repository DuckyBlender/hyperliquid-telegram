use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

pub async fn init_db(database_url: &str) -> anyhow::Result<SqlitePool> {
    // Create the database file if it doesn't exist
    let db_path = database_url.strip_prefix("sqlite:").unwrap_or(database_url);
    if !std::path::Path::new(db_path).exists() {
        std::fs::File::create(db_path)?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    // Run migrations
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS tracked_wallets (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            wallet_address TEXT NOT NULL,
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
            UNIQUE(user_id, wallet_address)
        )
        "#,
    )
    .execute(&pool)
    .await?;

    log::info!("Database initialized successfully");
    Ok(pool)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrackedWallet {
    #[allow(dead_code)]
    pub id: i64,
    pub user_id: i64,
    pub wallet_address: String,
}

pub async fn add_wallet(
    pool: &SqlitePool,
    user_id: i64,
    wallet_address: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT OR IGNORE INTO tracked_wallets (user_id, wallet_address) VALUES (?, ?)",
    )
    .bind(user_id)
    .bind(wallet_address.to_lowercase())
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn remove_wallet(
    pool: &SqlitePool,
    user_id: i64,
    wallet_address: &str,
) -> anyhow::Result<bool> {
    let result =
        sqlx::query("DELETE FROM tracked_wallets WHERE user_id = ? AND wallet_address = ?")
            .bind(user_id)
            .bind(wallet_address.to_lowercase())
            .execute(pool)
            .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn get_user_wallets(
    pool: &SqlitePool,
    user_id: i64,
) -> anyhow::Result<Vec<TrackedWallet>> {
    let wallets = sqlx::query_as::<_, TrackedWallet>(
        "SELECT id, user_id, wallet_address FROM tracked_wallets WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}

pub async fn get_all_tracked_wallets(pool: &SqlitePool) -> anyhow::Result<Vec<TrackedWallet>> {
    let wallets = sqlx::query_as::<_, TrackedWallet>(
        "SELECT id, user_id, wallet_address FROM tracked_wallets",
    )
    .fetch_all(pool)
    .await?;

    Ok(wallets)
}
