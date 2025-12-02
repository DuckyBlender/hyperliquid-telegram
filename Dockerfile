# Stage 1: Tools - install cargo-chef and sqlx-cli (cached layer)
FROM rust:slim AS tools
RUN cargo install cargo-chef 
RUN cargo install sqlx-cli --no-default-features --features sqlite,rustls

# Stage 2: Chef - base with tools
FROM rust:slim AS chef
COPY --from=tools /usr/local/cargo/bin/cargo-chef /usr/local/cargo/bin/cargo-chef
COPY --from=tools /usr/local/cargo/bin/sqlx /usr/local/cargo/bin/sqlx
WORKDIR /app

# Stage 3: Planner - create recipe.json
FROM chef AS planner
COPY Cargo.toml ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

# Stage 4: Builder - build dependencies and application
FROM chef AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy recipe and build dependencies
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy source code and migrations
COPY Cargo.toml ./
COPY src ./src
COPY migrations ./migrations

# Create database and run migrations to generate sqlx cache
ENV DATABASE_URL=sqlite:/app/build.db
RUN sqlx database create && sqlx migrate run

# Build application (sqlx will verify queries against the database)
RUN cargo build --release

# Stage 4: Runtime - minimal image
FROM debian:bookworm-slim AS runtime
WORKDIR /app

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy binary from builder
COPY --from=builder /app/target/release/hyperliquid-telegram /app/hyperliquid-telegram

# Copy migrations for runtime migration support
COPY migrations ./migrations

# Create data directory for SQLite database
RUN mkdir -p /app/data

ENV DATABASE_URL=sqlite:/app/data/bot.db?mode=rwc

ENTRYPOINT ["/app/hyperliquid-telegram"]
