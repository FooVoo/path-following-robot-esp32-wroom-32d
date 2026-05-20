//! ESP32-C3 path-following robot — production firmware composition root.
//!
//! This file mirrors `main.rs` (ESP32 / Xtensa) but targets the
//! **ESP32-C3-MINI-1** (RISC-V, `riscv32imc-unknown-none-elf`).
//!
//! Key differences from the ESP32 binary:
//!
//! * **No display** — the ESP32-C3-MINI-1 module only exposes GPIO0–GPIO10 and
//!   GPIO18–GPIO21 for user I/O.  GPIO11–GPIO17 are wired to internal SPI flash.
//!   Fitting a 6-pin HD44780 would require sacrificing UART0 debug output.
//!   The SSD1306 OLED is available in the dev binary (`main_c3_dev.rs`);
//!   the production firmware uses [`NoDisplay`] to keep binary size minimal.
//! * **All adapters re-used unchanged** — esp-hal exposes the same LEDC, ADC,
//!   I2C, and WiFi APIs on both chips.
//! * **`Robot::new_with_wifi`** is used instead of `Robot::new_full` because
//!   there is no display in the production build.
//!
//! # Building
//!
//! ```sh
//! cargo +esp build --features esp32c3-firmware --bin path-following-robot-c3 \
//!       --target riscv32imc-unknown-none-elf
//! # or via alias:
//! cargo build-firmware-c3
//! ```
//!
//! # Flashing
//!
//! ```sh
//! cargo +esp run --features esp32c3-firmware --bin path-following-robot-c3 \
//!       --target riscv32imc-unknown-none-elf
//! # or directly:
//! espflash flash --monitor --chip esp32c3 \
//!       target/riscv32imc-unknown-none-elf/debug/path-following-robot-c3
//! ```
//!
//! # Pin assignment (ESP32-C3-MINI-1)
//!
//! | Signal       | GPIO | Notes                                                       |
//! |--------------|------|-------------------------------------------------------------|
//! | AIN1         |   3  | Left motor forward  (LEDC CH0)                             |
//! | AIN2         |   4  | Left motor reverse  (LEDC CH1)                             |
//! | BIN1         |   5  | Right motor forward (LEDC CH2)                             |
//! | BIN2         |   6  | Right motor reverse (LEDC CH3)                             |
//! | I2C SDA      |   7  | Shared by TCA9548A + VL53L0X                                |
//! | I2C SCL      |   8  | ⚠ strapping pin — I2C pull-up holds it high at boot        |
//! | STEPPER IN1  |   2  | ⚠ strapping pin (JTAG) — fit 10 kΩ pull-up to 3.3 V       |
//! | STEPPER IN2  |   9  | ⚠ strapping pin (BOOT) — internal pull-up, safe at boot    |
//! | STEPPER IN3  |  18  |                                                             |
//! | STEPPER IN4  |  19  |                                                             |
//! | JOY X        |   0  | ADC1_CH0                                                    |
//! | JOY Y        |   1  | ADC1_CH1                                                    |
//! | JOY BTN      |  10  | Active-low, internal pull-up                                |
//! | UART0 TX     |  20  | esp-println debug output — do NOT reassign                  |
//! | UART0 RX     |  21  | UART0 RX — do NOT reassign                                  |
//!
//! # WiFi
//!
//! Set `WIFI_SSID` and `WIFI_PASSWORD` in `src/config.rs` before flashing.
//! The robot broadcasts JSON telemetry to UDP port 9001 and accepts 4-byte
//! command packets on UDP port 9000.

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp_hal types."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use core::panic::PanicInfo;

use core::cell::RefCell;

