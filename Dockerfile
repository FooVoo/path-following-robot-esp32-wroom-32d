# ─── Build stage ──────────────────────────────────────────────────────────────
FROM rust:slim AS builder

WORKDIR /app

# sqlx uses rustls (pure-Rust TLS) — no OpenSSL headers needed.
# protobuf codegen (prost-build) needs a C compiler.
RUN apt-get update && \
    apt-get install -y --no-install-recommends protobuf-compiler && \
    rm -rf /var/lib/apt/lists/*

# Override the project's rust-toolchain.toml (channel = "esp")
# The host server compiles cleanly with stable.
ENV RUSTUP_TOOLCHAIN=stable

COPY . .

# .cargo/config.toml sets [build] target = "xtensa-esp32-none-elf" for the ESP32
# firmware.  Override it here so the telemetry-server binary compiles against the
# container's host target instead of the bare-metal ESP32 target.
# Stage the binary to a fixed path so the COPY in the runtime stage is simple.
RUN HOST_TARGET=$(rustc -vV | sed -n 's|host: ||p') && \
    CARGO_BUILD_TARGET="$HOST_TARGET" \
    cargo build --release --features host-server --bin telemetry-server && \
    cp "target/$HOST_TARGET/release/telemetry-server" /tmp/telemetry-server

# ─── Runtime stage ────────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /tmp/telemetry-server /usr/local/bin/telemetry-server

# HTTP UI / API
EXPOSE 8080
# Robot telemetry (UDP)
EXPOSE 9001/udp

CMD ["telemetry-server"]
