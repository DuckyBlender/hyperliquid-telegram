# Hyperliquid Telegram Bot

A Telegram bot that tracks wallet positions on Hyperliquid and sends notifications when positions change.

## Features

- 📊 **Position Tracking** - Monitor any wallet's positions on Hyperliquid
- 🔔 **Real-time Notifications** - Get alerts when positions are opened, closed, increased, or decreased
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

1. Set the `TELOXIDE_TOKEN` environment variable with your Telegram bot token
2. Run the bot with `cargo run`

## Environment Variables

- `TELOXIDE_TOKEN` - Your Telegram Bot API token (required)
