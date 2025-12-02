CREATE TABLE IF NOT EXISTS tracked_wallets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    wallet_address TEXT NOT NULL,
    note TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(user_id, wallet_address)
);

CREATE INDEX IF NOT EXISTS idx_tracked_wallets_user_id ON tracked_wallets(user_id, id);

CREATE TABLE IF NOT EXISTS active_positions (
    wallet_address TEXT NOT NULL,
    coin TEXT NOT NULL,
    size TEXT NOT NULL,
    entry_px TEXT NOT NULL,
    unrealized_pnl TEXT NOT NULL,
    leverage INTEGER NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(wallet_address, coin)
);
