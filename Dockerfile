# ─── Build stage ──────────────────────────────────────────────────────────────
FROM rust:slim AS builder

WORKDIR /app

# sqlx uses native-tls; needs OpenSSL headers at compile time
RUN apt-get update && \
    apt-get install -y --no-install-recommends pkg-config libssl-dev && \
    rm -rf /var/lib/apt/lists/*

# Override the project's rust-toolchain.toml (channel = "esp")
# The host server compiles cleanly with stable.
ENV RUSTUP_TOOLCHAIN=stable

COPY . .

RUN cargo build --release --features host-server --bin telemetry-server

# ─── Runtime stage ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libssl3 && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/telemetry-server /usr/local/bin/telemetry-server

# HTTP UI / API
EXPOSE 8080
# Robot telemetry (UDP)
EXPOSE 9001/udp

CMD ["telemetry-server"]
