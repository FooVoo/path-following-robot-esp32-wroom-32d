# ADR-007 — sqlx: rustls instead of native-tls

| Field      | Value          |
|------------|----------------|
| Date       | 2026-05-19     |
| Status     | **Accepted**   |
| Deciders   | FooVoo         |
| Supersedes | sqlx `runtime-tokio-native-tls` (pre ADR-007) |

---

## Context

The `telemetry-server` binary uses `sqlx` to persist telemetry frames in
PostgreSQL.  `sqlx` requires a TLS runtime feature to be selected at compile
time; two options exist:

| Feature flag                | TLS implementation | Native library needed |
|-----------------------------|--------------------|-----------------------|
| `runtime-tokio-native-tls`  | `native-tls` → system TLS (OpenSSL on Linux, SecureTransport on macOS, SChannel on Windows) | `libssl-dev` / `openssl-devel` at build time; `libssl` at runtime |
| `runtime-tokio-rustls`      | `rustls` — pure-Rust TLS 1.2/1.3 | None — all Rust, no C library |

The project's `.cargo/config.toml` sets a global default build target:

```toml
[build]
target = "xtensa-esp32-none-elf"
```

This is required so that `cargo build` (no flags) produces ESP32 firmware
out of the box.  The side effect is that any invocation of

```bash
cargo build --features host-server --bin telemetry-server
```

without an explicit `--target` override compiles for the bare-metal ESP32
target.  When the target is `xtensa-esp32-none-elf`, `openssl-sys`'s
build script runs, fails to locate an OpenSSL installation for the Xtensa
cross-toolchain, and aborts:

```
Could not find openssl via pkg-config: pkg-config has not been configured
to support cross-compilation.
$HOST = aarch64-apple-darwin
$TARGET = xtensa-esp32-none-elf
openssl-sys = 0.9.116
```

The same failure occurs in Docker because the `Dockerfile` previously relied
on the `target/release/` output path, which also requires no `--target`
override — inheriting the xtensa default again.

`native-tls`'s dependency on `openssl-sys` is therefore a persistent hazard
in this workspace: any developer or CI step that forgets `--target` triggers
the error, and the Docker build is architecturally fragile.

---

## Decision

**Replace `runtime-tokio-native-tls` with `runtime-tokio-rustls` in the
`sqlx` dependency.**

```toml
# Before
sqlx = { version = "0.8", features = ["runtime-tokio-native-tls", ...], optional = true }

# After
sqlx = { version = "0.8", features = ["runtime-tokio-rustls",      ...], optional = true }
```

`rustls` is a pure-Rust TLS 1.2/1.3 implementation.  It has no C FFI, no
system-library dependency, and compiles cleanly on every target that has
`std` — including the host target when `CARGO_BUILD_TARGET` is set
differently from `.cargo/config.toml`'s default.

### Supporting changes

| File | Change |
|---|---|
| `Dockerfile` (builder) | Removed `pkg-config libssl-dev`; added `CARGO_BUILD_TARGET=$(rustc -vV …)` to override the xtensa default; binary staged to `/tmp/` for a predictable `COPY` path |
| `Dockerfile` (runtime) | Removed `libssl3` (no longer needed) |
| `.cargo/config.toml` | Added `build-server` (macOS arm64) and `build-server-linux` (Linux x86_64) aliases that always pass the correct `--target` |
| `docs/runbooks/05-troubleshooting.md` | Added section documenting the exact error, its cause, and the correct build commands |
| `docs/runbooks/08-fleet-management.md` | Replaced the broken bare `cargo build --features host-server` with alias / explicit `--target` commands |

---

## Alternatives Considered

### Keep `native-tls`; fix by always passing `--target` explicitly

- ✅ No change to the TLS stack; identical server behaviour
- ❌ Every developer, every CI job, and the Dockerfile must forever remember
  `--target`.  A single omission restores the original failure with no
  obvious error message pointing to the cause.
- ❌ Docker images continue to install `libssl-dev` (build) and `libssl3`
  (runtime), adding ~5 MB to the image and a C library to the attack surface.

### Keep `native-tls`; remove the global `[build] target` from config

- ✅ `cargo build` without flags would use the host target, making
  `--target` optional for server builds
- ❌ Breaks the primary use-case: `cargo build` for ESP32 firmware would
  require an explicit `--target xtensa-esp32-none-elf` on every invocation,
  which is the opposite of the project's ergonomic goal
- ❌ All existing firmware aliases and documentation would need updating

### Keep `native-tls`; add a separate workspace member for the server

- ✅ Isolates server and firmware Cargo configurations completely
- ❌ Significant workspace restructuring (Cargo.toml split, path changes,
  CI changes) for what is ultimately a one-line dependency change
- ❌ Ongoing maintenance cost of two `Cargo.toml` files

### Switch to a different async Postgres driver (e.g. `tokio-postgres` directly)

- ✅ More control over TLS configuration
- ❌ Loses sqlx compile-time query checking and migration support
- ❌ Larger code change; higher risk of regressions

---

## Consequences

**Positive**

- `openssl-sys` is fully removed from the dependency tree.  The error
  described in the Context section cannot recur regardless of what `--target`
  is active.
- Docker build image is simpler and ~5 MB smaller; runtime image has no
  dynamically-linked C TLS library.
- `rustls` enforces TLS 1.2+ and modern cipher suites by default, with no
  dependence on the host system's OpenSSL configuration.
- Cross-platform builds (Linux CI, macOS dev, Docker) all use the same pure
  code path with no platform-specific native library to manage.

**Negative / trade-offs**

- `rustls` does not use the OS certificate store by default.  The
  `telemetry-server` connects to PostgreSQL on a private LAN (`DATABASE_URL`
  in `docker-compose.yml`); TLS to Postgres is typically disabled in this
  configuration.  If TLS to Postgres is ever enabled in a deployment that
  uses a custom CA, the CA certificate must be supplied explicitly via
  `DATABASE_URL` query parameters or `PGSSLROOTCERT`, rather than being
  picked up from the OS trust store automatically.
- `native-tls` would use FIPS-validated system OpenSSL on hardened hosts.
  This project has no FIPS requirement, so this trade-off is accepted.

**No functional change for the current deployment**

The `docker-compose.yml` connects to a PostgreSQL container on an internal
Docker network without TLS (`postgres://robots:robots@postgres:5432/robots`).
Both `native-tls` and `rustls` behave identically in this configuration;
the switch is transparent to the running application.
