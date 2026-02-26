# Onchain Message Service Dockerfile
FROM rust:1.91-bookworm AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*

# Set working directory
WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./

# Copy all workspace members (required for workspace to resolve)
COPY common ./common
COPY proto ./proto
COPY match_engine ./match_engine
COPY store ./store
COPY api ./api
COPY processor ./processor
COPY depth ./depth
COPY event ./event
COPY onchain_msg ./onchain_msg
COPY websocket_depth ./websocket_depth
COPY websocket_user ./websocket_user
COPY asset ./asset
COPY statistics ./statistics
COPY tests ./tests

# Build the project in release mode
RUN cargo build --release --bin onchain_msg

# Runtime stage
FROM debian:bookworm-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Create app directory
WORKDIR /app

# Copy the compiled binary from builder
COPY --from=builder /app/target/release/onchain_msg /app/onchain_msg

# Copy deploy directory for configuration
COPY deploy /app/deploy

# Create logs directory
RUN mkdir -p /app/logs

# Set environment variable
ENV RUST_BACKTRACE=full

# Run the binary
CMD ["/app/onchain_msg"]
