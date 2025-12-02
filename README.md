# Hyperliquid Telegram Bot

A Telegram bot that tracks wallet positions on Hyperliquid and sends notifications when positions change.

## Features

- 📊 **Position Tracking** - Monitor any wallet's positions on Hyperliquid
- 🔔 **Real-time Notifications** - Get alerts when positions are opened, closed, increased, or decreased within 10 seconds (realtime coming soon)
- 📋 **Multiple Wallets** - Track multiple wallets with optional notes/labels
- 📈 **View Positions** - Check current open positions for all tracked wallets

## Commands

| Command | Description |
|---------|-------------|
| `/start` | Start the bot and see welcome message |
| `/help` | Display available commands |
| `/add <wallet> [note]` | Add a wallet to track (with optional note) |
| `/remove <wallet>` | Stop tracking a wallet |
| `/list` | List all tracked wallets |
| `/positions <wallet>` | Show current open positions for a wallet |

## Setup

### Standalone

1. Create a `.env` file with your Telegram bot token:
   ```
   TELOXIDE_TOKEN=your_bot_token_here
   ```
2. Run the bot:
   ```bash
   cargo run --release
   ```

The database will be created at the path specified by `DATABASE_URL` (defaults to `sqlite:data/bot.db?mode=rwc`).

### Docker

1. Create a `.env` file with your Telegram bot token:
   ```
   TELOXIDE_TOKEN=your_bot_token_here
   ```
   
2. Run with Docker Compose:
   ```bash
   docker compose up -d
   ```

The `data/` directory is mounted as a volume to persist the SQLite database. This directory mount is required (rather than mounting a single file) because SQLite creates additional temporary files (`-wal`, `-shm`) alongside the main database.
