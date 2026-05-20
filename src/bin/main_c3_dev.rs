//! ESP32-C3 path-following robot — dev/debug firmware composition root.
//!
//! This binary is intended for **bench testing with a single VL53L0X sensor**
//! (no TCA9548A multiplexer required).  It mirrors `main_dev.rs` (ESP32) but
//! targets the **ESP32-C3-MINI-1** (RISC-V, `riscv32imc-unknown-none-elf`).
//!
//! Differences from the C3 production binary (`main_c3.rs`):
//!
//! * **One real VL53L0X** on I2C at `0x29` (direct, no mux).
//! * **WiFi replaced by [`NoWifi`]** — no network stack, no RTOS.
//! * **SSD1306 OLED** — 4-pin I²C display sharing the same bus as the sensor.
//!
//! # Building
//!
//! ```sh
//! cargo +esp build --features esp32c3-dev --bin path-following-robot-c3-dev \
//!       --target riscv32imc-unknown-none-elf
//! # or via alias:
//! cargo build-dev-c3
//! ```
//!
//! # Flashing
//!
//! ```sh
//! cargo +esp run --features esp32c3-dev --bin path-following-robot-c3-dev \
//!       --target riscv32imc-unknown-none-elf
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
//! | I2C SDA      |   7  | VL53L0X + SSD1306 SDA (direct, no mux)                     |
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

#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is not safe with esp_hal types."
)]
#![deny(clippy::large_stack_frames)]

extern crate alloc;

use core::{cell::RefCell, panic::PanicInfo};

use esp_hal::{
    Config,
    analog::adc::{Adc, AdcConfig, Attenuation},
    delay::Delay,
    gpio::{DriveMode, Input, InputConfig, Level, Output, OutputConfig, Pull},
    i2c::master::{Config as I2cConfig, I2c},
    ledc::{
        LSGlobalClkSource, Ledc, LowSpeed,
        channel::{self, ChannelIFace},
        timer::{self, TimerIFace},
    },
    main,
    time::{Instant, Rate},
};
use log::info;

use path_following_robot::{
    adapters::esp32::{drv8833::Drv8833, joystick::Esp32Joystick, ssd1306_oled::Ssd1306Display, uln2003::Uln2003, vl53l0x_direct::Vl53l0xDirect},
    config::{
        I2C_FREQ_HZ, I2C_SCL_GPIO, I2C_SDA_GPIO, LOOP_MS, PWM_FREQ_HZ, SSD1306_I2C_ADDR,
        STEPPER_IN1_GPIO, STEPPER_IN2_GPIO, STEPPER_IN3_GPIO, STEPPER_IN4_GPIO,
    },
    domain::robot::{NoWifi, Robot},
    ports::distance::DistancePort,
};

// ── Panic handler ─────────────────────────────────────────────────────────────

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    esp_println::println!("PANIC: {}", info);
    loop {
        core::hint::spin_loop();
    }
}

// ── Application descriptor ───────────────────────────────────────────────────

esp_bootloader_esp_idf::esp_app_desc!();

// ── Single-sensor wrapper ────────────────────────────────────────────────────
//
// Mirrors the `DevSensor` pattern from `main_dev.rs`.  `Robot<M, L, I, W, D>`
// requires both lidar slots to be the same type `L`, so we use an enum that
// covers both the real sensor and the always-clear stub.

enum DevSensor<'d> {
    Direct(Vl53l0xDirect<'d>),
    /// Always returns `Some(200)` — stands in for the absent right sensor.
    AlwaysClear,
}

impl<'d> DistancePort for DevSensor<'d> {
    fn poll(&mut self) {
        if let Self::Direct(s) = self {
            s.poll();
        }
    }

    fn distance_cm(&self) -> Option<u16> {
        match self {
            Self::Direct(s) => s.distance_cm(),
            Self::AlwaysClear => Some(200),
        }
    }

    fn tick_staleness(&mut self) {
        if let Self::Direct(s) = self {
            s.tick_staleness();
        }
    }
}

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

    // Small heap for alloc types in prost-generated proto code.
    esp_alloc::heap_allocator!(size: 8192);

    #[cfg(debug_assertions)]
    esp_println::logger::init_logger(log::LevelFilter::Debug);
    #[cfg(not(debug_assertions))]
    esp_println::logger::init_logger(log::LevelFilter::Info);

    info!("=== path-following-robot-c3-dev booting (single VL53L0X, no WiFi, no LCD) ===");

    // ── LEDC / Motor PWM ────────────────────────────────────────────────────
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

    // ── I2C — single VL53L0X (no mux) ────────────────────────────────────────
    //
    // GPIO8 is a strapping pin but the I2C external pull-up holds it high.
    let i2c_cell = RefCell::new(
        I2c::new(
            peripherals.I2C0,
            I2cConfig::default().with_frequency(Rate::from_hz(I2C_FREQ_HZ)),
        )
        .expect("I2C init failed")
        .with_sda(peripherals.GPIO7)
        .with_scl(peripherals.GPIO8),
    );

    let lidar_l = DevSensor::Direct(Vl53l0xDirect::init(&i2c_cell));
    // Right slot is always-clear: one physical sensor on the dev bench.
    let lidar_r = DevSensor::AlwaysClear;

    info!(
        "LIDAR: single VL53L0X direct I2C SDA=GPIO{}  SCL=GPIO{}  @ {}kHz",
        I2C_SDA_GPIO,
        I2C_SCL_GPIO,
        I2C_FREQ_HZ / 1_000
    );

    // ── SSD1306 OLED display ──────────────────────────────────────────────────
    //
    // Shares the I2C bus with the VL53L0X (0x29 vs 0x3C — no conflict).
    let display = Ssd1306Display::init(&i2c_cell);
    info!("SSD1306: OLED 128×64 I2C addr=0x{:02X}", SSD1306_I2C_ADDR);

    // ── ADC — Joystick axes ─────────────────────────────────────────────────
    let mut adc_config = AdcConfig::new();
    let joy_x_pin = adc_config.enable_pin(peripherals.GPIO0, Attenuation::_11dB);
    let joy_y_pin = adc_config.enable_pin(peripherals.GPIO1, Attenuation::_11dB);
    let adc1 = Adc::new(peripherals.ADC1, adc_config);

    // ── Button ───────────────────────────────────────────────────────────────
    let btn = Input::new(
        peripherals.GPIO10,
        InputConfig::default().with_pull(Pull::Up),
    );

    // ── Joystick adapter ─────────────────────────────────────────────────────
    let joystick = Esp32Joystick::new(adc1, joy_x_pin, joy_y_pin, btn);
    info!("Joystick: X=GPIO0 (ADC1_CH0)  Y=GPIO1 (ADC1_CH1)  BTN=GPIO10");

    // ── ULN2003 stepper driver ────────────────────────────────────────────────
    //
    // See main_c3.rs for strapping-pin notes on GPIO2 and GPIO9.
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

    // ── Robot aggregate ───────────────────────────────────────────────────────
    let mut robot = Robot::new_full(motors, lidar_l, lidar_r, joystick, NoWifi, display);

    info!("Robot ready (C3 dev) — single sensor, no WiFi, OLED display");

    // ── Boot reference time ──────────────────────────────────────────────────
    let boot = Instant::now();
    let delay = Delay::new();

    // ── 100 Hz main loop ─────────────────────────────────────────────────────
    loop {
        let now_ms = boot.elapsed().as_millis();
        robot.tick(now_ms);
        delay.delay_millis(LOOP_MS as u32);
    }
}
