# Asset Service Dockerfile
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
COPY asset ./asset
COPY match_engine ./match_engine
COPY store ./store
COPY api ./api
COPY processor ./processor
COPY depth ./depth
COPY event ./event
COPY onchain_msg ./onchain_msg
COPY websocket_depth ./websocket_depth
COPY websocket_user ./websocket_user
COPY statistics ./statistics
COPY tests ./tests

# Build the project in release mode
RUN cargo build --release --bin asset

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
COPY --from=builder /app/target/release/asset /app/asset

# Copy deploy directory for configuration
COPY deploy /app/deploy

# Create logs directory
RUN mkdir -p /app/logs

# Set environment variable
ENV RUST_BACKTRACE=full

# Expose the gRPC port
EXPOSE 5003

# Run the binary
CMD ["/app/asset"]
