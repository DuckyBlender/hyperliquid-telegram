CREATE TABLE IF NOT EXISTS active_positions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    wallet_address TEXT NOT NULL,
    coin TEXT NOT NULL,
    size TEXT NOT NULL,
    entry_px TEXT NOT NULL,
    unrealized_pnl TEXT NOT NULL,
    leverage INTEGER NOT NULL,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(wallet_address, coin)
);