use esp_hal::{
    Config,
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{DriveMode, Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    main,
    rng::Rng,
    time::{Instant, Rate},
    timer::timg::TimerGroup,
};
use log::info;

use path_following_robot::{
    adapters::esp32::{
        drv8833::Drv8833, joystick::Esp32Joystick, tca9548a::Tca9548a,
        uln2003::Uln2003, vl53l0x::Vl53l0xOnMux, wifi::WifiAdapter,
    },
    config::{
        I2C_FREQ_HZ, LOOP_MS, PWM_FREQ_HZ, STEPPER_IN1_GPIO, STEPPER_IN2_GPIO,
        STEPPER_IN3_GPIO, STEPPER_IN4_GPIO, TCA9548A_ADDR,
        VL53L0X_LEFT_CHANNEL, VL53L0X_RIGHT_CHANNEL, WIFI_HEAP_SIZE,
    },
    domain::robot::Robot,
};

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    esp_println::println!("PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

// ── Application descriptor (required by the esp-idf bootloader) ───────────────

esp_bootloader_esp_idf::esp_app_desc!();

// ── Entry point ───────────────────────────────────────────────────────────────

#[allow(
    clippy::large_stack_frames,
    reason = "PathBuffer (2 KiB) is acceptable in main(); no recursion here."
)]
#[main]
fn main() -> ! {
    // ── Platform init ──────────────────────────────────────────────────────
    let config = Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max());
    let peripherals = esp_hal::init(config);

    // ── Heap for WiFi stack + smoltcp socket buffers ───────────────────────
    esp_alloc::heap_allocator!(size: WIFI_HEAP_SIZE);

    #[cfg(debug_assertions)]
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    #[cfg(not(debug_assertions))]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    info!("=== path-following-robot-c3 booting (ESP32-C3) ===");

    // ── LEDC / Motor PWM ────────────────────────────────────────────────────
    //
    // ESP32-C3 has 6 LEDC low-speed channels; we use 4 (CH0–CH3) for the
    // DRV8833 inputs.  The API is identical to the ESP32.

    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);

    let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_hz(PWM_FREQ_HZ),
        })
        .expect("LEDC timer0 configure failed");

    let mut ch_ain1 = ledc.channel::<LowSpeed>(channel::Number::Channel0, peripherals.GPIO3);
    ch_ain1
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_ain1 configure failed");

    let mut ch_ain2 = ledc.channel::<LowSpeed>(channel::Number::Channel1, peripherals.GPIO4);
    ch_ain2
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_ain2 configure failed");

    let mut ch_bin1 = ledc.channel::<LowSpeed>(channel::Number::Channel2, peripherals.GPIO5);
    ch_bin1
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_bin1 configure failed");

    let mut ch_bin2 = ledc.channel::<LowSpeed>(channel::Number::Channel3, peripherals.GPIO6);
    ch_bin2
        .configure(channel::config::Config {
            timer: &timer0,
            duty_pct: 0,
            drive_mode: DriveMode::PushPull,
        })
        .expect("LEDC ch_bin2 configure failed");

    info!("LEDC: 4 × 8-bit channels @ {} Hz (GPIO3,4,5,6)", PWM_FREQ_HZ);

    // ── Motor adapter ────────────────────────────────────────────────────────
    let motors = Drv8833::new(ch_ain1, ch_ain2, ch_bin1, ch_bin2);

    // ── I2C bus (shared by TCA9548A mux + both VL53L0X sensors) ───────────────
    //
    // GPIO8 is a strapping pin but the I2C external pull-up holds it high at
    // boot, satisfying the "ROM log disabled" default.
    let i2c_cell = RefCell::new(
        I2c::new(
            peripherals.I2C0,
            I2cConfig::default().with_frequency(Rate::from_hz(I2C_FREQ_HZ)),
        )
        .expect("I2C init failed")
        .with_sda(peripherals.GPIO7)
        .with_scl(peripherals.GPIO8),
    );
    info!("I2C: SDA=GPIO7  SCL=GPIO8  freq={}Hz", I2C_FREQ_HZ);

    // ── TCA9548A multiplexer ─────────────────────────────────────────────────
    let mux = Tca9548a::new(&i2c_cell, TCA9548A_ADDR);
    info!("TCA9548A: addr=0x{:02X}", TCA9548A_ADDR);

    // ── VL53L0X ToF LIDARs (via mux channels 0 and 1) ───────────────────────
    let _ = mux;
    let lidar_l = Vl53l0xOnMux::init(&i2c_cell, TCA9548A_ADDR, VL53L0X_LEFT_CHANNEL);
    let lidar_r = Vl53l0xOnMux::init(&i2c_cell, TCA9548A_ADDR, VL53L0X_RIGHT_CHANNEL);
    info!(
        "VL53L0X: left=ch{}  right=ch{}",
        VL53L0X_LEFT_CHANNEL, VL53L0X_RIGHT_CHANNEL
    );

    // ── ADC — Joystick axes ─────────────────────────────────────────────────
    //
    // ESP32-C3 ADC1 channels: CH0=GPIO0, CH1=GPIO1.
    // No internal pull resistors on ADC pins — leave floating for joystick.
    // 11 dB attenuation → 0–3.3 V full-scale.
    let mut adc_config = AdcConfig::new();
    let joy_x_pin = adc_config.enable_pin(peripherals.GPIO0, Attenuation::_11dB);
    let joy_y_pin = adc_config.enable_pin(peripherals.GPIO1, Attenuation::_11dB);
    let adc1 = Adc::new(peripherals.ADC1, adc_config);
    info!("ADC: joystick X=GPIO0 (ADC1_CH0)  Y=GPIO1 (ADC1_CH1)");

    // ── Button ───────────────────────────────────────────────────────────────
    // GPIO10 has no special function and supports the internal pull-up.
    let btn = Input::new(
        peripherals.GPIO10,
        InputConfig::default().with_pull(Pull::Up),
    );
    info!("Button: GPIO10 (active-low, internal pull-up)");

    // ── Joystick adapter ─────────────────────────────────────────────────────
    let joystick = Esp32Joystick::new(adc1, joy_x_pin, joy_y_pin, btn);

    // ── WiFi adapter ─────────────────────────────────────────────────────────
    //
    // esp-rtos and esp-radio use the same API on ESP32-C3 as on ESP32.
    // TIMG1 is used so TIMG0 (LEDC above) is not double-used.
    let timg1 = TimerGroup::new(peripherals.TIMG1);
    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timg1.timer0, sw_int.software_interrupt0);

    let rng = Rng::new();
    let _ = rng;
    let wifi = WifiAdapter::connect(peripherals.WIFI);

    // ── ULN2003 stepper driver ────────────────────────────────────────────────
    //
    // GPIO2 (IN1) is a strapping pin for JTAG mode selection.  The ULN2003
    // input presents high-impedance during chip reset, so an external 10 kΩ
    // pull-up resistor from GPIO2 to 3.3 V is REQUIRED to keep the chip in
    // normal (non-JTAG) mode.  Once the LEDC driver starts, it overrides the
    // resistor.
    //
    // GPIO9 (IN2) is the BOOT strapping pin; its internal pull-up keeps it
    // high under normal conditions — no external resistor needed here.
    let _stepper = Uln2003::new(
        Output::new(peripherals.GPIO2, Level::Low, OutputConfig::default()), // IN1
        Output::new(peripherals.GPIO9, Level::Low, OutputConfig::default()), // IN2
        Output::new(peripherals.GPIO18, Level::Low, OutputConfig::default()), // IN3
        Output::new(peripherals.GPIO19, Level::Low, OutputConfig::default()), // IN4
    );
    info!(
        "ULN2003: IN1-IN4={},{},{},{}",
        STEPPER_IN1_GPIO, STEPPER_IN2_GPIO, STEPPER_IN3_GPIO, STEPPER_IN4_GPIO
    );

    // ── Robot aggregate ──────────────────────────────────────────────────────
    //
    // `new_with_wifi` uses `NoDisplay` as the default `D` type parameter.
    let mut robot = Robot::new_with_wifi(motors, lidar_l, lidar_r, joystick, wifi);

    info!("Robot ready (C3) — entering main loop at ~100 Hz");

    // ── Boot reference time ──────────────────────────────────────────────────
    let boot = Instant::now();
    let delay = Delay::new();

    // ── Cooperative 100 Hz main loop ─────────────────────────────────────────
    loop {
        let now_ms = boot.elapsed().as_millis();
        robot.tick(now_ms);
        delay.delay_millis(LOOP_MS as u32);
    }
}
