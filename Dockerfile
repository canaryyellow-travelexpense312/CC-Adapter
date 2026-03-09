# ---- Build stage ----
FROM rust:1-bookworm AS builder

WORKDIR /app

# Cache dependencies: copy manifests first, build with a dummy main
COPY Cargo.toml ./
COPY Cargo.lock* ./
RUN mkdir src && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ---- Runtime stage ----
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/claude-adapter /usr/local/bin/
COPY config.toml /app/config.toml

WORKDIR /app

EXPOSE 8080

# Default: listen on 0.0.0.0 so the port is reachable outside the container
CMD ["claude-adapter", "serve", "--config", "/app/config.toml", "--host", "0.0.0.0"]
