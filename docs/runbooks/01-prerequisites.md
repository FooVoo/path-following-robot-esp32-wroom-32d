# Runbook 01 — Prerequisites

> **Audience:** First-time setup on a development machine.

---

## 1  Host operating system

The toolchain is tested on macOS and Linux.  Windows (WSL2) also works but
USB device pass-through requires an extra udev step.

---

## 2  Rust toolchains

### 2.1  Stable Rust (for host unit tests)

```bash
# Install rustup if not present
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Verify
rustup toolchain install stable
cargo +stable --version          # must be ≥ 1.88.0
```

### 2.2  ESP Rust toolchain (for cross-compilation)

The `esp` toolchain is Espressif's fork of the Rust compiler with Xtensa
LLVM support.  It is managed by `espup`.

```bash
cargo install espup
espup install          # downloads the `esp` toolchain + Xtensa LLVM backend
```

After installation, add the toolchain activation to your shell profile:

```bash
# bash / zsh
echo '. ~/export-esp.sh' >> ~/.bashrc    # or ~/.zshrc
source ~/export-esp.sh
```

Verify:

```bash
rustup toolchain list | grep esp
cargo +esp --version
```

The project's `rust-toolchain.toml` sets `channel = "esp"`, so any `cargo`
command in the project directory automatically uses the esp toolchain.

---

## 3  Flash tool

```bash
cargo install espflash
espflash --version      # must be ≥ 3.0
```

---

## 4  USB-UART driver

The ESP32-WROOM-32D module uses a **CP2102** or **CH340** USB-UART bridge.

| OS | Action |
|---|---|
| macOS | Drivers included in macOS ≥12. For CH340: `brew install --cask wch-ch34x-usb-serial-driver` |
| Ubuntu / Debian | `sudo apt install brltty`; then add your user to the `dialout` group: `sudo usermod -aG dialout $USER` |
| Windows (WSL2) | Install [CP2102 driver](https://www.silabs.com/developer-tools/usb-to-uart-bridge-vcp-drivers); use `usbipd` to attach the COM port to WSL2 |

Verify the device appears:

```bash
# macOS
ls /dev/cu.usbserial*

# Linux
ls /dev/ttyUSB* /dev/ttyACM*
```

---

## 5  Python (optional — for monitor scripts)

The telemetry monitor script (`docs/runbooks/03-wifi-setup.md`) requires
Python ≥3.8 with no extra packages.  Standard library only.

---

## 6  Quick verification

```bash
cd path-following-robot-esp32-wroom-32d

# Host unit tests — no ESP32 required
cargo +stable test --lib --target aarch64-apple-darwin

# Expected output:
# test result: ok. 19 passed; 0 failed; 0 ignored
```

If the tests pass, the development environment is ready.
